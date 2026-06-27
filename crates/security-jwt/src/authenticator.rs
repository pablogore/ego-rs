//! JWT authenticator — implements [`ego_security_sdk::AuthenticationProvider`] for JWT tokens.
//!
//! Validates JWT signatures, standard time claims (`exp`, `nbf`), and
//! optional issuer/audience constraints.
//!
//! **Claim extraction policy**:
//! - `sub` absent → `AuthenticationError::MissingClaim("sub")`
//! - `sub` present but not a string → `AuthenticationError::InvalidToken`
//! - `sub` present but empty → `AuthenticationError::InvalidToken`
//! - `roles` / `tenant_id` / `tid`: wrong type → skip (graceful degradation); raw value preserved in `Claims.custom`

use std::sync::{Arc, OnceLock};

use ego_domain::auth::{AuthenticationError, Clock};
use futures_executor::ThreadPool;
use ego_security_sdk::{AuthenticationProvider, Credential, SecurityContext};
use jsonwebtoken::{Algorithm, DecodingKey};

use crate::config::{JwtAlgorithm, JwtProviderConfig};
use crate::key_resolver::{KeyResolver, KeyResolverError, VerificationKey};
use crate::validation::{JwtValidationEngine, ValidationParams};

// ---------------------------------------------------------------------------
// Single-algorithm providers — shared helpers, macro, and three impl types
// ---------------------------------------------------------------------------

fn map_resolver_error(e: KeyResolverError) -> AuthenticationError {
    match e {
        KeyResolverError::KeyNotFound { .. } => AuthenticationError::InvalidSignature,
        KeyResolverError::AlgorithmMismatch { .. } => {
            AuthenticationError::AlgorithmNotSupported(format!("{e}"))
        }
        KeyResolverError::InvalidKeyMaterial(msg) => {
            AuthenticationError::InvalidToken(format!("key material: {msg}"))
        }
    }
}

static RESOLVER_POOL: OnceLock<ThreadPool> = OnceLock::new();

fn resolver_pool() -> &'static ThreadPool {
    RESOLVER_POOL.get_or_init(|| {
        ThreadPool::builder()
            .pool_size(4)
            .create()
            .expect("failed to create JWT key resolver thread pool")
    })
}

/// Bridge an async [`KeyResolver::resolve`] call into a sync context.
///
/// Submits the resolve future to a bounded thread pool (4 workers) so
/// neither unbounded OS threads nor direct `block_on` inside a Tokio worker
/// thread are used (B-2 fix). The calling thread blocks on an mpsc channel
/// until the pool delivers the result.
fn resolve_key_sync(
    resolver: &Arc<dyn KeyResolver>,
    kid: Option<String>,
    algorithm: JwtAlgorithm,
) -> Result<VerificationKey, AuthenticationError> {
    let resolver = Arc::clone(resolver);
    let (tx, rx) = std::sync::mpsc::channel();
    resolver_pool().spawn_ok(async move {
        let _ = tx.send(resolver.resolve(kid.as_deref(), algorithm).await);
    });
    rx.recv()
        .map_err(|_| AuthenticationError::InvalidToken("key resolver panicked".into()))?
        .map_err(map_resolver_error)
}

/// Extract a bearer token string from a [`Credential`], or return
/// [`AuthenticationError::InvalidToken`] for any other credential type.
fn bearer_token(credential: &Credential) -> Result<&str, AuthenticationError> {
    match credential {
        Credential::Bearer(t) => Ok(t.as_str()),
        _ => Err(AuthenticationError::InvalidToken("unsupported credential type".into())),
    }
}

fn rsa_decoding_key(pem: &str) -> Result<DecodingKey, AuthenticationError> {
    DecodingKey::from_rsa_pem(pem.as_bytes())
        .map_err(|e| AuthenticationError::InvalidToken(format!("bad RSA public key: {e}")))
}

fn ec_decoding_key(pem: &str) -> Result<DecodingKey, AuthenticationError> {
    DecodingKey::from_ec_pem(pem.as_bytes())
        .map_err(|e| AuthenticationError::InvalidToken(format!("bad EC public key: {e}")))
}

