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

use std::sync::Arc;

use ego_domain::auth::{AuthenticationError, Clock};
use ego_security_sdk::{AuthenticationProvider, Credential, SecurityContext};
use jsonwebtoken::{Algorithm, DecodingKey};

use crate::config::{JwtAlgorithm, JwtConfig, JwtProviderConfig};
use crate::key_resolver::{KeyResolver, KeyResolverError, VerificationKey};
use crate::validation::{JwtValidationEngine, ValidationParams};

// ---------------------------------------------------------------------------
// JwtAuthenticator
// ---------------------------------------------------------------------------

/// A synchronous JWT authenticator.
///
/// Validates a `Bearer` credential by:
/// 1. Resolving the verification key via the injected [`KeyResolver`].
/// 2. Verifying the token signature using the resolved key and algorithm.
/// 3. Rejecting tokens whose `exp` has passed (using the injected [`Clock`]).
/// 4. Rejecting tokens whose `nbf` has not yet been reached.
/// 5. Optionally validating `iss` and `aud` claims.
/// 6. Extracting `sub` (strict — absent or non-string → error), and
///    `roles`/`tenant_id`/`tid` (graceful — wrong type is skipped) into a [`Principal`].
///
/// # Clocks
///
/// This authenticator NEVER calls `Utc::now()` directly. All time-sensitive
/// checks go through the injected `Arc<dyn Clock>`.
///
/// # Key Resolver
///
/// Key material is not embedded in this struct. The injected `Arc<dyn KeyResolver>`
/// is called on every `authenticate()` invocation. The resolver MUST satisfy
/// the cache-first contract (AD-013): `resolve` must complete from local state
/// without I/O.
pub struct JwtAuthenticator {
    config: JwtConfig,
    resolver: Arc<dyn KeyResolver>,
    clock: Arc<dyn Clock>,
}

impl JwtAuthenticator {
    /// Constructs a new authenticator.
    ///
    /// - `config`: algorithm discriminant and optional iss/aud constraints.
    ///   Key material is NOT stored here — it lives in `resolver`.
    /// - `resolver`: provides the [`VerificationKey`] for each authenticate call.
    ///   Multiple authenticators MAY share one `Arc<dyn KeyResolver>` (NFR-010).
    /// - `clock`: injectable time source — use a mock in tests.
    pub fn new(config: JwtConfig, resolver: Arc<dyn KeyResolver>, clock: Arc<dyn Clock>) -> Self {
        Self { config, resolver, clock }
    }
}

impl AuthenticationProvider for JwtAuthenticator {
    fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        let token = bearer_token(credential)?;
        // Map the configured algorithm to the jsonwebtoken + crate discriminant pair, then
        // delegate entirely to authenticate_inner — header parsing, alg enforcement, key
        // resolution, decoding-key construction, and claim validation all happen there.
        let (expected_alg, jwt_alg) = match self.config.algorithm {
            JwtAlgorithm::Hs256 => (Algorithm::HS256, JwtAlgorithm::Hs256),
            JwtAlgorithm::Rs256 => (Algorithm::RS256, JwtAlgorithm::Rs256),
            JwtAlgorithm::Es256 => (Algorithm::ES256, JwtAlgorithm::Es256),
        };
        authenticate_inner(
            token,
            &self.config.validation,
            &self.resolver,
            &self.clock,
            expected_alg,
            jwt_alg,
            |key| {
                // VerificationKey is #[non_exhaustive]; the wildcard arm handles future
                // variants and suppresses the unreachable_patterns lint inside this crate.
                #[allow(unreachable_patterns)]
                match key {
                    VerificationKey::Hmac(bytes) => Ok(DecodingKey::from_secret(bytes)),
                    VerificationKey::RsaPem(pem) => rsa_decoding_key(pem),
                    VerificationKey::EcPem(pem) => ec_decoding_key(pem),
                    _ => Err(AuthenticationError::InvalidToken(
                        "unsupported verification key variant".into(),
                    )),
                }
            },
        )
    }
}

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

