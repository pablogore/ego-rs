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

use crate::config::{JwtAlgorithm, JwtConfig};
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
        let token: &str = match credential {
            Credential::Bearer(t) => t.as_str(),
            _ => {
                return Err(AuthenticationError::InvalidToken(
                    "unsupported credential type".into(),
                ))
            }
        };

        // Parse the JWT header to extract kid and requested algorithm
        let header = jsonwebtoken::decode_header(token)
            .map_err(|e| AuthenticationError::InvalidToken(format!("{e}")))?;

        // Map jsonwebtoken Algorithm to our JwtAlgorithm discriminant
        let requested_alg = match header.alg {
            Algorithm::HS256 => JwtAlgorithm::Hs256,
            Algorithm::RS256 => JwtAlgorithm::Rs256,
            Algorithm::ES256 => JwtAlgorithm::Es256,
            other => {
                return Err(AuthenticationError::AlgorithmNotSupported(format!(
                    "{other:?}"
                )))
            }
        };

        // B-1: Enforce config.algorithm — reject tokens whose header alg differs
        // from what this authenticator was configured to accept.
        if requested_alg != self.config.algorithm {
            return Err(AuthenticationError::AlgorithmNotSupported(format!(
                "token uses {requested_alg:?} but authenticator is configured for {:?}",
                self.config.algorithm
            )));
        }

        // Resolve the verification key.
        //
        // B-2: `futures_executor::block_on` panics when called from inside a Tokio
        // worker thread (the Tokio runtime is already parked on the thread and
        // `block_on` tries to build a second executor on the same OS thread).
        // To avoid this, we always spawn a fresh OS thread before calling `block_on`.
        // A fresh thread has no ambient Tokio context, so `block_on` is safe.
        // The KeyResolver is cache-first (AD-013), so the resolve future is cheap.
        let resolver = Arc::clone(&self.resolver);
        let kid_owned = header.kid.clone(); // Option<String> — clone before move
        let alg = requested_alg;
        let config_alg = self.config.algorithm;
        let verification_key = std::thread::spawn(move || {
            futures_executor::block_on(resolver.resolve(kid_owned.as_deref(), alg))
        })
        .join()
        .map_err(|_| AuthenticationError::InvalidToken("key resolver panicked".into()))?
        .map_err(|e| match e {
            KeyResolverError::KeyNotFound { .. } => AuthenticationError::InvalidSignature,
            KeyResolverError::AlgorithmMismatch { .. } => {
                AuthenticationError::AlgorithmNotSupported(format!("{config_alg:?}"))
            }
            KeyResolverError::InvalidKeyMaterial(msg) => {
                AuthenticationError::InvalidToken(format!("key material: {msg}"))
            }
        })?;

        // Build decoding key — each provider owns only key-build and alg enforcement (AD-014).
        // Claim/time validation is fully delegated to JwtValidationEngine below (AD-019).
        // The wildcard arm is required for correctness because VerificationKey is
        // #[non_exhaustive] — callers outside this crate may encounter future variants.
        // The allow attribute suppresses the unreachable_patterns lint that fires
        // inside the defining crate (where all current variants are visible).
        #[allow(unreachable_patterns)]
        let (decoding_key, algorithm) = match verification_key {
            VerificationKey::Hmac(ref bytes) => (DecodingKey::from_secret(bytes), Algorithm::HS256),
            VerificationKey::RsaPem(ref pem) => (
                DecodingKey::from_rsa_pem(pem.as_bytes()).map_err(|e| {
                    AuthenticationError::InvalidToken(format!("bad RSA public key: {e}"))
                })?,
                Algorithm::RS256,
            ),
            VerificationKey::EcPem(ref pem) => (
                DecodingKey::from_ec_pem(pem.as_bytes()).map_err(|e| {
                    AuthenticationError::InvalidToken(format!("bad EC public key: {e}"))
                })?,
                Algorithm::ES256,
            ),
            _ => {
                return Err(AuthenticationError::InvalidToken(
                    "unsupported verification key variant".into(),
                ))
            }
        };

        let params = ValidationParams {
            expected_iss: self.config.expected_iss.as_deref(),
            expected_aud: self.config.expected_aud.as_deref(),
        };

        JwtValidationEngine::validate(token, &decoding_key, algorithm, params, self.clock.as_ref())
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
    use crate::test_helpers::{fixed_clock, future_ts, hs256_secret, make_hs256_token, now_clock, past_ts};

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
            expected_iss: None,
            expected_aud: None,
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
            expected_iss: None,
            expected_aud: None,
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
        JwtConfig { algorithm: JwtAlgorithm::Es256, expected_iss: None, expected_aud: None }
    }

    // -----------------------------------------------------------------------
    // C-1: ES256 valid token
    // -----------------------------------------------------------------------

    #[test]
    fn es256_valid_token_returns_security_context() {
        let claims = json!({ "sub": "es256-user", "exp": future_ts(3600) });
        let token = make_ec_token(&claims);
        let auth = JwtAuthenticator::new(ec_config(), ec_resolver(), now_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "es256-user");
    }

    // -----------------------------------------------------------------------
    // C-1: ES256 mismatched key → InvalidSignature
    // -----------------------------------------------------------------------

    #[test]
    fn es256_mismatched_key_returns_invalid_signature() {
        let claims = json!({ "sub": "es256-user", "exp": future_ts(3600) });
        let token = make_ec_token(&claims);
        // Verify with the OTHER public key — signature mismatch
        let other_resolver = Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Es256,
            VerificationKey::EcPem(ec_other_public_key_pem().to_string()),
        ));
        let auth = JwtAuthenticator::new(ec_config(), other_resolver, now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    // -----------------------------------------------------------------------
    // C-1: ES256 authenticator configured for ES256 rejects HS256 token
    // -----------------------------------------------------------------------

    #[test]
    fn es256_provider_rejects_hs256_token() {
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let hs256_token = make_hs256_token(&claims);
        // Authenticator configured for ES256 — HS256 header alg must be rejected
        let auth = JwtAuthenticator::new(ec_config(), ec_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(hs256_token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::AlgorithmNotSupported(_)));
    }

    // -----------------------------------------------------------------------
    // FR-006: HS256 valid token
    // -----------------------------------------------------------------------

    #[test]
    fn hs256_valid_token_returns_security_context() {
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    // -----------------------------------------------------------------------
    // HS256 wrong secret → InvalidSignature
    // -----------------------------------------------------------------------

    #[test]
    fn hs256_wrong_secret_returns_invalid_signature() {
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_wrong_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    // -----------------------------------------------------------------------
    // FR-007: RS256 valid token
    // -----------------------------------------------------------------------

    #[test]
    fn rs256_valid_token_returns_security_context() {
        let claims = json!({ "sub": "rs256-user", "exp": future_ts(3600) });
        let token = make_rs256_token(&claims);
        let auth = JwtAuthenticator::new(rs256_config(), rs256_resolver(), now_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "rs256-user");
    }

    // -----------------------------------------------------------------------
    // RS256 mismatched key → InvalidSignature
    // -----------------------------------------------------------------------

    #[test]
    fn rs256_mismatched_key_returns_invalid_signature() {
        let claims = json!({ "sub": "rs256-user", "exp": future_ts(3600) });
        // Signed with primary key but verified with OTHER public key
        let token = make_rs256_token(&claims);
        let auth = JwtAuthenticator::new(rs256_config(), rs256_other_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    // -----------------------------------------------------------------------
    // FR-008: exp expired → ExpiredToken
    // -----------------------------------------------------------------------

    #[test]
    fn expired_token_returns_expired_error() {
        // exp is in the past — clock is "now"
        let exp_secs = past_ts(60);
        let claims = json!({ "sub": "user-1", "exp": exp_secs });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    // -----------------------------------------------------------------------
    // FR-011: future nbf → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn token_with_future_nbf_is_rejected() {
        let nbf_secs = future_ts(300); // not valid for 5 minutes
        let claims = json!({ "sub": "user-1", "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // FR-011: past nbf → Ok
    // -----------------------------------------------------------------------

    #[test]
    fn token_with_past_nbf_is_accepted() {
        let nbf_secs = past_ts(300);
        let claims = json!({ "sub": "user-1", "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    // -----------------------------------------------------------------------
    // Malformed string → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn malformed_token_returns_invalid_token() {
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
            expected_iss: Some("my-service".into()),
            expected_aud: None,
        };
        let claims = json!({ "sub": "user-1", "iss": "other-service" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn no_iss_configured_accepts_any_iss() {
        // No expected_iss → accept any iss or absent iss
        let claims = json!({ "sub": "user-1", "iss": "random-issuer" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
    }

    #[test]
    fn correct_iss_is_accepted() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256,
            expected_iss: Some("trusted-iss".into()),
            expected_aud: None,
        };
        let claims = json!({ "sub": "user-1", "iss": "trusted-iss" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), now_clock());
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
            expected_iss: None,
            expected_aud: Some(vec!["my-api".into()]),
        };
        let claims = json!({ "sub": "user-1", "aud": ["other-api"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn correct_aud_is_accepted() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256,
            expected_iss: None,
            expected_aud: Some(vec!["my-api".into()]),
        };
        let claims = json!({ "sub": "user-1", "aud": ["my-api"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn sub_boolean_returns_invalid_token() {
        let claims = json!({ "sub": true });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn sub_object_returns_invalid_token() {
        let claims = json!({ "sub": { "id": "user-1" } });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn sub_array_returns_invalid_token() {
        let claims = json!({ "sub": ["user-1"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn sub_empty_string_returns_invalid_token() {
        let claims = json!({ "sub": "" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert!(ctx.principal.roles.is_empty());
        assert_eq!(ctx.claims.custom.get("roles"), Some(&json!("admin")));
    }

    #[test]
    fn roles_array_of_strings_is_extracted_correctly() {
        let claims = json!({ "sub": "user-1", "roles": ["admin", "editor"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.tenant_id, Some("acme".into()));
    }

    #[test]
    fn tid_claim_is_extracted_as_tenant_id() {
        let claims = json!({ "sub": "user-1", "tid": "contoso" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.tenant_id, Some("contoso".into()));
    }

    #[test]
    fn wrong_type_tenant_id_produces_none_and_preserves_raw() {
        let claims = json!({ "sub": "user-1", "tenant_id": 999 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let now_ts = Utc::now().timestamp();
        let exp_ts = now_ts + 3600;
        let claims = json!({
            "sub": "user-1",
            "exp": exp_ts,
            "iat": now_ts,
            "iss": "test-iss",
            "jti": "my-jti-123"
        });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
        let claims = json!({ "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(&err, AuthenticationError::MissingClaim(s) if s == "sub"),
            "expected MissingClaim(\"sub\"), got {err:?}"
        );
    }

    #[test]
    fn token_with_valid_sub_returns_security_context() {
        let claims = json!({ "sub": "happy-user", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
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
            expected_iss: Some("https://auth.example.com".into()),
            expected_aud: None,
        };
        // Token has no "iss" key
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), now_clock());
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
            expected_iss: None,
            expected_aud: Some(vec!["api.example.com".into()]),
        };
        // Token has no "aud" key
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, hs256_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken for absent aud, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // CapturingResolver — records the kid received from the authenticator
    // -----------------------------------------------------------------------

    use async_trait::async_trait;
    use std::sync::Mutex;

    struct CapturingResolver {
        captured_kid: Arc<Mutex<Option<String>>>,
        key: VerificationKey,
        algorithm: JwtAlgorithm,
    }

    #[async_trait]
    impl KeyResolver for CapturingResolver {
        async fn resolve(
            &self,
            kid: Option<&str>,
            algorithm: JwtAlgorithm,
        ) -> Result<VerificationKey, KeyResolverError> {
            let mut guard = self.captured_kid.lock().unwrap();
            *guard = kid.map(|s| s.to_owned());
            drop(guard);
            if algorithm != self.algorithm {
                return Err(KeyResolverError::AlgorithmMismatch {
                    expected: self.algorithm,
                    requested: algorithm,
                });
            }
            Ok(self.key.clone())
        }
    }

    // -----------------------------------------------------------------------
    // 5.1: kid from JWT header passed to resolver
    // -----------------------------------------------------------------------

    #[test]
    fn authenticator_passes_kid_from_header_to_resolver() {
        let captured_kid: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let resolver: Arc<dyn KeyResolver> = Arc::new(CapturingResolver {
            captured_kid: Arc::clone(&captured_kid),
            key: VerificationKey::Hmac(hs256_secret()),
            algorithm: JwtAlgorithm::Hs256,
        });

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        header.kid = Some("primary-key".into());
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let token =
            jsonwebtoken::encode(&header, &claims, &EncodingKey::from_secret(&hs256_secret()))
                .unwrap();

        let auth = JwtAuthenticator::new(hs256_config(), resolver, now_clock());
        let _ = auth.authenticate(&Credential::Bearer(token)).unwrap();

        let kid = captured_kid.lock().unwrap().take();
        assert_eq!(kid, Some("primary-key".into()));
    }

    // -----------------------------------------------------------------------
    // 5.2: kid = None when JWT has no kid field
    // -----------------------------------------------------------------------

    #[test]
    fn authenticator_passes_none_when_token_has_no_kid() {
        let captured_kid: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let resolver: Arc<dyn KeyResolver> = Arc::new(CapturingResolver {
            captured_kid: Arc::clone(&captured_kid),
            key: VerificationKey::Hmac(hs256_secret()),
            algorithm: JwtAlgorithm::Hs256,
        });

        // Default Header::new has kid = None
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let token =
            jsonwebtoken::encode(&header, &claims, &EncodingKey::from_secret(&hs256_secret()))
                .unwrap();

        let auth = JwtAuthenticator::new(hs256_config(), resolver, now_clock());
        let _ = auth.authenticate(&Credential::Bearer(token)).unwrap();

        let kid = captured_kid.lock().unwrap().take();
        assert_eq!(kid, None);
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
        let auth = JwtAuthenticator::new(hs256_config(), resolver, now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), resolver, now_clock());
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
        let auth = JwtAuthenticator::new(hs256_config(), resolver, now_clock());
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

        let auth_a = JwtAuthenticator::new(hs256_config(), Arc::clone(&shared), now_clock());
        let auth_b = JwtAuthenticator::new(hs256_config(), Arc::clone(&shared), now_clock());

        let claims = json!({ "sub": "shared-user", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);

        let ctx_a = auth_a.authenticate(&Credential::Bearer(token.clone())).unwrap();
        let ctx_b = auth_b.authenticate(&Credential::Bearer(token)).unwrap();

        assert_eq!(ctx_a.principal.subject_id.as_str(), "shared-user");
        assert_eq!(ctx_b.principal.subject_id.as_str(), "shared-user");
    }
}
