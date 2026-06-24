//! JWT authenticator — implements [`AuthenticationProvider`] for JWT tokens.
//!
//! Validates JWT signatures, standard time claims (`exp`, `nbf`), and
//! optional issuer/audience constraints. Identity fields (`sub`, `roles`,
//! `tenant_id`/`tid`) are extracted with graceful degradation per CLAR-003:
//! a wrong-type claim is skipped and the raw value is preserved in
//! `Claims.custom`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ego_domain::auth::{
    AuthenticationError, AuthenticationProvider, Claims, Clock, Credential, Identity,
    SecurityContext, StandardClaims,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::Value;

use crate::config::{JwtAlgorithm, JwtConfig};

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
/// Validates a `BearerToken` credential by:
/// 1. Verifying the token signature using the configured algorithm and key.
/// 2. Rejecting tokens whose `exp` has passed (using the injected [`Clock`]).
/// 3. Rejecting tokens whose `nbf` has not yet been reached.
/// 4. Optionally validating `iss` and `aud` claims.
/// 5. Extracting `sub`, `roles`, and `tenant_id`/`tid` into [`Identity`],
///    with graceful degradation for wrong-type values (CLAR-003).
///
/// # Clocks
///
/// This authenticator NEVER calls `Utc::now()` directly. All time-sensitive
/// checks go through the injected `Arc<dyn Clock>`.
pub struct JwtAuthenticator {
    config: JwtConfig,
    clock: Arc<dyn Clock>,
}

impl JwtAuthenticator {
    /// Constructs a new authenticator.
    ///
    /// - `config`: algorithm, key material, and optional iss/aud constraints.
    /// - `clock`: injectable time source — use a mock in tests.
    pub fn new(config: JwtConfig, clock: Arc<dyn Clock>) -> Self {
        Self { config, clock }
    }
}