/// Bridge an async [`KeyResolver::resolve`] call into a sync context.
///
/// Spawns a fresh OS thread so `futures_executor::block_on` is never called
/// from inside a Tokio worker thread (B-2 fix). The resolver is cache-first
/// (AD-013) so the spawned thread completes immediately.
fn resolve_key_sync(
    resolver: &Arc<dyn KeyResolver>,
    kid: Option<String>,
    algorithm: JwtAlgorithm,
) -> Result<VerificationKey, AuthenticationError> {
    let resolver = Arc::clone(resolver);
    std::thread::spawn(move || {
        futures_executor::block_on(resolver.resolve(kid.as_deref(), algorithm))
    })
    .join()
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
/// Parses the JWT header, enforces the expected algorithm, resolves the
/// verification key, builds the algorithm-specific [`DecodingKey`] via the
/// caller-supplied closure, and delegates full claim validation to
/// [`JwtValidationEngine`].
fn authenticate_inner(
    token: &str,
    config: &JwtProviderConfig,
    resolver: &Arc<dyn KeyResolver>,
    clock: &Arc<dyn Clock>,
    expected_alg: Algorithm,
    jwt_alg: JwtAlgorithm,
    build_decoding_key: impl FnOnce(&VerificationKey) -> Result<DecodingKey, AuthenticationError>,
) -> Result<SecurityContext, AuthenticationError> {
    // 1. Parse header
    let header = jsonwebtoken::decode_header(token)
        .map_err(|e| AuthenticationError::InvalidToken(format!("{e}")))?;
    // 2. Assert algorithm
    if header.alg != expected_alg {
        return Err(AuthenticationError::AlgorithmNotSupported(format!(
            "expected {expected_alg:?} but token uses {:?}",
            header.alg
        )));
    }
    // 3. Resolve key
    let verification_key = resolve_key_sync(resolver, header.kid.clone(), jwt_alg)?;
    // 4. Build decoding key (caller-supplied, algorithm-specific)
    let decoding_key = build_decoding_key(&verification_key)?;
    // 5. Validate
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
    use ego_security_sdk::principal::Role;
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

    fn hs256_wrong_secret() -> Vec<u8> {
        b"wrong-secret".to_vec()
    }

    fn hs256_resolver() -> Arc<dyn KeyResolver> {
        Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(hs256_secret()),
        ))
    }

    fn hs256_wrong_resolver() -> Arc<dyn KeyResolver> {
        Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(hs256_wrong_secret()),
        ))
    }

    fn hs256_config() -> JwtConfig {
        JwtConfig {
            algorithm: JwtAlgorithm::Hs256,
            validation: crate::config::JwtProviderConfig::default(),
        }
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

    fn rs256_config() -> JwtConfig {
        JwtConfig {
            algorithm: JwtAlgorithm::Rs256,
            validation: crate::config::JwtProviderConfig::default(),
        }
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

    fn ec_config() -> JwtConfig {
        JwtConfig {
            algorithm: JwtAlgorithm::Es256,
            validation: crate::config::JwtProviderConfig::default(),
        }
    }

    // -----------------------------------------------------------------------
    // C-1: ES256 valid token
    // -----------------------------------------------------------------------

    #[test]
    fn es256_valid_token_returns_security_context() {
        let claims = json!({ "sub": "es256-user", "exp": pinned_future_ts(3600) });
        let token = make_ec_token(&claims);
        let auth = JwtAuthenticator::new(ec_config(), ec_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "es256-user");
    }

    // -----------------------------------------------------------------------
    // C-1: ES256 mismatched key → InvalidSignature
    // -----------------------------------------------------------------------

    #[test]
    fn es256_mismatched_key_returns_invalid_signature() {
        let now = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
        let exp = (now + chrono::Duration::seconds(3600)).timestamp();
        let claims = json!({ "sub": "es256-user", "exp": exp });
        let token = make_ec_token(&claims);
        // Verify with the OTHER public key — signature mismatch
        let other_resolver = ec_other_resolver();
        let auth = JwtAuthenticator::new(ec_config(), other_resolver, fixed_clock(now));
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    // -----------------------------------------------------------------------
    // C-1: ES256 authenticator configured for ES256 rejects HS256 token
    // -----------------------------------------------------------------------

    #[test]
    fn es256_provider_rejects_hs256_token() {
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let hs256_token = make_hs256_token(&claims);
        // Authenticator configured for ES256 — HS256 header alg must be rejected
        let auth = JwtAuthenticator::new(ec_config(), ec_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(hs256_token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::AlgorithmNotSupported(_)));
    }

    // -----------------------------------------------------------------------
    // FR-006: HS256 valid token
    // -----------------------------------------------------------------------

    #[test]
    fn hs256_valid_token_returns_security_context() {
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    // -----------------------------------------------------------------------
    // HS256 wrong secret → InvalidSignature
    // -----------------------------------------------------------------------

    #[test]
    fn hs256_wrong_secret_returns_invalid_signature() {
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_wrong_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    // -----------------------------------------------------------------------
    // FR-007: RS256 valid token
    // -----------------------------------------------------------------------

    #[test]
    fn rs256_valid_token_returns_security_context() {
        let claims = json!({ "sub": "rs256-user", "exp": pinned_future_ts(3600) });
        let token = make_rs256_token(&claims);
        let auth = JwtAuthenticator::new(rs256_config(), rs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "rs256-user");
    }

    // -----------------------------------------------------------------------
    // RS256 mismatched key → InvalidSignature
    // -----------------------------------------------------------------------

    #[test]
    fn rs256_mismatched_key_returns_invalid_signature() {
        let claims = json!({ "sub": "rs256-user", "exp": pinned_future_ts(3600) });
        // Signed with primary key but verified with OTHER public key
        let token = make_rs256_token(&claims);
        let auth = JwtAuthenticator::new(rs256_config(), rs256_other_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    // -----------------------------------------------------------------------
    // FR-008: exp expired → ExpiredToken
    // -----------------------------------------------------------------------

    #[test]
    fn expired_token_returns_expired_error() {
        // exp is in the past — clock is "now"
        let exp_secs = pinned_past_ts(60);
        let claims = json!({ "sub": "user-1", "exp": exp_secs });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::ExpiredToken);
    }

    // -----------------------------------------------------------------------
    // FR-008 boundary: exp == now → ExpiredToken
    // -----------------------------------------------------------------------

    #[test]
    fn token_exp_equal_to_now_is_expired() {
        let now = Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let exp_secs = now.timestamp();
        let claims = json!({ "sub": "user-1", "exp": exp_secs });
        let token = make_hs256_token(&claims);
        // clock returns exactly `now`, which equals exp
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), fixed_clock(now));
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::ExpiredToken);
    }

    // -----------------------------------------------------------------------
    // No exp claim → Ok
    // -----------------------------------------------------------------------

    #[test]
    fn token_without_exp_is_accepted() {
        let claims = json!({ "sub": "user-1" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    // -----------------------------------------------------------------------
    // FR-011: future nbf → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn token_with_future_nbf_is_rejected() {
        let nbf_secs = pinned_future_ts(300); // not valid for 5 minutes
        let claims = json!({ "sub": "user-1", "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // FR-011: past nbf → Ok
    // -----------------------------------------------------------------------

    #[test]
    fn token_with_past_nbf_is_accepted() {
        let nbf_secs = pinned_past_ts(300);
        let claims = json!({ "sub": "user-1", "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    // -----------------------------------------------------------------------
    // Malformed string → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn malformed_token_returns_invalid_token() {
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth
            .authenticate(&Credential::Bearer("not.a.jwt".into()))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // iss validation
    // -----------------------------------------------------------------------

    #[test]
    fn unexpected_iss_returns_invalid_token() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256,
            validation: crate::config::JwtProviderConfig {
                expected_iss: Some("my-service".into()),
                expected_aud: None,
            },
        };
        let claims = json!({ "sub": "user-1", "iss": "other-service" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn no_iss_configured_accepts_any_iss() {
        // No expected_iss → accept any iss or absent iss
        let claims = json!({ "sub": "user-1", "iss": "random-issuer" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    #[test]
    fn correct_iss_is_accepted() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256,
            validation: crate::config::JwtProviderConfig {
                expected_iss: Some("trusted-iss".into()),
                expected_aud: None,
            },
        };
        let claims = json!({ "sub": "user-1", "iss": "trusted-iss" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    // -----------------------------------------------------------------------
    // aud validation
    // -----------------------------------------------------------------------

    #[test]
    fn unexpected_aud_returns_invalid_token() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256,
            validation: crate::config::JwtProviderConfig {
                expected_iss: None,
                expected_aud: Some(vec!["my-api".into()]),
            },
        };
        let claims = json!({ "sub": "user-1", "aud": ["other-api"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn correct_aud_is_accepted() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256,
            validation: crate::config::JwtProviderConfig {
                expected_iss: None,
                expected_aud: Some(vec!["my-api".into()]),
            },
        };
        let claims = json!({ "sub": "user-1", "aud": ["my-api"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    // -----------------------------------------------------------------------
    // AlgorithmNotSupported — header alg doesn't match config
    // -----------------------------------------------------------------------

    #[test]
    fn algorithm_mismatch_returns_not_supported() {
        // Present a valid RS256 token to an HS256 config — alg mismatch detected at header time.
        let rs256_token = make_rs256_token(&json!({ "sub": "user-1" }));
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(rs256_token)).unwrap_err();
        // Fix B-1: config.algorithm mismatch is now detected immediately after header parse,
        // before the resolver is called — so this MUST be AlgorithmNotSupported.
        assert!(matches!(err, AuthenticationError::AlgorithmNotSupported(_)));
    }

    // -----------------------------------------------------------------------
    // CLAR-005: sub claim — all failure modes reject the token
    // -----------------------------------------------------------------------

    #[test]
    fn sub_integer_returns_invalid_token() {
        let claims = json!({ "sub": 12345 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn sub_boolean_returns_invalid_token() {
        let claims = json!({ "sub": true });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn sub_object_returns_invalid_token() {
        let claims = json!({ "sub": { "id": "user-1" } });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn sub_array_returns_invalid_token() {
        let claims = json!({ "sub": ["user-1"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn sub_empty_string_returns_invalid_token() {
        let claims = json!({ "sub": "" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // CLAR-005: wrong-type roles → graceful degradation
    // -----------------------------------------------------------------------

    #[test]
    fn wrong_type_roles_produces_empty_set_and_preserves_raw() {
        // roles is a string instead of an array
        let claims = json!({ "sub": "user-1", "roles": "admin" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert!(ctx.principal.roles.is_empty());
        assert_eq!(ctx.claims.custom.get("roles"), Some(&json!("admin")));
    }

    #[test]
    fn roles_array_of_strings_is_extracted_correctly() {
        let claims = json!({ "sub": "user-1", "roles": ["admin", "editor"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert!(ctx.principal.roles.contains(&Role("admin".into())));
        assert!(ctx.principal.roles.contains(&Role("editor".into())));
        assert_eq!(ctx.principal.roles.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Claims.custom lexicographic order (BTreeMap)
    // -----------------------------------------------------------------------

    #[test]
    fn custom_claims_are_in_lexicographic_order() {
        let claims = json!({
            "sub": "user-1",
            "z_custom": "last",
            "a_custom": "first",
            "m_custom": "middle"
        });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        let keys: Vec<&str> = ctx.claims.custom.keys().map(|s| s.as_str()).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "custom claims must be in lexicographic order");
    }

    // -----------------------------------------------------------------------
    // No HashMap in public API
    // -----------------------------------------------------------------------

    #[test]
    fn security_context_uses_btree_types_not_hashmap() {
        use std::any::type_name;
        let claims_type = type_name::<ego_domain::auth::Claims>();
        // BTreeMap should appear in type name, HashMap should not
        assert!(!claims_type.contains("HashMap"));
    }

    // -----------------------------------------------------------------------
    // tenant_id extraction
    // -----------------------------------------------------------------------

    #[test]
    fn tenant_id_claim_is_extracted() {
        let claims = json!({ "sub": "user-1", "tenant_id": "acme" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.tenant_id, Some("acme".into()));
    }

    #[test]
    fn tid_claim_is_extracted_as_tenant_id() {
        let claims = json!({ "sub": "user-1", "tid": "contoso" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.tenant_id, Some("contoso".into()));
    }

    #[test]
    fn wrong_type_tenant_id_produces_none_and_preserves_raw() {
        let claims = json!({ "sub": "user-1", "tenant_id": 999 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.tenant_id, None);
        assert_eq!(ctx.claims.custom.get("tenant_id"), Some(&json!(999)));
    }

    #[test]
    fn wrong_type_tid_alias_preserves_raw_under_tid_key() {
        // CLAR-005: wrong-type `tid` (the alias) must reappear under "tid",
        // NOT renamed to "tenant_id" in custom claims.
        let claims = json!({ "sub": "user-1", "tid": 42 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.tenant_id, None);
        assert_eq!(ctx.claims.custom.get("tid"), Some(&json!(42)));
        assert!(!ctx.claims.custom.contains_key("tenant_id"));
    }

    // -----------------------------------------------------------------------
    // Standard claims populated correctly
    // -----------------------------------------------------------------------

    #[test]
    fn standard_claims_are_populated() {
        let now_ts = pinned_now().timestamp();
        let exp_ts = pinned_future_ts(3600);
        let claims = json!({
            "sub": "user-1",
            "exp": exp_ts,
            "iat": now_ts,
            "iss": "test-iss",
            "jti": "my-jti-123"
        });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert!(ctx.claims.standard.exp.is_some());
        assert!(ctx.claims.standard.iat.is_some());
        assert_eq!(ctx.claims.standard.iss, Some("test-iss".into()));
        assert_eq!(ctx.claims.standard.jti, Some("my-jti-123".into()));
    }

    // -----------------------------------------------------------------------
    // Mixed-type roles array → graceful degradation
    // -----------------------------------------------------------------------

    #[test]
    fn mixed_type_roles_array_produces_empty_set() {
        let claims = json!({ "sub": "user-1", "roles": ["admin", 42] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        // Mixed array → graceful degradation
        assert!(ctx.principal.roles.is_empty());
    }

    // -----------------------------------------------------------------------
    // FIX-1: exp with non-integer JSON type must return InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn exp_as_string_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "exp": "never" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    #[test]
    fn exp_as_float_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "exp": 9_999_999_999.5_f64 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    #[test]
    fn exp_as_bool_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "exp": true });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    #[test]
    fn nbf_as_string_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "nbf": "yesterday" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    #[test]
    fn nbf_as_float_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "nbf": 0.5_f64 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    #[test]
    fn nbf_as_bool_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "nbf": false });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // FIX-2: absent sub must return MissingClaim("sub")
    // -----------------------------------------------------------------------

    #[test]
    fn token_without_sub_returns_missing_claim() {
        // Token has no "sub" key at all
        let claims = json!({ "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(&err, AuthenticationError::MissingClaim(s) if s == "sub"),
            "expected MissingClaim(\"sub\"), got {err:?}"
        );
    }

    #[test]
    fn token_with_valid_sub_returns_security_context() {
        let claims = json!({ "sub": "happy-user", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), pinned_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "happy-user");
    }

    // -----------------------------------------------------------------------
    // FIX-4: iss absent with expected value configured → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn expected_iss_configured_but_token_has_no_iss_returns_invalid_token() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256,
            validation: crate::config::JwtProviderConfig {
                expected_iss: Some("https://auth.example.com".into()),
                expected_aud: None,
            },
        };
        // Token has no "iss" key
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken for absent iss, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // FIX-4: aud absent with expected value configured → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn expected_aud_configured_but_token_has_no_aud_returns_invalid_token() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256,
            validation: crate::config::JwtProviderConfig {
                expected_iss: None,
                expected_aud: Some(vec!["api.example.com".into()]),
            },
        };
        // Token has no "aud" key
        let claims = json!({ "sub": "user-1", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken for absent aud, got {err:?}"
        );
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
    // 5.1: kid from JWT header passed to resolver
    // -----------------------------------------------------------------------

    #[test]
    fn authenticator_passes_kid_from_header_to_resolver() {
        let now = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
        let exp = (now + chrono::Duration::seconds(3600)).timestamp();
        let (capturing, received_kid) =
            CapturingResolver::new(VerificationKey::Hmac(hs256_secret()), Some(JwtAlgorithm::Hs256));
        let resolver: Arc<dyn KeyResolver> = capturing;

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        header.kid = Some("primary-key".into());
        let claims = json!({ "sub": "user-1", "exp": exp });
        let token =
            jsonwebtoken::encode(&header, &claims, &EncodingKey::from_secret(&hs256_secret()))
                .unwrap();

        let auth = JwtAuthenticator::new(hs256_config(), resolver, fixed_clock(now));
        let _ = auth.authenticate(&Credential::Bearer(token)).unwrap();

        let kid = received_kid.lock().unwrap().take();
        assert_eq!(kid, Some(Some("primary-key".to_owned())));
    }

    // -----------------------------------------------------------------------
    // 5.2: kid = None when JWT has no kid field
    // -----------------------------------------------------------------------

    #[test]
    fn authenticator_passes_none_when_token_has_no_kid() {
        let now = Utc.with_ymd_and_hms(2025, 6, 1, 12, 0, 0).unwrap();
        let exp = (now + chrono::Duration::seconds(3600)).timestamp();
        let (capturing, received_kid) =
            CapturingResolver::new(VerificationKey::Hmac(hs256_secret()), Some(JwtAlgorithm::Hs256));
        let resolver: Arc<dyn KeyResolver> = capturing;

        // Default Header::new has kid = None
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let claims = json!({ "sub": "user-1", "exp": exp });
        let token =
            jsonwebtoken::encode(&header, &claims, &EncodingKey::from_secret(&hs256_secret()))
                .unwrap();

        let auth = JwtAuthenticator::new(hs256_config(), resolver, fixed_clock(now));
        let _ = auth.authenticate(&Credential::Bearer(token)).unwrap();

        let kid = received_kid.lock().unwrap().take();
        assert_eq!(kid, Some(None));
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
    // 5.3: KeyNotFound → InvalidSignature
    // -----------------------------------------------------------------------

    #[test]
    fn key_not_found_maps_to_invalid_signature() {
        let resolver: Arc<dyn KeyResolver> = Arc::new(FailingResolver {
            error: KeyResolverError::KeyNotFound { kid: None },
        });

        let claims = json!({ "sub": "user-1" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), resolver, pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    // -----------------------------------------------------------------------
    // 5.4: AlgorithmMismatch → AlgorithmNotSupported
    // -----------------------------------------------------------------------

    #[test]
    fn algorithm_mismatch_maps_to_not_supported() {
        let resolver: Arc<dyn KeyResolver> = Arc::new(FailingResolver {
            error: KeyResolverError::AlgorithmMismatch {
                expected: JwtAlgorithm::Hs256,
                requested: JwtAlgorithm::Rs256,
            },
        });

        let claims = json!({ "sub": "user-1" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), resolver, pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::AlgorithmNotSupported(_)));
    }

    // -----------------------------------------------------------------------
    // 5.5: InvalidKeyMaterial → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn invalid_key_material_maps_to_invalid_token() {
        let resolver: Arc<dyn KeyResolver> = Arc::new(FailingResolver {
            error: KeyResolverError::InvalidKeyMaterial("bad pem".into()),
        });

        let claims = json!({ "sub": "user-1" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), resolver, pinned_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // 5.6: Shared resolver — two authenticators sharing one Arc (AC-019)
    // -----------------------------------------------------------------------

    #[test]
    fn shared_resolver_authenticates_across_multiple_instances() {
        let shared: Arc<dyn KeyResolver> = Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(hs256_secret()),
        ));

        let auth_a = JwtAuthenticator::new(hs256_config(), Arc::clone(&shared), pinned_clock());
        let auth_b = JwtAuthenticator::new(hs256_config(), Arc::clone(&shared), pinned_clock());

        let claims = json!({ "sub": "shared-user", "exp": pinned_future_ts(3600) });
        let token = make_hs256_token(&claims);

        let ctx_a = auth_a.authenticate(&Credential::Bearer(token.clone())).unwrap();
        let ctx_b = auth_b.authenticate(&Credential::Bearer(token)).unwrap();

        assert_eq!(ctx_a.principal.subject_id.as_str(), "shared-user");
        assert_eq!(ctx_b.principal.subject_id.as_str(), "shared-user");
    }

    // -----------------------------------------------------------------------
    // Provider test helpers (moved from providers.rs)
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