/// Core authenticate logic shared by all three providers.
///
/// Parses the JWT header, enforces `expected_alg`, resolves the key via the
/// injected resolver, then calls `build_decoding_key` — a caller-supplied
/// closure that converts the resolved [`VerificationKey`] to a
/// [`DecodingKey`] in an algorithm-specific way. Full claim validation is
/// delegated to [`JwtValidationEngine`].
fn authenticate_inner(
    token: &str,
    config: &JwtProviderConfig,
    resolver: &Arc<dyn KeyResolver>,
    clock: &Arc<dyn Clock>,
    expected_alg: Algorithm,
    jwt_alg: JwtAlgorithm,
    build_decoding_key: impl FnOnce(&VerificationKey) -> Result<DecodingKey, AuthenticationError>,
) -> Result<SecurityContext, AuthenticationError> {
    let header = jsonwebtoken::decode_header(token)
        .map_err(|e| AuthenticationError::InvalidToken(format!("{e}")))?;
    if header.alg != expected_alg {
        return Err(AuthenticationError::AlgorithmNotSupported(format!(
            "expected {expected_alg:?} but token uses {:?}",
            header.alg
        )));
    }
    let verification_key = resolve_key_sync(resolver, header.kid.clone(), jwt_alg)?;
    let decoding_key = build_decoding_key(&verification_key)?;
    let params = ValidationParams {
        expected_iss: config.expected_iss.as_deref(),
        expected_aud: config.expected_aud.as_deref(),
    };
    JwtValidationEngine::validate(token, &decoding_key, expected_alg, params, clock.as_ref())
}