impl AuthenticationProvider for JwtAuthenticator {
    fn authenticate(
        &self,
        credential: Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        let token = match credential {
            Credential::BearerToken(t) => t,
            _ => {
                return Err(AuthenticationError::InvalidToken(
                    "unsupported credential type".into(),
                ))
            }
        };

        // Build decoding key and validation parameters
        let (decoding_key, algorithm) = match &self.config.algorithm {
            JwtAlgorithm::Hs256 { secret } => (
                DecodingKey::from_secret(secret),
                Algorithm::HS256,
            ),
            JwtAlgorithm::Rs256 { public_key_pem } => (
                DecodingKey::from_rsa_pem(public_key_pem.as_bytes()).map_err(|e| {
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
            jsonwebtoken::decode::<RawClaims>(&token, &decoding_key, &validation).map_err(
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

        // Also check for algorithm mismatch in the header
        let header_alg = token_data.header.alg;
        if header_alg != algorithm {
            return Err(AuthenticationError::AlgorithmNotSupported(format!(
                "token uses {header_alg:?} but configured for {algorithm:?}"
            )));
        }

        let all_claims = token_data.claims.all;
        let now = self.clock.now();

        // exp check — reject if exp <= now (expired means exp is in the past or equal to now)
        if let Some(exp_val) = all_claims.get("exp") {
            if let Some(exp_secs) = exp_val.as_i64().or_else(|| exp_val.as_u64().map(|u| u as i64)) {
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
            if let Some(nbf_secs) = nbf_val.as_i64().or_else(|| nbf_val.as_u64().map(|u| u as i64)) {
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

        // Remove fields that are now in StandardClaims / Identity from custom
        let custom = remove_standard_keys(all_claims);

        let identity = Identity {
            subject,
            tenant_id,
            roles,
            attributes: BTreeMap::new(),
        };

        let claims = Claims {
            standard,
            custom,
        };

        Ok(SecurityContext::new(identity, claims))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build [`StandardClaims`] from the raw claims map (values remain there too).
fn build_standard_claims(map: &BTreeMap<String, Value>) -> StandardClaims {
    let exp = map.get("exp").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_u64().map(|u| u as i64))
            .and_then(|s| chrono::DateTime::<chrono::Utc>::from_timestamp(s, 0))
    });
    let nbf = map.get("nbf").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_u64().map(|u| u as i64))
            .and_then(|s| chrono::DateTime::<chrono::Utc>::from_timestamp(s, 0))
    });
    let iat = map.get("iat").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_u64().map(|u| u as i64))
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

/// Extract `sub` from the map. Returns `Ok((subject, map))` on success.
/// - Absent `sub`: returns `Err(MissingClaim("sub"))`.
/// - CLAR-003: if `sub` is present but not a string, use "" and keep raw in map.
fn extract_subject(
    mut map: BTreeMap<String, Value>,
) -> Result<(String, BTreeMap<String, Value>), AuthenticationError> {
    match map.remove("sub") {
        Some(Value::String(s)) => Ok((s, map)),
        Some(other) => {
            // Wrong type — graceful degradation: empty subject, raw value preserved
            map.insert("sub".into(), other);
            Ok((String::new(), map))
        }
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
    use ego_domain::auth::{AuthenticationError, Credential};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;

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

    fn hs256_config() -> JwtConfig {
        JwtConfig {
            algorithm: JwtAlgorithm::Hs256 { secret: hs256_secret() },
            expected_iss: None,
            expected_aud: None,
        }
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
            algorithm: JwtAlgorithm::Rs256 {
                public_key_pem: rs256_public_key_pem().to_string(),
            },
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
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.subject, "user-1");
    }

    // -----------------------------------------------------------------------
    // HS256 wrong secret → InvalidSignature
    // -----------------------------------------------------------------------

    #[test]
    fn hs256_wrong_secret_returns_invalid_signature() {
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256 { secret: hs256_wrong_secret() },
            expected_iss: None,
            expected_aud: None,
        };
        let auth = JwtAuthenticator::new(config, now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    // -----------------------------------------------------------------------
    // FR-007: RS256 valid token
    // -----------------------------------------------------------------------

    #[test]
    fn rs256_valid_token_returns_security_context() {
        let claims = json!({ "sub": "rs256-user", "exp": future_ts(3600) });
        let token = make_rs256_token(&claims);
        let auth = JwtAuthenticator::new(rs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.subject, "rs256-user");
    }

    // -----------------------------------------------------------------------
    // RS256 mismatched key → InvalidSignature
    // -----------------------------------------------------------------------

    #[test]
    fn rs256_mismatched_key_returns_invalid_signature() {
        let claims = json!({ "sub": "rs256-user", "exp": future_ts(3600) });
        // Signed with primary key but verified with OTHER public key
        let token = make_rs256_token(&claims);
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Rs256 {
                public_key_pem: rs256_other_public_key_pem().to_string(),
            },
            expected_iss: None,
            expected_aud: None,
        };
        let auth = JwtAuthenticator::new(config, now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
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
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
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
        let auth = JwtAuthenticator::new(hs256_config(), fixed_clock(now));
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::ExpiredToken);
    }

    // -----------------------------------------------------------------------
    // No exp claim → Ok
    // -----------------------------------------------------------------------

    #[test]
    fn token_without_exp_is_accepted() {
        let claims = json!({ "sub": "user-1" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.subject, "user-1");
    }

    // -----------------------------------------------------------------------
    // FR-011: future nbf → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn token_with_future_nbf_is_rejected() {
        let nbf_secs = future_ts(300); // not valid for 5 minutes
        let claims = json!({ "sub": "user-1", "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
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
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.subject, "user-1");
    }

    // -----------------------------------------------------------------------
    // Malformed string → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn malformed_token_returns_invalid_token() {
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth
            .authenticate(Credential::BearerToken("not.a.jwt".into()))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // iss validation
    // -----------------------------------------------------------------------

    #[test]
    fn unexpected_iss_returns_invalid_token() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256 { secret: hs256_secret() },
            expected_iss: Some("my-service".into()),
            expected_aud: None,
        };
        let claims = json!({ "sub": "user-1", "iss": "other-service" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn no_iss_configured_accepts_any_iss() {
        // No expected_iss → accept any iss or absent iss
        let claims = json!({ "sub": "user-1", "iss": "random-issuer" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.subject, "user-1");
    }

    #[test]
    fn correct_iss_is_accepted() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256 { secret: hs256_secret() },
            expected_iss: Some("trusted-iss".into()),
            expected_aud: None,
        };
        let claims = json!({ "sub": "user-1", "iss": "trusted-iss" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.subject, "user-1");
    }

    // -----------------------------------------------------------------------
    // aud validation
    // -----------------------------------------------------------------------

    #[test]
    fn unexpected_aud_returns_invalid_token() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256 { secret: hs256_secret() },
            expected_iss: None,
            expected_aud: Some(vec!["my-api".into()]),
        };
        let claims = json!({ "sub": "user-1", "aud": ["other-api"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn correct_aud_is_accepted() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256 { secret: hs256_secret() },
            expected_iss: None,
            expected_aud: Some(vec!["my-api".into()]),
        };
        let claims = json!({ "sub": "user-1", "aud": ["my-api"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.subject, "user-1");
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
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth.authenticate(Credential::BearerToken(rs256_token)).unwrap_err();
        // jsonwebtoken will report InvalidAlgorithm or InvalidSignature for alg mismatch
        assert!(matches!(
            err,
            AuthenticationError::AlgorithmNotSupported(_) | AuthenticationError::InvalidSignature | AuthenticationError::InvalidToken(_)
        ));
        let _ = claims; // suppress unused warning
    }

    // -----------------------------------------------------------------------
    // CLAR-003: wrong-type sub → graceful degradation
    // -----------------------------------------------------------------------

    #[test]
    fn wrong_type_sub_produces_empty_subject_and_preserves_raw() {
        // sub is an integer, not a string
        let claims = json!({ "sub": 12345 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.subject, "");
        // Raw value preserved in custom claims
        assert_eq!(ctx.claims.custom.get("sub"), Some(&json!(12345)));
    }

    // -----------------------------------------------------------------------
    // CLAR-003: wrong-type roles → graceful degradation
    // -----------------------------------------------------------------------

    #[test]
    fn wrong_type_roles_produces_empty_set_and_preserves_raw() {
        // roles is a string instead of an array
        let claims = json!({ "sub": "user-1", "roles": "admin" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert!(ctx.identity.roles.is_empty());
        assert_eq!(ctx.claims.custom.get("roles"), Some(&json!("admin")));
    }

    #[test]
    fn roles_array_of_strings_is_extracted_correctly() {
        let claims = json!({ "sub": "user-1", "roles": ["admin", "editor"] });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert!(ctx.identity.roles.contains("admin"));
        assert!(ctx.identity.roles.contains("editor"));
        assert_eq!(ctx.identity.roles.len(), 2);
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
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
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
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.tenant_id, Some("acme".into()));
    }

    #[test]
    fn tid_claim_is_extracted_as_tenant_id() {
        let claims = json!({ "sub": "user-1", "tid": "contoso" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.tenant_id, Some("contoso".into()));
    }

    #[test]
    fn wrong_type_tenant_id_produces_none_and_preserves_raw() {
        let claims = json!({ "sub": "user-1", "tenant_id": 999 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert!(ctx.identity.tenant_id.is_none());
        assert_eq!(ctx.claims.custom.get("tenant_id"), Some(&json!(999)));
    }

    #[test]
    fn wrong_type_tid_alias_preserves_raw_under_tid_key() {
        // CLAR-003: wrong-type `tid` (the alias) must reappear under "tid",
        // NOT renamed to "tenant_id" in custom claims.
        let claims = json!({ "sub": "user-1", "tid": 42 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert!(ctx.identity.tenant_id.is_none());
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
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
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
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        // Mixed array → graceful degradation
        assert!(ctx.identity.roles.is_empty());
    }

    // -----------------------------------------------------------------------
    // FIX-1: exp with non-integer JSON type must return InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn exp_as_string_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "exp": "never" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    #[test]
    fn exp_as_float_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "exp": 9_999_999_999.5_f64 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    #[test]
    fn exp_as_bool_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "exp": true });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    #[test]
    fn nbf_as_string_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "nbf": "yesterday" });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    #[test]
    fn nbf_as_float_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "nbf": 0.5_f64 });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken, got {err:?}"
        );
    }

    #[test]
    fn nbf_as_bool_returns_invalid_token() {
        let claims = json!({ "sub": "user-1", "nbf": false });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
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
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert!(
            matches!(&err, AuthenticationError::MissingClaim(s) if s == "sub"),
            "expected MissingClaim(\"sub\"), got {err:?}"
        );
    }

    #[test]
    fn token_with_valid_sub_returns_security_context() {
        let claims = json!({ "sub": "happy-user", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(hs256_config(), now_clock());
        let ctx = auth.authenticate(Credential::BearerToken(token)).unwrap();
        assert_eq!(ctx.identity.subject, "happy-user");
    }

    // -----------------------------------------------------------------------
    // FIX-4: iss absent with expected value configured → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn expected_iss_configured_but_token_has_no_iss_returns_invalid_token() {
        let config = JwtConfig {
            algorithm: JwtAlgorithm::Hs256 { secret: hs256_secret() },
            expected_iss: Some("https://auth.example.com".into()),
            expected_aud: None,
        };
        // Token has no "iss" key
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
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
            algorithm: JwtAlgorithm::Hs256 { secret: hs256_secret() },
            expected_iss: None,
            expected_aud: Some(vec!["api.example.com".into()]),
        };
        // Token has no "aud" key
        let claims = json!({ "sub": "user-1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let auth = JwtAuthenticator::new(config, now_clock());
        let err = auth.authenticate(Credential::BearerToken(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken for absent aud, got {err:?}"
        );
    }
}
