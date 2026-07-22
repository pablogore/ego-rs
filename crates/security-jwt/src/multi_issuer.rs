//! Multi-issuer routing — `IssuerResolver`, `StaticIssuerResolver`,
//! `MultiIssuerAuthenticationProvider`.
//!
//! Extracts the `iss` claim from the **unverified** JWT payload (base64url-decode,
//! no signature check) for routing only. The routed sub-provider re-validates
//! `iss` post-signature (INV-4).

use std::collections::HashMap;
use std::sync::Arc;

use base64::{engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD}, Engine as _};
use ego_domain::auth::AuthenticationError;
use ego_security_sdk::{AuthenticationProvider, Credential, SecurityContext};

// ---------------------------------------------------------------------------
// unverified_iss — pub(crate) helper
// ---------------------------------------------------------------------------

/// Extracts the `iss` claim from an **unverified** JWT payload.
///
/// Safety: the value is used for routing only. The sub-provider that receives
/// the credential re-validates `iss` as part of full signature verification (INV-4).
pub(crate) fn unverified_iss(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    // Require exactly 3 parts (header.payload.signature). JWE has 5 parts;
    // parts[1] there is the encrypted key, not a readable payload — reject it.
    if parts.len() != 3 {
        return None;
    }
    // Payload is parts[1] — base64url (URL_SAFE_NO_PAD, RFC 4648 §5).
    // Some IdPs emit padded base64url (with '='). Try unpadded first; fall back to padded.
    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1])
        .or_else(|_| URL_SAFE.decode(parts[1]))
        .ok()?;
    let payload: serde_json::Value = serde_json::from_slice(&payload_bytes).ok()?;
    payload.get("iss").and_then(|v| v.as_str()).map(str::to_owned)
}

// ---------------------------------------------------------------------------
// IssuerResolver trait
// ---------------------------------------------------------------------------

/// Routes an issuer string to its `AuthenticationProvider`.
///
/// Called with the **unverified** `iss` from the JWT payload — the routed
/// sub-provider is responsible for re-validating `iss` post-signature (INV-4).
pub trait IssuerResolver: Send + Sync {
    /// Returns the provider registered for `issuer`, or `None` if unknown.
    fn resolve(&self, issuer: &str) -> Option<Arc<dyn AuthenticationProvider>>;
}

// ---------------------------------------------------------------------------
// StaticIssuerResolver
// ---------------------------------------------------------------------------

/// Static `HashMap`-backed `IssuerResolver`.
///
/// Suitable for deployments with a fixed, known set of issuers.
pub struct StaticIssuerResolver {
    providers: HashMap<String, Arc<dyn AuthenticationProvider>>,
}

impl StaticIssuerResolver {
    /// Construct from a pre-built map of `issuer → provider`.
    pub fn new(providers: HashMap<String, Arc<dyn AuthenticationProvider>>) -> Self {
        Self { providers }
    }
}

impl IssuerResolver for StaticIssuerResolver {
    fn resolve(&self, issuer: &str) -> Option<Arc<dyn AuthenticationProvider>> {
        self.providers.get(issuer).cloned()
    }
}

// ---------------------------------------------------------------------------
// MultiIssuerAuthenticationProvider
// ---------------------------------------------------------------------------

/// Routes JWT credentials to per-issuer `AuthenticationProvider` instances.
///
/// Implements `AuthenticationProvider` so it can be used anywhere a single
/// provider is expected (AD-OIDC-012).
pub struct MultiIssuerAuthenticationProvider {
    resolver: Arc<dyn IssuerResolver>,
}

impl MultiIssuerAuthenticationProvider {
    /// Construct with the given `IssuerResolver`.
    pub fn new(resolver: Arc<dyn IssuerResolver>) -> Self {
        Self { resolver }
    }
}

impl AuthenticationProvider for MultiIssuerAuthenticationProvider {
    fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        let token = match credential {
            Credential::Bearer(t) => t.as_str(),
            _ => {
                return Err(AuthenticationError::InvalidToken(
                    "unsupported credential type".into(),
                ))
            }
        };

        // Pre-check: reject oversized tokens before any base64 work (INV per spec)
        if token.len() > crate::authenticator::MAX_TOKEN_BYTES {
            return Err(AuthenticationError::InvalidToken("token exceeds 8 KiB limit".into()));
        }

        // Extract iss without verifying signature (routing only — INV-4)
        let iss = unverified_iss(token).ok_or_else(|| {
            AuthenticationError::InvalidToken("missing or unreadable iss claim".into())
        })?;

        let provider = self.resolver.resolve(&iss).ok_or_else(|| {
            // Truncate to avoid log injection from attacker-controlled iss values.
            let safe_iss: String = iss.chars().take(128).collect();
            AuthenticationError::InvalidToken(format!("unknown issuer: {safe_iss}"))
        })?;

        provider.authenticate(credential)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use ego_domain::auth::AuthenticationError;
    use ego_security_sdk::{AuthenticationProvider, Credential};
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde_json::json;