/// Defines a JWT authentication provider struct and its `new()` constructor.
/// Each type stays distinct for algorithm enforcement at the type level.
macro_rules! define_provider {
    ($name:ident) => {
        #[doc = concat!(
            "Single-algorithm JWT authentication provider (`",
            stringify!($name),
            "`).\n\n",
            "Implements [`ego_security_sdk::AuthenticationProvider`]. ",
            "Construct via [`",
            stringify!($name),
            "::new`]."
        )]
        pub struct $name {
            config: JwtProviderConfig,
            resolver: Arc<dyn KeyResolver>,
            clock: Arc<dyn Clock>,
        }

        impl $name {
            #[doc = concat!("Construct a new `", stringify!($name), "`.")]
            pub fn new(
                config: JwtProviderConfig,
                resolver: Arc<dyn KeyResolver>,
                clock: Arc<dyn Clock>,
            ) -> Self {
                Self { config, resolver, clock }
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Hs256AuthenticationProvider
// ---------------------------------------------------------------------------

define_provider!(Hs256AuthenticationProvider);

impl AuthenticationProvider for Hs256AuthenticationProvider {
    /// Authenticate a [`Credential`] as an HS256-signed JWT.
    ///
    /// Tokens whose `alg` header is not `HS256` are rejected with
    /// [`AuthenticationError::AlgorithmNotSupported`] before any key resolution
    /// occurs, providing algorithm-pinning at the type level.
    fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        let token = bearer_token(credential)?;
        authenticate_inner(
            token,
            &self.config,
            &self.resolver,
            &self.clock,
            Algorithm::HS256,
            JwtAlgorithm::Hs256,
            |key| match key {
                VerificationKey::Hmac(bytes) => Ok(DecodingKey::from_secret(bytes)),
                _ => Err(AuthenticationError::InvalidToken("expected HMAC key".into())),
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Rs256AuthenticationProvider
// ---------------------------------------------------------------------------

define_provider!(Rs256AuthenticationProvider);

impl AuthenticationProvider for Rs256AuthenticationProvider {
    /// Authenticate a [`Credential`] as an RS256-signed JWT.
    ///
    /// Tokens whose `alg` header is not `RS256` are rejected with
    /// [`AuthenticationError::AlgorithmNotSupported`] before any key resolution
    /// occurs.
    fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        let token = bearer_token(credential)?;
        authenticate_inner(
            token,
            &self.config,
            &self.resolver,
            &self.clock,
            Algorithm::RS256,
            JwtAlgorithm::Rs256,
            |key| match key {
                VerificationKey::RsaPem(pem) => rsa_decoding_key(pem),
                _ => Err(AuthenticationError::InvalidToken("expected RSA PEM key".into())),
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Es256AuthenticationProvider
// ---------------------------------------------------------------------------

define_provider!(Es256AuthenticationProvider);

impl AuthenticationProvider for Es256AuthenticationProvider {
    /// Authenticate a [`Credential`] as an ES256-signed JWT.
    ///
    /// Tokens whose `alg` header is not `ES256` are rejected with
    /// [`AuthenticationError::AlgorithmNotSupported`] before any key resolution
    /// occurs.
    fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        let token = bearer_token(credential)?;
        authenticate_inner(
            token,
            &self.config,
            &self.resolver,
            &self.clock,
            Algorithm::ES256,
            JwtAlgorithm::Es256,
            |key| match key {
                VerificationKey::EcPem(pem) => ec_decoding_key(pem),
                _ => Err(AuthenticationError::InvalidToken("expected EC PEM key".into())),
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use ego_domain::auth::AuthenticationError;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;

    use crate::key_resolver::{LocalKeyResolver, VerificationKey};
    use chrono::Duration;
    use crate::test_helpers::{fixed_clock, hs256_secret, make_hs256_token};

    // Deterministic time anchor for all clock-sensitive tests (2025-06-01 12:00:00 UTC).
    fn pinned_now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap()
    }

    fn pinned_clock() -> Arc<dyn ego_domain::auth::Clock> {
        fixed_clock(pinned_now())
    }

    fn pinned_future_ts(offset_secs: i64) -> i64 {
        (pinned_now() + Duration::seconds(offset_secs)).timestamp()
    }

    fn pinned_past_ts(offset_secs: i64) -> i64 {
        (pinned_now() - Duration::seconds(offset_secs)).timestamp()
    }

    // -----------------------------------------------------------------------
    // HS256 key helpers
    // -----------------------------------------------------------------------

    fn hs256_resolver() -> Arc<dyn KeyResolver> {
        Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(hs256_secret()),
        ))
    }

    // -----------------------------------------------------------------------
    // RS256 key helpers — 2048-bit test keys (generated offline, not real)
    // -----------------------------------------------------------------------

    // NOTE: These are TEST ONLY keys. Never use in production.
    fn rs256_private_key_pem() -> &'static str {
        include_str!("../tests/fixtures/test_rsa_private.pem")
    }

    fn rs256_public_key_pem() -> &'static str {
        include_str!("../tests/fixtures/test_rsa_public.pem")
    }

    fn rs256_other_public_key_pem() -> &'static str {
        include_str!("../tests/fixtures/test_rsa_other_public.pem")
    }

    fn rs256_resolver() -> Arc<dyn KeyResolver> {
        Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Rs256,
            VerificationKey::RsaPem(rs256_public_key_pem().to_string()),
        ))
    }

    fn rs256_other_resolver() -> Arc<dyn KeyResolver> {
        Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Rs256,
            VerificationKey::RsaPem(rs256_other_public_key_pem().to_string()),
        ))
    }

    fn make_rs256_token(claims: &serde_json::Value) -> String {
        let header = Header::new(Algorithm::RS256);
        encode(
            &header,
            claims,
            &EncodingKey::from_rsa_pem(rs256_private_key_pem().as_bytes()).unwrap(),
        )
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // ES256 key helpers — P-256 test keys (generated offline, not real)
    // -----------------------------------------------------------------------

    // NOTE: These are TEST ONLY keys. Never use in production.
    fn ec_private_key_pem() -> &'static str {
        include_str!("../tests/fixtures/test_ec_private.pem")
    }

    fn ec_public_key_pem() -> &'static str {
        include_str!("../tests/fixtures/test_ec_public.pem")
    }

    fn ec_other_public_key_pem() -> &'static str {
        include_str!("../tests/fixtures/test_ec_other_public.pem")
    }

    fn ec_resolver() -> Arc<dyn KeyResolver> {
        Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Es256,
            VerificationKey::EcPem(ec_public_key_pem().to_string()),
        ))
    }

    fn make_ec_token(claims: &serde_json::Value) -> String {
        let header = Header::new(Algorithm::ES256);
        encode(
            &header,
            claims,
            &EncodingKey::from_ec_pem(ec_private_key_pem().as_bytes()).unwrap(),
        )
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // CapturingResolver — records the kid received from the authenticator;
    // optionally enforces an expected algorithm.
    // -----------------------------------------------------------------------

    use async_trait::async_trait;
    use std::sync::Mutex;

    struct CapturingResolver {
        key: VerificationKey,
        received_kid: Arc<Mutex<Option<Option<String>>>>,
        expected_alg: Option<JwtAlgorithm>,
    }

    impl CapturingResolver {
        fn new(
            key: VerificationKey,
            expected_alg: Option<JwtAlgorithm>,
        ) -> (Arc<Self>, Arc<Mutex<Option<Option<String>>>>) {
            let received_kid = Arc::new(Mutex::new(None));
            let resolver = Arc::new(Self {
                key,
                received_kid: Arc::clone(&received_kid),
                expected_alg,
            });
            (resolver, received_kid)
        }
    }

    #[async_trait]
    impl KeyResolver for CapturingResolver {
        async fn resolve(
            &self,
            kid: Option<&str>,
            algorithm: JwtAlgorithm,
        ) -> Result<VerificationKey, KeyResolverError> {
            let mut guard = self.received_kid.lock().unwrap();
            *guard = Some(kid.map(|s| s.to_owned()));
            drop(guard);
            if let Some(expected) = self.expected_alg {
                if algorithm != expected {
                    return Err(KeyResolverError::AlgorithmMismatch {
                        expected,
                        requested: algorithm,
                    });
                }
            }
            Ok(self.key.clone())
        }
    }

    // -----------------------------------------------------------------------
    // FailingResolver — always returns a specific KeyResolverError
    // -----------------------------------------------------------------------

    struct FailingResolver {
        error: KeyResolverError,
    }

    #[async_trait]
    impl KeyResolver for FailingResolver {
        async fn resolve(
            &self,
            _kid: Option<&str>,
            _algorithm: JwtAlgorithm,
        ) -> Result<VerificationKey, KeyResolverError> {
            Err(self.error.clone())
        }
    }

    // -----------------------------------------------------------------------
    // Provider test helpers
    // -----------------------------------------------------------------------

    fn ec_other_resolver() -> Arc<dyn KeyResolver> {
        Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Es256,
            VerificationKey::EcPem(ec_other_public_key_pem().to_string()),
        ))
    }

    fn default_config() -> crate::config::JwtProviderConfig {
        crate::config::JwtProviderConfig::default()
    }


    // -----------------------------------------------------------------------
    // Hs256AuthenticationProvider tests
    // -----------------------------------------------------------------------

    #[test]
    fn hs256_provider_valid_token_returns_security_context() {
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let provider = Hs256AuthenticationProvider::new(default_config(), hs256_resolver(), pinned_clock());
        let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    #[test]
    fn hs256_provider_wrong_secret_returns_invalid_signature() {
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let wrong_resolver = Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(b"wrong-secret".to_vec()),
        ));
        let provider = Hs256AuthenticationProvider::new(default_config(), wrong_resolver, pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    #[test]
    fn hs256_provider_rejects_rs256_token() {
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let rs256_token = make_rs256_token(&claims);
        let provider = Hs256AuthenticationProvider::new(default_config(), hs256_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(rs256_token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::AlgorithmNotSupported(_)));
    }

    #[test]
    fn hs256_provider_non_bearer_credential_returns_invalid_token() {
        let provider = Hs256AuthenticationProvider::new(default_config(), hs256_resolver(), pinned_clock());
        let err = provider
            .authenticate(&Credential::Basic { username: "user".into(), secret: "pass".into() })
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn hs256_provider_expired_token_returns_expired_token() {
        let claims = json!({ "sub": "user-1", "exp": pinned_past_ts(60) });
        let token = make_hs256_token(&claims);
        let provider = Hs256AuthenticationProvider::new(default_config(), hs256_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::ExpiredToken);
    }

    #[test]
    fn hs256_provider_missing_sub_returns_missing_claim() {
        let claims = json!({ "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let provider = Hs256AuthenticationProvider::new(default_config(), hs256_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(&err, AuthenticationError::MissingClaim(s) if s == "sub"),
            "expected MissingClaim(\"sub\"), got {err:?}"
        );
    }

    #[test]
    fn hs256_provider_key_not_found_returns_invalid_signature() {
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let resolver: Arc<dyn KeyResolver> = Arc::new(FailingResolver { error: KeyResolverError::KeyNotFound { kid: None } });
        let provider = Hs256AuthenticationProvider::new(default_config(), resolver, pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    #[test]
    fn hs256_provider_kid_forwarded_to_resolver() {
        let now = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
        let exp = (now + chrono::Duration::seconds(3600)).timestamp();
        let (capturing, received_kid) =
            CapturingResolver::new(VerificationKey::Hmac(hs256_secret()), None);
        let resolver: Arc<dyn KeyResolver> = capturing;

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        header.kid = Some("my-key-id".into());
        let claims = json!({ "sub": "user-1", "exp": exp });
        let token = jsonwebtoken::encode(&header, &claims, &EncodingKey::from_secret(&hs256_secret())).unwrap();

        let provider = Hs256AuthenticationProvider::new(default_config(), resolver, fixed_clock(now));
        let _ = provider.authenticate(&Credential::Bearer(token)).unwrap();

        let received = received_kid.lock().unwrap().clone();
        assert_eq!(received, Some(Some("my-key-id".to_string())));
    }

    // -----------------------------------------------------------------------
    // Rs256AuthenticationProvider tests
    // -----------------------------------------------------------------------

    #[test]
    fn rs256_provider_valid_token_returns_security_context() {
        let claims = json!({ "sub": "rs256-user", "exp": pinned_future_ts(3600) });
        let token = make_rs256_token(&claims);
        let provider = Rs256AuthenticationProvider::new(default_config(), rs256_resolver(), pinned_clock());
        let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "rs256-user");
    }

    #[test]
    fn rs256_provider_mismatched_key_returns_invalid_signature() {
        let claims = json!({ "sub": "rs256-user", "exp": pinned_future_ts(3600) });
        let token = make_rs256_token(&claims);
        let provider = Rs256AuthenticationProvider::new(default_config(), rs256_other_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    #[test]
    fn rs256_provider_rejects_hs256_token() {
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let hs256_token = make_hs256_token(&claims);
        let provider = Rs256AuthenticationProvider::new(default_config(), rs256_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(hs256_token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::AlgorithmNotSupported(_)));
    }

    #[test]
    fn rs256_provider_key_not_found_returns_invalid_signature() {
        let claims = json!({ "sub": "rs256-user", "exp": pinned_future_ts(3600) });
        let token = make_rs256_token(&claims);
        let resolver: Arc<dyn KeyResolver> = Arc::new(FailingResolver { error: KeyResolverError::KeyNotFound { kid: None } });
        let provider = Rs256AuthenticationProvider::new(default_config(), resolver, pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    #[test]
    fn rs256_provider_non_bearer_credential_returns_invalid_token() {
        let provider = Rs256AuthenticationProvider::new(default_config(), rs256_resolver(), pinned_clock());
        let err = provider
            .authenticate(&Credential::Basic { username: "user".into(), secret: "pass".into() })
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn rs256_provider_expired_token_returns_expired_token() {
        let claims = json!({ "sub": "rs256-user", "exp": pinned_past_ts(60) });
        let token = make_rs256_token(&claims);
        let provider = Rs256AuthenticationProvider::new(default_config(), rs256_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::ExpiredToken);
    }

    #[test]
    fn rs256_provider_missing_sub_returns_missing_claim() {
        let claims = json!({ "exp": pinned_future_ts(3600) });
        let token = make_rs256_token(&claims);
        let provider = Rs256AuthenticationProvider::new(default_config(), rs256_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(&err, AuthenticationError::MissingClaim(s) if s == "sub"),
            "expected MissingClaim(\"sub\"), got {err:?}"
        );
    }

    #[test]
    fn rs256_provider_kid_forwarded_to_resolver() {
        let now = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
        let exp = (now + chrono::Duration::seconds(3600)).timestamp();
        let (capturing, received_kid) =
            CapturingResolver::new(VerificationKey::RsaPem(rs256_public_key_pem().to_string()), None);
        let resolver: Arc<dyn KeyResolver> = capturing;

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some("rs256-key-id".into());
        let claims = json!({ "sub": "rs256-user", "exp": exp });
        let token = jsonwebtoken::encode(
            &header,
            &claims,
            &EncodingKey::from_rsa_pem(rs256_private_key_pem().as_bytes()).unwrap(),
        )
        .unwrap();

        let provider = Rs256AuthenticationProvider::new(default_config(), resolver, fixed_clock(now));
        let _ = provider.authenticate(&Credential::Bearer(token)).unwrap();

        let received = received_kid.lock().unwrap().clone();
        assert_eq!(received, Some(Some("rs256-key-id".to_string())));
    }

    // -----------------------------------------------------------------------
    // Es256AuthenticationProvider tests
    // -----------------------------------------------------------------------

    #[test]
    fn es256_provider_valid_token_returns_security_context() {
        let claims = json!({ "sub": "es256-user", "exp": pinned_future_ts(3600) });
        let token = make_ec_token(&claims);
        let provider = Es256AuthenticationProvider::new(default_config(), ec_resolver(), pinned_clock());
        let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "es256-user");
    }

    #[test]
    fn es256_provider_mismatched_key_returns_invalid_signature() {
        let claims = json!({ "sub": "es256-user", "exp": pinned_future_ts(3600) });
        let token = make_ec_token(&claims);
        let provider = Es256AuthenticationProvider::new(default_config(), ec_other_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    #[test]
    fn es256_authentication_provider_rejects_hs256_token() {
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let hs256_token = make_hs256_token(&claims);
        let provider = Es256AuthenticationProvider::new(default_config(), ec_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(hs256_token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::AlgorithmNotSupported(_)));
    }

    #[test]
    fn es256_provider_key_not_found_returns_invalid_signature() {
        let claims = json!({ "sub": "es256-user", "exp": pinned_future_ts(3600) });
        let token = make_ec_token(&claims);
        let resolver: Arc<dyn KeyResolver> = Arc::new(FailingResolver { error: KeyResolverError::KeyNotFound { kid: None } });
        let provider = Es256AuthenticationProvider::new(default_config(), resolver, pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    #[test]
    fn es256_provider_non_bearer_credential_returns_invalid_token() {
        let provider = Es256AuthenticationProvider::new(default_config(), ec_resolver(), pinned_clock());
        let err = provider
            .authenticate(&Credential::Basic { username: "user".into(), secret: "pass".into() })
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn es256_provider_expired_token_returns_expired_token() {
        let claims = json!({ "sub": "es256-user", "exp": pinned_past_ts(60) });
        let token = make_ec_token(&claims);
        let provider = Es256AuthenticationProvider::new(default_config(), ec_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::ExpiredToken);
    }

    #[test]
    fn es256_provider_missing_sub_returns_missing_claim() {
        let claims = json!({ "exp": pinned_future_ts(3600) });
        let token = make_ec_token(&claims);
        let provider = Es256AuthenticationProvider::new(default_config(), ec_resolver(), pinned_clock());
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(&err, AuthenticationError::MissingClaim(s) if s == "sub"),
            "expected MissingClaim(\"sub\"), got {err:?}"
        );
    }

    #[test]
    fn es256_provider_kid_forwarded_to_resolver() {
        let now = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
        let exp = (now + chrono::Duration::seconds(3600)).timestamp();
        let (capturing, received_kid) =
            CapturingResolver::new(VerificationKey::EcPem(ec_public_key_pem().to_string()), None);
        let resolver: Arc<dyn KeyResolver> = capturing;

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some("ec-key-id".into());
        let claims = json!({ "sub": "es256-user", "exp": exp });
        let token = jsonwebtoken::encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(ec_private_key_pem().as_bytes()).unwrap(),
        )
        .unwrap();

        let provider = Es256AuthenticationProvider::new(default_config(), resolver, fixed_clock(now));
        let _ = provider.authenticate(&Credential::Bearer(token)).unwrap();

        let received = received_kid.lock().unwrap().clone();
        assert_eq!(received, Some(Some("ec-key-id".to_string())));
    }

    // -----------------------------------------------------------------------
    // W2: AlgorithmMismatch + InvalidKeyMaterial per provider
    // -----------------------------------------------------------------------

    #[test]
    fn hs256_provider_algorithm_mismatch_maps_to_not_supported() {
        let resolver = Arc::new(FailingResolver {
            error: KeyResolverError::AlgorithmMismatch {
                expected: JwtAlgorithm::Rs256,
                requested: JwtAlgorithm::Hs256,
            },
        });
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let err = Hs256AuthenticationProvider::new(default_config(), resolver, pinned_clock())
            .authenticate(&Credential::Bearer(token))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::AlgorithmNotSupported(_)));
    }

    #[test]
    fn hs256_provider_invalid_key_material_maps_to_invalid_token() {
        let resolver = Arc::new(FailingResolver {
            error: KeyResolverError::InvalidKeyMaterial("corrupt key".into()),
        });
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let err = Hs256AuthenticationProvider::new(default_config(), resolver, pinned_clock())
            .authenticate(&Credential::Bearer(token))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn rs256_provider_algorithm_mismatch_maps_to_not_supported() {
        let resolver = Arc::new(FailingResolver {
            error: KeyResolverError::AlgorithmMismatch {
                expected: JwtAlgorithm::Hs256,
                requested: JwtAlgorithm::Rs256,
            },
        });
        let claims = json!({ "sub": "rs256-user", "exp": pinned_future_ts(3600) });
        let token = make_rs256_token(&claims);
        let err = Rs256AuthenticationProvider::new(default_config(), resolver, pinned_clock())
            .authenticate(&Credential::Bearer(token))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::AlgorithmNotSupported(_)));
    }

    #[test]
    fn rs256_provider_invalid_key_material_maps_to_invalid_token() {
        let resolver = Arc::new(FailingResolver {
            error: KeyResolverError::InvalidKeyMaterial("corrupt pem".into()),
        });
        let claims = json!({ "sub": "rs256-user", "exp": pinned_future_ts(3600) });
        let token = make_rs256_token(&claims);
        let err = Rs256AuthenticationProvider::new(default_config(), resolver, pinned_clock())
            .authenticate(&Credential::Bearer(token))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn es256_provider_algorithm_mismatch_maps_to_not_supported() {
        let resolver = Arc::new(FailingResolver {
            error: KeyResolverError::AlgorithmMismatch {
                expected: JwtAlgorithm::Hs256,
                requested: JwtAlgorithm::Es256,
            },
        });
        let claims = json!({ "sub": "es256-user", "exp": pinned_future_ts(3600) });
        let token = make_ec_token(&claims);
        let err = Es256AuthenticationProvider::new(default_config(), resolver, pinned_clock())
            .authenticate(&Credential::Bearer(token))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::AlgorithmNotSupported(_)));
    }

    #[test]
    fn es256_provider_invalid_key_material_maps_to_invalid_token() {
        let resolver = Arc::new(FailingResolver {
            error: KeyResolverError::InvalidKeyMaterial("corrupt ec pem".into()),
        });
        let claims = json!({ "sub": "es256-user", "exp": pinned_future_ts(3600) });
        let token = make_ec_token(&claims);
        let err = Es256AuthenticationProvider::new(default_config(), resolver, pinned_clock())
            .authenticate(&Credential::Bearer(token))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // W3: Es256 wrong-variant key (HMAC returned for EC provider)
    // -----------------------------------------------------------------------

    #[test]
    fn es256_wrong_variant_hmac_key_returns_invalid_token() {
        let resolver = Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Es256,
            VerificationKey::Hmac(hs256_secret()),
        ));
        let claims = json!({ "sub": "es256-user", "exp": pinned_future_ts(3600) });
        let token = make_ec_token(&claims);
        let err = Es256AuthenticationProvider::new(default_config(), resolver, pinned_clock())
            .authenticate(&Credential::Bearer(token))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // W4: hs256_no_kid_forwards_none at provider level
    // -----------------------------------------------------------------------

    #[test]
    fn hs256_provider_no_kid_forwards_none_to_resolver() {
        let (capturing, received_kid) =
            CapturingResolver::new(VerificationKey::Hmac(hs256_secret()), None);
        let resolver: Arc<dyn KeyResolver> = capturing;
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        Hs256AuthenticationProvider::new(default_config(), resolver, pinned_clock())
            .authenticate(&Credential::Bearer(token))
            .unwrap();
        assert_eq!(*received_kid.lock().unwrap(), Some(None));
    }

    // -----------------------------------------------------------------------
    // Send + Sync compile-time assertions for all three providers
    // -----------------------------------------------------------------------

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn providers_are_send_sync() {
        assert_send_sync::<Hs256AuthenticationProvider>();
        assert_send_sync::<Rs256AuthenticationProvider>();
        assert_send_sync::<Es256AuthenticationProvider>();
    }
}
