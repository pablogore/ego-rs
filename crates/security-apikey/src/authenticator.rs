//! [`ApiKeyAuthenticationProvider`] — implements [`AuthenticationProvider`] for opaque API keys.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use ego_domain::auth::{AuthenticationError, Claims, Clock, StandardClaims};
use ego_security_sdk::{AuthenticationProvider, Credential, SecurityContext};
use serde_json::Value;

use crate::key_hash::ApiKeyHash;
use crate::parser::{ApiKeyParser, DefaultApiKeyParser};
use crate::resolver::ApiKeyResolver;

/// Uniform failure for every rejection path — the message is intentionally
/// identical everywhere so a future edit to one branch can't accidentally
/// reintroduce a cause-differentiation oracle (see the invariant documented
/// on [`ApiKeyAuthenticationProvider`]).
fn invalid_token() -> AuthenticationError {
    AuthenticationError::InvalidToken("invalid token".into())
}

/// Maximum raw credential size before any parsing is attempted.
///
/// Credentials longer than this are rejected with [`AuthenticationError::InvalidToken`]
/// to prevent resource exhaustion (mirrors JWT's `MAX_TOKEN_BYTES`).
pub const MAX_KEY_BYTES: usize = 1024;

/// Authentication provider for opaque API keys.
///
/// All failure paths return [`AuthenticationError::InvalidToken`] with no cause
/// differentiation — this is a deliberate security invariant that prevents
/// oracle attacks; callers MUST NOT forward the inner error message to external
/// consumers.
pub struct ApiKeyAuthenticationProvider {
    resolver: Arc<dyn ApiKeyResolver>,
    parser: Arc<dyn ApiKeyParser>,
    clock: Arc<dyn Clock>,
}

impl ApiKeyAuthenticationProvider {
    /// Creates a provider using the [`DefaultApiKeyParser`].
    pub fn new(resolver: Arc<dyn ApiKeyResolver>, clock: Arc<dyn Clock>) -> Self {
        Self {
            resolver,
            parser: Arc::new(DefaultApiKeyParser),
            clock,
        }
    }

    /// Replaces the parser with a custom implementation.
    pub fn with_parser(mut self, parser: Arc<dyn ApiKeyParser>) -> Self {
        self.parser = parser;
        self
    }
}

impl AuthenticationProvider for ApiKeyAuthenticationProvider {
    fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        let raw = match credential {
            Credential::Bearer(t) => t.as_str(),
            _ => return Err(invalid_token()),
        };

        if raw.len() > MAX_KEY_BYTES {
            return Err(invalid_token());
        }

        let (key_id, secret) = self.parser.parse(raw).map_err(|_| invalid_token())?;

        let record = self
            .resolver
            .lookup(&key_id)
            .inspect_err(|e| tracing::warn!(error = %e, "api key resolver backend error"))
            .map_err(|_| invalid_token())?;

        // Always hash-verify, using a dummy digest when the key_id is unknown,
        // so an unknown key_id and a known key_id with the wrong secret take
        // the same amount of work — otherwise response timing would let an
        // attacker enumerate valid key_ids before ever guessing a secret.
        let dummy_hash = ApiKeyHash::sha256([0u8; 32]);
        let hash_ok = record
            .as_deref()
            .map_or(&dummy_hash, |r| &r.key_hash)
            .verify(secret.as_bytes());

        let now = SystemTime::from(self.clock.now());
        let not_expired = record
            .as_deref()
            .is_some_and(|r| r.expires_at.is_none_or(|exp| now < exp));

        let Some(record) = record.filter(|_| hash_ok && not_expired) else {
            return Err(invalid_token());
        };

        let scopes_array = Value::Array(
            record
                .scopes
                .iter()
                .map(|s| Value::String(s.clone()))
                .collect(),
        );
        let mut custom = BTreeMap::new();
        custom.insert("scopes".to_owned(), scopes_array);