    use crate::config::{JwtAlgorithm, JwtProviderConfig};
    use crate::key_resolver::{LocalKeyResolver, VerificationKey};
    use crate::test_helpers::{fixed_clock, TEST_AUD};
    use crate::{Rs256AuthenticationProvider};

    // -----------------------------------------------------------------------
    // Key fixtures
    // -----------------------------------------------------------------------

    fn rs256_private_pem() -> &'static str {
        include_str!("../tests/fixtures/test_rsa_private.pem")
    }

    fn rs256_public_pem() -> &'static str {
        include_str!("../tests/fixtures/test_rsa_public.pem")
    }

    fn rs256_other_public_pem() -> &'static str {
        include_str!("../tests/fixtures/test_rsa_other_public.pem")
    }

    fn pinned_now() -> chrono::DateTime<chrono::Utc> {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2025, 6, 1, 12, 0, 0).unwrap()
    }

    fn future_ts(secs: i64) -> i64 {
        (pinned_now() + chrono::Duration::seconds(secs)).timestamp()
    }

    fn make_rs256_token_with_iss(sub: &str, iss: &str) -> String {
        let header = Header::new(Algorithm::RS256);
        let claims = json!({
            "sub": sub,
            "iss": iss,
            "aud": TEST_AUD,
            "exp": future_ts(3600),
        });
        encode(&header, &claims, &EncodingKey::from_rsa_pem(rs256_private_pem().as_bytes()).unwrap())
            .unwrap()
    }

    fn rs256_provider_for_iss(iss: &str, pub_pem: &str) -> Arc<dyn AuthenticationProvider> {
        let resolver = Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Rs256,
            VerificationKey::RsaPem(pub_pem.to_string()),
        ));
        let config = JwtProviderConfig {
            expected_iss: Some(iss.to_string()),
            expected_aud: Some(vec![TEST_AUD.to_string()]),
            leeway_seconds: None,
        };
        Arc::new(
            Rs256AuthenticationProvider::try_new(config, resolver, fixed_clock(pinned_now()))
                .expect("valid JWT provider config"),
        )
    }

    fn build_multi_issuer(
        pairs: Vec<(&str, Arc<dyn AuthenticationProvider>)>,
    ) -> MultiIssuerAuthenticationProvider {
        let map: HashMap<String, Arc<dyn AuthenticationProvider>> =
            pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        MultiIssuerAuthenticationProvider::new(Arc::new(StaticIssuerResolver::new(map)))
    }

    // -----------------------------------------------------------------------
    // unverified_iss
    // -----------------------------------------------------------------------

    #[test]
    fn unverified_iss_extracts_iss_from_valid_jwt() {
        let token = make_rs256_token_with_iss("user", "https://issuer-a.example.com");
        let iss = unverified_iss(&token).unwrap();
        assert_eq!(iss, "https://issuer-a.example.com");
    }

    #[test]
    fn unverified_iss_handles_base64url_chars() {
        // Manually craft a minimal base64url payload with - and _ characters.
        // JSON: {"iss":"ab+cd/ef"} — when encoded as base64url, may produce - and _.
        let payload = json!({ "iss": "issuer-with_dash" });
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_bytes);
        let fake_token = format!("header.{payload_b64}.signature");
        let iss = unverified_iss(&fake_token).unwrap();
        assert_eq!(iss, "issuer-with_dash");
    }

    #[test]
    fn unverified_iss_returns_none_for_non_jwt() {
        assert!(unverified_iss("not-a-jwt").is_none());
        assert!(unverified_iss("two.parts").is_none());
    }

    #[test]
    fn unverified_iss_returns_none_when_iss_absent() {
        let payload = json!({ "sub": "user" });
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        let payload_b64 = URL_SAFE_NO_PAD.encode(&payload_bytes);
        let fake_token = format!("header.{payload_b64}.signature");
        assert!(unverified_iss(&fake_token).is_none());
    }

    #[test]
    fn unverified_iss_handles_padded_base64url_payload() {
        // Some IdPs emit base64url WITH padding ('='). Craft a payload whose base64url
        // encoding requires '=' padding characters and verify unverified_iss still works.
        // JSON bytes length is chosen so that base64 encoding is not a multiple of 3,
        // forcing '=' padding. We build it with URL_SAFE (padded) engine.
        let payload = json!({ "iss": "https://padded-issuer.example.com", "sub": "u" });
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        // Encode WITH padding using URL_SAFE engine.
        let padded_b64 = URL_SAFE.encode(&payload_bytes);
        // Confirm that the encoded string actually contains padding (otherwise the test
        // would not exercise the fallback path).
        assert!(
            padded_b64.contains('='),
            "test setup: padded_b64 must contain '=' to exercise the fallback"
        );
        let fake_token = format!("header.{padded_b64}.signature");
        let iss = unverified_iss(&fake_token).expect("unverified_iss must succeed for padded payload");
        assert_eq!(iss, "https://padded-issuer.example.com");
    }

    // -----------------------------------------------------------------------
    // MultiIssuerAuthenticationProvider
    // -----------------------------------------------------------------------

    #[test]
    fn routes_token_from_issuer_a_to_provider_a() {
        let provider_a = rs256_provider_for_iss("https://issuer-a.example.com", rs256_public_pem());
        let multi = build_multi_issuer(vec![
            ("https://issuer-a.example.com", provider_a),
        ]);

        let token = make_rs256_token_with_iss("user-a", "https://issuer-a.example.com");
        let ctx = multi.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-a");
    }

    #[test]
    fn unknown_iss_returns_invalid_token() {
        let provider_a = rs256_provider_for_iss("https://issuer-a.example.com", rs256_public_pem());
        let multi = build_multi_issuer(vec![
            ("https://issuer-a.example.com", provider_a),
        ]);

        let token = make_rs256_token_with_iss("user", "https://unknown.example.com");
        let err = multi.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken for unknown issuer, got {err:?}"
        );
    }

    #[test]
    fn forged_iss_routes_to_wrong_provider_and_fails_signature() {
        // Token claims iss=A but is signed by issuer B's key.
        // Provider A has only A's public key → signature fails.
        // Sign with "other" key but claim iss=A
        let header = Header::new(Algorithm::RS256);
        let claims = json!({
            "sub": "attacker",
            "iss": "https://issuer-a.example.com",
            "exp": future_ts(3600),
        });
        // Use a *different* private key (we don't have one directly, but
        // we can manually corrupt the signature to simulate a mismatch).
        // Instead: encode with the real private key (which corresponds to public_pem)
        // but route to provider_b which has other_public_pem. We need a separate scenario:
        // Provider B is registered as iss=A but has B's public key.
        let provider_b_with_wrong_key = {
            let resolver = Arc::new(LocalKeyResolver::new(
                JwtAlgorithm::Rs256,
                VerificationKey::RsaPem(rs256_other_public_pem().to_string()),
            ));
            let config = JwtProviderConfig {
                expected_iss: Some("https://issuer-a.example.com".to_string()),
                expected_aud: Some(vec![TEST_AUD.to_string()]),
                leeway_seconds: None,
            };
            Arc::new(
                Rs256AuthenticationProvider::try_new(config, resolver, fixed_clock(pinned_now()))
                    .expect("valid JWT provider config"),
            ) as Arc<dyn AuthenticationProvider>
        };

        let multi2 = build_multi_issuer(vec![
            ("https://issuer-a.example.com", provider_b_with_wrong_key),
        ]);

        // Token is signed with the real private key (corresponds to public_pem, not other_public_pem)
        let token = encode(
            &header,
            &claims,
            &EncodingKey::from_rsa_pem(rs256_private_pem().as_bytes()).unwrap(),
        )
        .unwrap();

        let err = multi2.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(
            err,
            AuthenticationError::InvalidSignature,
            "forged iss should route to wrong provider and fail signature"
        );
    }

    fn rs256_other_private_pem() -> &'static str {
        include_str!("../tests/fixtures/test_rsa_other_private.pem")
    }

    // Real cross-provider attack: attacker holds a token signed by B's key,
    // forges iss=A. Router sends it to Provider A (which has A's public key).
    // B's signature cannot be verified with A's key → InvalidSignature.
    #[test]
    fn forged_iss_using_b_token_routed_to_a_fails_signature() {
        let provider_a = rs256_provider_for_iss("https://issuer-a.example.com", rs256_public_pem());
        let provider_b = rs256_provider_for_iss("https://issuer-b.example.com", rs256_other_public_pem());
        let multi = build_multi_issuer(vec![
            ("https://issuer-a.example.com", provider_a),
            ("https://issuer-b.example.com", provider_b),
        ]);

        // Attacker signs with B's private key but claims iss=A
        let header = Header::new(Algorithm::RS256);
        let claims = json!({
            "sub": "attacker",
            "iss": "https://issuer-a.example.com",
            "exp": future_ts(3600),
        });
        let token = encode(
            &header,
            &claims,
            &EncodingKey::from_rsa_pem(rs256_other_private_pem().as_bytes()).unwrap(),
        )
        .unwrap();

        // Routes to Provider A (A's pubkey) — B's signature fails
        let err = multi.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(
            err,
            AuthenticationError::InvalidSignature,
            "B-signed token claiming iss=A must fail signature at Provider A"
        );
    }

    #[test]
    fn multi_issuer_is_usable_as_arc_dyn_authentication_provider() {
        // Compile-time object-safety check
        fn accepts_provider(_: Arc<dyn AuthenticationProvider>) {}
        let multi = Arc::new(build_multi_issuer(vec![]));
        accepts_provider(multi);
    }

    #[test]
    fn token_over_8kib_returns_invalid_token_before_decode() {
        let multi = build_multi_issuer(vec![]);
        let huge = "x".repeat(8193);
        let err = multi.authenticate(&Credential::Bearer(huge)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }
}
