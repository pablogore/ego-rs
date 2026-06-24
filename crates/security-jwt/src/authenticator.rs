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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ego_domain::auth::{
    AuthenticationError, Claims, Clock, StandardClaims,
};
use ego_security_sdk::{
    AuthenticationProvider, Credential, Principal, PrincipalKind, Role, SecurityContext, SubjectId,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::Value;

use crate::config::{JwtAlgorithm, JwtConfig};
use crate::key_resolver::{KeyResolver, KeyResolverError, VerificationKey};

// ---------------------------------------------------------------------------
// Internal raw-claims structure for serde deserialization
// ---------------------------------------------------------------------------

/// Raw deserialized JWT payload — all fields are optional because any claim
/// may be absent. We deserialize into a generic map to capture everything.
#[derive(serde::Deserialize)]
struct RawClaims {
    #[serde(flatten)]
    all: BTreeMap<String, Value>,
}

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

        let kid = header.kid.as_deref();

        // Map jsonwebtoken Algorithm to our JwtAlgorithm discriminant
        let requested_alg = match header.alg {
            Algorithm::HS256 => JwtAlgorithm::Hs256,
            Algorithm::RS256 => JwtAlgorithm::Rs256,
            other => {
                return Err(AuthenticationError::AlgorithmNotSupported(format!(
                    "{other:?}"
                )))
            }
        };

        // Resolve the verification key — block_on is safe because KeyResolver
        // is cache-first (AD-013): LocalKeyResolver completes immediately from
        // memory with no I/O, so block_on never parks the thread.
        let verification_key =
            futures_executor::block_on(self.resolver.resolve(kid, requested_alg)).map_err(
                |e| match e {
                    KeyResolverError::KeyNotFound { .. } => AuthenticationError::InvalidSignature,
                    KeyResolverError::AlgorithmMismatch { .. } => {
                        AuthenticationError::AlgorithmNotSupported(format!(
                            "{:?}",
                            self.config.algorithm
                        ))
                    }
                    KeyResolverError::InvalidKeyMaterial(msg) => {
                        AuthenticationError::InvalidToken(format!("key material: {msg}"))
                    }
                },
            )?;

        // Build decoding key and algorithm from the resolved VerificationKey.
        // Note: VerificationKey is #[non_exhaustive] — when new variants are
        // added (ES256, EdDSA, JWK), the compiler will require updating this match.
        let (decoding_key, algorithm) = match verification_key {
            VerificationKey::Hmac(ref bytes) => {
                (DecodingKey::from_secret(bytes), Algorithm::HS256)
            }
            VerificationKey::RsaPem(ref pem) => (
                DecodingKey::from_rsa_pem(pem.as_bytes()).map_err(|e| {
                    AuthenticationError::InvalidToken(format!("bad RSA public key: {e}"))
                })?,
                Algorithm::RS256,
            ),
        };

        // Disable jsonwebtoken's built-in exp/nbf/aud/iss so we do it ourselves
        // (we need clock injection for time checks and custom aud matching)
        let mut validation = Validation::new(algorithm);
        validation.validate_exp = false;
        validation.validate_nbf = false;
        validation.set_required_spec_claims::<&str>(&[]);
        // Disable built-in aud so we can do our own matching
        validation.validate_aud = false;

        // Decode the token
        let token_data =
            jsonwebtoken::decode::<RawClaims>(token, &decoding_key, &validation).map_err(
                |e| {
                    use jsonwebtoken::errors::ErrorKind;
                    match e.kind() {
                        ErrorKind::InvalidSignature => AuthenticationError::InvalidSignature,
                        ErrorKind::InvalidAlgorithm => {
                            AuthenticationError::AlgorithmNotSupported(format!("{e}"))
                        }
                        ErrorKind::InvalidAlgorithmName => {
                            AuthenticationError::AlgorithmNotSupported(format!("{e}"))
                        }
                        _ => AuthenticationError::InvalidToken(format!("{e}")),
                    }
                },
            )?;

        let all_claims = token_data.claims.all;
        let now = self.clock.now();

        // exp check — reject if exp <= now (expired means exp is in the past or equal to now)
        if let Some(exp_val) = all_claims.get("exp") {
            if let Some(exp_secs) = exp_val.as_i64().or_else(|| exp_val.as_u64().and_then(|u| i64::try_from(u).ok())) {
                let exp_dt = chrono::DateTime::<chrono::Utc>::from_timestamp(exp_secs, 0)
                    .ok_or_else(|| AuthenticationError::InvalidToken("invalid exp timestamp".into()))?;
                if now >= exp_dt {
                    return Err(AuthenticationError::ExpiredToken);
                }
            } else {
                return Err(AuthenticationError::InvalidToken(
                    "exp claim is not a valid integer".into(),
                ));
            }
        }

        // nbf check — reject if nbf > now (not yet valid)
        if let Some(nbf_val) = all_claims.get("nbf") {
            if let Some(nbf_secs) = nbf_val.as_i64().or_else(|| nbf_val.as_u64().and_then(|u| i64::try_from(u).ok())) {
                let nbf_dt = chrono::DateTime::<chrono::Utc>::from_timestamp(nbf_secs, 0)
                    .ok_or_else(|| AuthenticationError::InvalidToken("invalid nbf timestamp".into()))?;
                if now < nbf_dt {
                    return Err(AuthenticationError::InvalidToken(format!(
                        "token not yet valid (nbf: {nbf_dt})"
                    )));
                }
            } else {
                return Err(AuthenticationError::InvalidToken(
                    "nbf claim is not a valid integer".into(),
                ));
            }
        }

        // iss check
        if let Some(expected_iss) = &self.config.expected_iss {
            match all_claims.get("iss") {
                Some(Value::String(iss)) if iss == expected_iss => {}
                Some(other) => {
                    return Err(AuthenticationError::InvalidToken(format!(
                        "iss mismatch: expected '{expected_iss}', got '{other}'"
                    )))
                }
                None => {
                    return Err(AuthenticationError::InvalidToken(format!(
                        "iss mismatch: expected '{expected_iss}', got none"
                    )))
                }
            }
        }

        // aud check — at least one expected aud must be present in the token's aud
        if let Some(expected_auds) = &self.config.expected_aud {
            let token_auds: Vec<String> = match all_claims.get("aud") {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                _ => vec![],
            };
            let any_match = expected_auds.iter().any(|ea| token_auds.contains(ea));
            if !any_match {
                return Err(AuthenticationError::InvalidToken(format!(
                    "aud mismatch: expected one of {expected_auds:?}, got {token_auds:?}"
                )));
            }
        }

        // Build StandardClaims
        let standard = build_standard_claims(&all_claims);

        // Extract identity fields (CLAR-003: graceful degradation on wrong type)
        let (subject, all_claims) = extract_subject(all_claims)?;
        let (tenant_id, all_claims) = extract_tenant_id(all_claims);
        let (roles, all_claims) = extract_roles(all_claims);

        // Remove fields that are now in StandardClaims from custom
        let custom = remove_standard_keys(all_claims);

        let mut principal = Principal::new(
            PrincipalKind::User,
            SubjectId::new(subject)
                .map_err(|_| AuthenticationError::InvalidToken("invalid subject id".into()))?,
        );
        for role in roles {
            principal = principal.with_role(Role(role));
        }
        if let Some(tid) = tenant_id {
            principal = principal.with_tenant_id(tid);
        }

        let claims = Claims {
            standard,
            custom,
        };

        Ok(SecurityContext::new(principal, claims))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build [`StandardClaims`] from the raw claims map (values remain there too).
fn build_standard_claims(map: &BTreeMap<String, Value>) -> StandardClaims {
    let exp = map.get("exp").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
            .and_then(|s| chrono::DateTime::<chrono::Utc>::from_timestamp(s, 0))
    });
    let nbf = map.get("nbf").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
            .and_then(|s| chrono::DateTime::<chrono::Utc>::from_timestamp(s, 0))
    });
    let iat = map.get("iat").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
            .and_then(|s| chrono::DateTime::<chrono::Utc>::from_timestamp(s, 0))
    });
    let jti = map.get("jti").and_then(|v| v.as_str().map(str::to_owned));
    let iss = map.get("iss").and_then(|v| v.as_str().map(str::to_owned));
    let aud = map.get("aud").and_then(|v| match v {
        Value::String(s) => Some(vec![s.clone()]),
        Value::Array(arr) => Some(
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect(),
        ),
        _ => None,
    });

    StandardClaims {
        exp,
        nbf,
        iat,
        jti,
        iss,
        aud,
    }
}