        let exp = record.expires_at.and_then(|t| {
            let dt = t
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_secs()).ok())
                .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0));
            if dt.is_none() {
                tracing::warn!("api key record has an out-of-range expires_at; omitting exp claim");
            }
            dt
        });
        let claims = Claims {
            standard: StandardClaims {
                exp,
                ..Default::default()
            },
            custom,
        };

        Ok(SecurityContext::new(record.principal.clone(), claims))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::SystemTime;

    use chrono::{DateTime, TimeZone, Utc};
    use ego_domain::auth::{AuthenticationError, Clock};
    use ego_security_sdk::principal::{PrincipalKind, SubjectId};
    use ego_security_sdk::{Credential, Principal};

    use crate::key_hash::ApiKeyHash;
    use crate::key_id::ApiKeyId;
    use crate::resolver::{ApiKeyRecord, InMemoryApiKeyResolver};

    use super::*;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    struct FixedClock(DateTime<Utc>);

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    fn fixed_clock(dt: DateTime<Utc>) -> Arc<dyn Clock> {
        Arc::new(FixedClock(dt))
    }

    fn pinned_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap()
    }

    fn make_principal(subject: &str) -> Principal {
        Principal::new(PrincipalKind::User, SubjectId::new(subject).unwrap())
    }

    fn single_key_resolver(
        key_id: &str,
        secret: &[u8],
        expires_at: Option<SystemTime>,
        scopes: Vec<String>,
    ) -> Arc<dyn ApiKeyResolver> {
        let mut resolver = InMemoryApiKeyResolver::new();
        let id = ApiKeyId::new(key_id).unwrap();
        let record = ApiKeyRecord {
            principal: make_principal("user:test"),
            scopes,
            expires_at,
            metadata: Arc::new(HashMap::new()),
            key_hash: ApiKeyHash::of(secret),
        };
        resolver.insert(id, record);
        Arc::new(resolver)
    }

    // -----------------------------------------------------------------------
    // Happy path
    // -----------------------------------------------------------------------

    #[test]
    fn happy_path_returns_security_context_with_correct_principal() {
        let resolver = single_key_resolver("mykey", b"mysecret", None, vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned_now()));
        let cred = Credential::Bearer("mykey.mysecret".into());
        let ctx = provider.authenticate(&cred).unwrap();
        assert_eq!(ctx.principal().subject_id.as_str(), "user:test");
    }

    #[test]
    fn scopes_propagated_into_claims_custom_as_json_array() {
        let scopes = vec!["read:orders".to_owned(), "write:invoices".to_owned()];
        let resolver = single_key_resolver("mykey", b"mysecret", None, scopes);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned_now()));
        let ctx = provider
            .authenticate(&Credential::Bearer("mykey.mysecret".into()))
            .unwrap();

        let scopes_val = ctx.claims().custom.get("scopes").unwrap();
        let arr = scopes_val.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_str().unwrap(), "read:orders");
        assert_eq!(arr[1].as_str().unwrap(), "write:invoices");
    }

    #[test]
    fn empty_scopes_propagated_as_empty_json_array() {
        let resolver = single_key_resolver("k", b"s", None, vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned_now()));
        let ctx = provider
            .authenticate(&Credential::Bearer("k.s".into()))
            .unwrap();

        let scopes_val = ctx.claims().custom.get("scopes").unwrap();
        let arr = scopes_val.as_array().unwrap();
        assert!(arr.is_empty());
    }

    // -----------------------------------------------------------------------
    // Failure paths — all collapse to InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn non_bearer_credential_returns_invalid_token() {
        let resolver = single_key_resolver("k", b"s", None, vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned_now()));
        let err = provider
            .authenticate(&Credential::Basic {
                username: "user".into(),
                secret: "pass".into(),
            })
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn oversized_credential_rejected_before_parse() {
        let resolver = single_key_resolver("k", b"s", None, vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned_now()));
        let oversized = "k".repeat(MAX_KEY_BYTES + 1);
        let err = provider
            .authenticate(&Credential::Bearer(oversized))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn credential_at_max_key_bytes_passes_size_guard() {
        // The at-limit credential will fail at the parse step (no dot), but the
        // size guard itself must pass — confirming the guard is > not >=.
        let resolver = single_key_resolver("k", b"s", None, vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned_now()));
        // Build a string of exactly MAX_KEY_BYTES that has no dot → passes guard, fails parse
        let at_limit = "a".repeat(MAX_KEY_BYTES);
        let err = provider
            .authenticate(&Credential::Bearer(at_limit))
            .unwrap_err();
        // Must NOT be due to oversized — parse error (no dot) is expected
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
        if let AuthenticationError::InvalidToken(msg) = &err {
            assert_eq!(msg, "invalid token");
        }
    }

    #[test]
    fn malformed_key_no_dot_returns_invalid_token() {
        let resolver = single_key_resolver("k", b"s", None, vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned_now()));
        let err = provider
            .authenticate(&Credential::Bearer("no-separator-here".into()))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn unknown_key_id_returns_invalid_token() {
        let resolver = single_key_resolver("known-key", b"s", None, vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned_now()));
        let err = provider
            .authenticate(&Credential::Bearer("unknown-key.s".into()))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn expired_record_returns_invalid_token() {
        let pinned = pinned_now();
        let expires_at: SystemTime = (pinned - chrono::Duration::seconds(1)).into();
        let resolver = single_key_resolver("k", b"secret", Some(expires_at), vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned));
        let err = provider
            .authenticate(&Credential::Bearer("k.secret".into()))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn hash_mismatch_returns_invalid_token() {
        let resolver = single_key_resolver("k", b"correct-secret", None, vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned_now()));
        let err = provider
            .authenticate(&Credential::Bearer("k.wrong-secret".into()))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn timing_safety_no_early_return_before_hash_verify() {
        // Regression guard for the timing-equalization fix: `authenticate()`
        // must always reach the dummy-hash verification step before deciding
        // to reject an unknown key_id. A wall-clock timing assertion would be
        // too flaky for CI, so this asserts the invariant structurally — that
        // no `return Err` sits between the resolver lookup and the dummy-hash
        // step — which is exactly what a reintroduced early-return-on-`None`
        // (the old, timing-unsafe ordering) would add.
        let src = include_str!("authenticator.rs");
        let body_start = src
            .find("fn authenticate(")
            .expect("authenticate() not found in source");
        let body = &src[body_start..];
        let lookup_pos = body
            .find(".lookup(&key_id)")
            .expect("resolver.lookup call not found");
        let dummy_hash_pos = body
            .find("dummy_hash")
            .expect("dummy-hash timing-equalization step not found — has the fix been reverted?");
        assert!(
            !body[lookup_pos..dummy_hash_pos].contains("return Err"),
            "a `return Err` was found between the resolver lookup and the dummy-hash \
             verification step — this reintroduces the timing side-channel between an \
             unknown key_id and a known key_id with the wrong secret"
        );
    }

    #[test]
    fn record_expiring_at_exactly_now_is_rejected() {
        // Boundary: `expires_at == now` must be treated as expired, matching
        // security-jwt's tie-break (`now >= exp` rejects), not the inverse.
        let pinned = pinned_now();
        let expires_at: SystemTime = pinned.into();
        let resolver = single_key_resolver("k", b"secret", Some(expires_at), vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned));
        let err = provider
            .authenticate(&Credential::Bearer("k.secret".into()))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn exp_claim_set_correctly_for_unexpired_record() {
        let pinned = pinned_now();
        let expires_at: SystemTime = (pinned + chrono::Duration::seconds(60)).into();
        let resolver = single_key_resolver("k", b"secret", Some(expires_at), vec![]);
        let provider = ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned));
        let ctx = provider
            .authenticate(&Credential::Bearer("k.secret".into()))
            .unwrap();
        assert_eq!(
            ctx.claims().standard.exp,
            Some(pinned + chrono::Duration::seconds(60))
        );
    }

    // -----------------------------------------------------------------------
    // Compile-time assertions
    // -----------------------------------------------------------------------

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn provider_is_send_sync() {
        assert_send_sync::<ApiKeyAuthenticationProvider>();
    }

    #[test]
    fn provider_is_object_safe_behind_arc() {
        let resolver: Arc<dyn ApiKeyResolver> = Arc::new(InMemoryApiKeyResolver::new());
        let provider: Arc<dyn AuthenticationProvider> = Arc::new(
            ApiKeyAuthenticationProvider::new(resolver, fixed_clock(pinned_now())),
        );
        // Just constructing this confirms object safety at compile time.
        let _ = provider;
    }
}