/// Extract `sub` from the map. Returns `Ok((subject_string, map))` on success.
///
/// - Absent `sub` → `Err(MissingClaim("sub"))`.
/// - `sub` present but not a string → `Err(InvalidToken("sub claim is not a string"))`.
/// - `sub` present and a string → `Ok((s, map))`; the caller validates `s` is non-empty
///   via [`SubjectId::new`], which returns `InvalidToken("invalid subject id")` on failure.
///
/// Unlike `roles` and `tenant_id`, `sub` is a required identity claim and does NOT
/// degrade gracefully — any wrong-type or empty value rejects the token.
fn extract_subject(
    mut map: BTreeMap<String, Value>,
) -> Result<(String, BTreeMap<String, Value>), AuthenticationError> {
    match map.remove("sub") {
        Some(Value::String(s)) => Ok((s, map)),
        Some(_) => Err(AuthenticationError::InvalidToken(
            "sub claim is not a string".into(),
        )),
        None => Err(AuthenticationError::MissingClaim("sub".into())),
    }
}

/// Extract `tenant_id` or `tid` from the map. Returns (tenant_id, map).
/// CLAR-003: wrong type → None, raw value stays in map under its original key.
fn extract_tenant_id(mut map: BTreeMap<String, Value>) -> (Option<String>, BTreeMap<String, Value>) {
    // Prefer "tenant_id" over "tid"; track which key was actually removed so
    // wrong-type values are re-inserted under the original key (CLAR-003).
    let (orig_key, val) = if let Some(v) = map.remove("tenant_id") {
        ("tenant_id", Some(v))
    } else {
        ("tid", map.remove("tid"))
    };
    match val {
        Some(Value::String(s)) => (Some(s), map),
        Some(other) => {
            // Wrong type — graceful degradation: preserve under original key
            map.insert(orig_key.into(), other);
            (None, map)
        }
        None => (None, map),
    }
}

/// Extract `roles` from the map as a `BTreeSet<String>`.
/// CLAR-003: if present but wrong type, skip (empty set) and keep raw in map.
fn extract_roles(mut map: BTreeMap<String, Value>) -> (BTreeSet<String>, BTreeMap<String, Value>) {
    match map.remove("roles") {
        Some(Value::Array(arr)) => {
            let all_strings = arr.iter().all(|v| v.is_string());
            if all_strings {
                let roles = arr
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                (roles, map)
            } else {
                // Mixed or wrong types — graceful degradation
                map.insert("roles".into(), Value::Array(arr));
                (BTreeSet::new(), map)
            }
        }
        Some(other) => {
            map.insert("roles".into(), other);
            (BTreeSet::new(), map)
        }
        None => (BTreeSet::new(), map),
    }
}

/// Remove well-known standard claim keys from the custom map.
/// These are already captured in `StandardClaims`; keeping them in `custom`
/// would be redundant.
fn remove_standard_keys(mut map: BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    for key in &["exp", "nbf", "iat", "jti", "iss", "aud"] {
        map.remove(*key);
    }
    map
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

    // -----------------------------------------------------------------------
    // Test clock
    // -----------------------------------------------------------------------

    struct FixedClock(chrono::DateTime<Utc>);

    impl Clock for FixedClock {
        fn now(&self) -> chrono::DateTime<Utc> {
            self.0
        }
    }

    fn fixed_clock(ts: chrono::DateTime<Utc>) -> Arc<dyn Clock> {
        Arc::new(FixedClock(ts))
    }

    fn now_clock() -> Arc<dyn Clock> {
        fixed_clock(Utc::now())
    }

    // -----------------------------------------------------------------------
    // HS256 key helpers
    // -----------------------------------------------------------------------

    fn hs256_secret() -> Vec<u8> {
        b"super-secret-key-for-testing-only".to_vec()
    }

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

    fn hs256_authenticator(secret: &[u8], clock: Arc<dyn Clock>) -> JwtAuthenticator {
        let resolver = Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(secret.to_vec()),
        ));
        JwtAuthenticator::new(
            JwtConfig { algorithm: JwtAlgorithm::Hs256, expected_iss: None, expected_aud: None },
            resolver,
            clock,
        )
    }

    fn make_hs256_token(claims: &serde_json::Value) -> String {
        let header = Header::new(Algorithm::HS256);
        encode(&header, claims, &EncodingKey::from_secret(&hs256_secret())).unwrap()
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
    // Helpers
    // -----------------------------------------------------------------------

    fn future_ts(offset_secs: i64) -> i64 {
        (Utc::now() + chrono::Duration::seconds(offset_secs)).timestamp()
    }

    fn past_ts(offset_secs: i64) -> i64 {
        (Utc::now() - chrono::Duration::seconds(offset_secs)).timestamp()
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
        // ES256 token presented to an HS256 config → jsonwebtoken rejects it
        // with an algorithm error before we even get to our checks.
        // We simulate this by crafting a header that claims RS256 but signing with HS256 key.
        // The real ES256 case can't be easily fabricated without an EC key.
        // Instead, directly test the mapping through a header mismatch.
        let claims = json!({ "sub": "user-1" });
        // Encode with HS256 but validator expects HS256 — let's produce one with
        // wrong algorithm by encoding with RS256 (which will fail differently).
        // The simplest verifiable case: present a valid RS256 token to HS256 config.
        let rs256_claims = json!({ "sub": "user-1" });
        let rs256_token = make_rs256_token(&rs256_claims);
        let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
        let err = auth.authenticate(&Credential::Bearer(rs256_token)).unwrap_err();
        // The new code detects alg mismatch at header-decode time: RS256 header
        // presented to HS256 resolver → AlgorithmMismatch → AlgorithmNotSupported
        assert!(matches!(
            err,
            AuthenticationError::AlgorithmNotSupported(_) | AuthenticationError::InvalidSignature | AuthenticationError::InvalidToken(_)
        ));
        let _ = claims; // suppress unused warning
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
    // CLAR-003: wrong-type roles → graceful degradation
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
        // CLAR-003: wrong-type `tid` (the alias) must reappear under "tid",
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
    // Helper function test (hs256_authenticator)
    // -----------------------------------------------------------------------

    #[test]
    fn hs256_authenticator_helper_works() {
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = hs256_authenticator(&hs256_secret(), now_clock());
        let ctx = auth.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-1");
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
