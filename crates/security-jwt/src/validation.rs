//! Internal JWT claim and time validation engine — shared by all algorithm providers.
//!
//! This module is `pub(crate)` only. It MUST NOT be re-exported from `lib.rs`.
//! See AD-019.

use std::collections::BTreeMap;

use ego_domain::auth::{AuthenticationError, Clock};
use ego_security_sdk::{PrincipalMapper, SecurityContext};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::Value;
use tracing::warn;

use crate::principal_mapper::claims_map_to_claim_set;
#[cfg(test)]
use crate::principal_mapper::DefaultPrincipalMapper;

// ---------------------------------------------------------------------------
// ValidationParams
// ---------------------------------------------------------------------------

/// Caller-supplied validation parameters forwarded to the engine.
pub(crate) struct ValidationParams<'a> {
    /// If `Some`, the token's `iss` claim MUST equal this value.
    pub expected_iss: Option<&'a str>,
    /// If `Some`, the token's `aud` claim MUST contain at least one of these values.
    pub expected_aud: Option<&'a [String]>,
    /// Leeway in seconds applied to `exp` and `nbf` checks.
    ///
    /// Tokens expired by fewer than this many seconds are still accepted (effective validity
    /// window extends past `exp` by this amount). Use small values (≤ 30s) to avoid weakening
    /// revocation. This is NOT symmetric clock-skew tolerance — only `exp` and `nbf` are affected.
    pub leeway_seconds: Option<u64>,
}

// ---------------------------------------------------------------------------
// JwtValidationEngine
// ---------------------------------------------------------------------------

/// Internal engine that centralises all JWT claim / time validation.
///
/// Each provider builds a [`DecodingKey`] + `Algorithm` from the resolved
/// [`crate::VerificationKey`] and then delegates here. The engine owns:
/// - Signature verification (via `jsonwebtoken::decode`)
/// - `exp` / `nbf` clock-injected checks (CLAR-005)
/// - `iss` / `aud` optional checks
/// - `sub` strict extraction
/// - `roles` / `tenant_id` / `tid` graceful extraction
/// - [`SecurityContext`] construction
///
/// AD-019: this struct and the whole module are `pub(crate)`. Never re-export.
pub(crate) struct JwtValidationEngine;

impl JwtValidationEngine {
    /// Validate `token` and return a [`SecurityContext`] on success.
    ///
    /// Uses `DefaultPrincipalMapper`. Existing callers are unchanged.
    #[cfg(test)]
    pub(crate) fn validate(
        token: &str,
        key: &DecodingKey,
        alg: Algorithm,
        params: ValidationParams<'_>,
        clock: &dyn Clock,
    ) -> Result<SecurityContext, AuthenticationError> {
        Self::validate_with_mapper(token, key, alg, params, clock, &DefaultPrincipalMapper)
    }

    /// Validate `token` with an injected `PrincipalMapper`.
    ///
    /// The engine owns: signature verification, `exp`/`nbf`/`iss`/`aud` checks,
    /// and `ClaimSet` assembly. The `(Principal, Claims)` pair comes from `mapper`.
    pub(crate) fn validate_with_mapper(
        token: &str,
        key: &DecodingKey,
        alg: Algorithm,
        params: ValidationParams<'_>,
        clock: &dyn Clock,
        mapper: &dyn PrincipalMapper,
    ) -> Result<SecurityContext, AuthenticationError> {
        // Disable jsonwebtoken's built-in exp/nbf/aud/iss so we do it ourselves
        // (we need clock injection for time checks and custom aud matching).
        let mut validation = Validation::new(alg);
        validation.validate_exp = false;
        validation.validate_nbf = false;
        validation.set_required_spec_claims::<&str>(&[]);
        validation.validate_aud = false;

        let token_data =
            jsonwebtoken::decode::<RawClaims>(token, key, &validation).map_err(|e| {
                use jsonwebtoken::errors::ErrorKind;
                match e.kind() {
                    ErrorKind::InvalidSignature => {
                        warn!(error = "invalid_signature", "JWT validation failed");
                        AuthenticationError::InvalidSignature
                    }
                    ErrorKind::InvalidAlgorithm | ErrorKind::InvalidAlgorithmName => {
                        warn!(error = "algorithm_not_supported", "JWT validation failed");
                        AuthenticationError::AlgorithmNotSupported(format!("{e}"))
                    }
                    _ => {
                        warn!(error = "invalid_token", "JWT validation failed");
                        AuthenticationError::InvalidToken(format!("{e}"))
                    }
                }
            })?;

        let all_claims = token_data.claims.all;
        let now = clock.now();

        // exp check — exp is required; reject missing or invalid exp
        let exp_val = all_claims.get("exp").ok_or_else(|| {
            warn!(error = "invalid_token", "JWT validation failed");
            AuthenticationError::InvalidToken("missing required claim: exp".to_string())
        })?;
        let leeway = chrono::Duration::seconds(
            i64::try_from(params.leeway_seconds.unwrap_or(0)).unwrap_or(0),
        );
        match parse_timestamp(exp_val) {
            Some(exp_dt) => {
                if now >= exp_dt + leeway {
                    warn!(error = "expired_token", "JWT validation failed");
                    return Err(AuthenticationError::ExpiredToken);
                }
            }
            None => {
                warn!(error = "invalid_token", "JWT validation failed");
                return Err(AuthenticationError::InvalidToken(
                    "exp claim is not a valid integer".into(),
                ));
            }
        }

        // nbf check — reject if nbf > now + leeway (same tolerance as exp)
        if let Some(nbf_val) = all_claims.get("nbf") {
            match parse_timestamp(nbf_val) {
                Some(nbf_dt) => {
                    if now + leeway < nbf_dt {
                        warn!(error = "invalid_token", "JWT validation failed");
                        return Err(AuthenticationError::InvalidToken(
                            "token not yet valid".into(),
                        ));
                    }
                }
                None => {
                    warn!(error = "invalid_token", "JWT validation failed");
                    return Err(AuthenticationError::InvalidToken(
                        "nbf claim is not a valid integer".into(),
                    ));
                }
            }
        }

        // iss check
        if let Some(expected_iss) = params.expected_iss {
            match all_claims.get("iss") {
                Some(Value::String(iss)) if iss == expected_iss => {}
                Some(_) => {
                    warn!(error = "invalid_token", "JWT validation failed");
                    return Err(AuthenticationError::InvalidToken(
                        "issuer mismatch".into(),
                    ));
                }
                None => {
                    warn!(error = "invalid_token", "JWT validation failed");
                    return Err(AuthenticationError::InvalidToken(
                        "issuer mismatch".into(),
                    ));
                }
            }
        } else if all_claims.contains_key("iss") {
            // B-1: expected_iss not configured — iss claim present but not validated
            warn!("expected_iss not configured — iss claim is not validated for this provider");
        }

        // aud check — at least one expected aud must be present
        if let Some(expected_auds) = params.expected_aud {
            let token_auds: Vec<String> = match all_claims.get("aud") {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                None => vec![],
                Some(_) => {
                    warn!(error = "invalid_token", "JWT validation failed");
                    return Err(AuthenticationError::InvalidToken(
                        "aud claim is not a string or array".into(),
                    ));
                }
            };
            let any_match = expected_auds.iter().any(|ea| token_auds.contains(ea));
            if !any_match {
                warn!(error = "invalid_token", "JWT validation failed");
                return Err(AuthenticationError::InvalidToken(
                    "aud mismatch".into(),
                ));
            }
        } else if all_claims.contains_key("aud") {
            // B-2: expected_aud not configured — aud claim present but not validated
            warn!("expected_aud not configured — aud claim is not validated for this provider");
        }

        // Convert raw claims to ClaimSet and delegate to mapper (INV-3).
        let claim_set = claims_map_to_claim_set(all_claims);
        let (principal, claims) = mapper.map(&claim_set).map_err(|e| {
            warn!(error = "principal_mapping_failed", "JWT validation failed");
            e
        })?;

        Ok(SecurityContext::new(principal, claims))
    }
}

// ---------------------------------------------------------------------------
// Timestamp parsing helper
// ---------------------------------------------------------------------------

/// Parse a JSON value as a Unix timestamp, returning `None` if the value is
/// not representable as a valid `i64` second count.
///
/// Accepts both `i64` and `u64`-shaped JSON numbers. Rejects floats, strings,
/// booleans, and out-of-range `u64` values.
fn parse_timestamp(val: &serde_json::Value) -> Option<chrono::DateTime<chrono::Utc>> {
    val.as_i64()
        .or_else(|| val.as_u64().and_then(|u| i64::try_from(u).ok()))
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
}

// ---------------------------------------------------------------------------
// Internal raw-claims structure for serde deserialization
// ---------------------------------------------------------------------------

/// Raw deserialized JWT payload. All fields optional; captured via flatten map.
#[derive(serde::Deserialize)]
struct RawClaims {
    #[serde(flatten)]
    all: BTreeMap<String, Value>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use ego_domain::context::TenantId;
    use serde_json::json;

    use crate::test_helpers::{fixed_clock, future_ts, hs256_secret, make_hs256_token, now_clock, past_ts};

    fn hs256_key() -> DecodingKey {
        DecodingKey::from_secret(&hs256_secret())
    }

    fn no_params<'a>() -> ValidationParams<'a> {
        ValidationParams { expected_iss: None, expected_aud: None, leeway_seconds: None }
    }

    // -----------------------------------------------------------------------
    // Happy path
    // -----------------------------------------------------------------------

    #[test]
    fn engine_returns_security_context_for_valid_hs256_token() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let key = hs256_key();
        let clock = now_clock();
        let ctx = JwtValidationEngine::validate(
            &token,
            &key,
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            clock.as_ref(),
        )
        .unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "u1");
    }

    // -----------------------------------------------------------------------
    // FR-024: exp boundary (exp == now → ExpiredToken)
    // -----------------------------------------------------------------------

    #[test]
    fn exp_equal_to_now_returns_expired_token() {
        let now = Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let exp_secs = now.timestamp();
        let claims = json!({ "sub": "u1", "exp": exp_secs });
        let token = make_hs256_token(&claims);
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            fixed_clock(now).as_ref(),
        )
        .unwrap_err();
        assert_eq!(err, AuthenticationError::ExpiredToken);
    }

    // -----------------------------------------------------------------------
    // FR-024: nbf future (nbf > now → InvalidToken)
    // -----------------------------------------------------------------------

    #[test]
    fn nbf_future_returns_invalid_token() {
        let nbf_secs = future_ts(300);
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // FR-024: non-integer exp → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn exp_as_string_returns_invalid_token() {
        let claims = json!({ "sub": "u1", "exp": "never" });
        let token = make_hs256_token(&claims);
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // BL-03: missing exp → InvalidToken (Closes #68)
    // -----------------------------------------------------------------------

    #[test]
    fn missing_exp_claim_is_rejected() {
        // Token has sub and nbf but no exp — must be rejected
        let claims = json!({ "sub": "u1", "nbf": past_ts(300) });
        let token = make_hs256_token(&claims);
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert!(
            matches!(&err, AuthenticationError::InvalidToken(msg) if msg.contains("missing required claim: exp")),
            "expected InvalidToken with missing exp message, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // FR-024: missing sub → MissingClaim("sub")
    // -----------------------------------------------------------------------

    #[test]
    fn missing_sub_returns_missing_claim() {
        let claims = json!({ "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert!(
            matches!(&err, AuthenticationError::MissingClaim(s) if s == "sub"),
            "expected MissingClaim(\"sub\"), got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // FR-024: non-string sub → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn sub_integer_returns_invalid_token() {
        let claims = json!({ "sub": 42, "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // FR-024: empty string sub → InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn sub_empty_string_returns_invalid_token() {
        let claims = json!({ "sub": "", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // FR-024: wrong-type roles → graceful skip, raw preserved
    // -----------------------------------------------------------------------

    #[test]
    fn wrong_type_roles_graceful_skip_raw_preserved() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "roles": "admin" });
        let token = make_hs256_token(&claims);
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap();
        assert!(ctx.principal.roles.is_empty());
        assert_eq!(ctx.claims.custom.get("roles"), Some(&json!("admin")));
    }

    // -----------------------------------------------------------------------
    // FR-024: wrong-type tid → graceful skip, preserved under original key
    // -----------------------------------------------------------------------

    #[test]
    fn wrong_type_tid_graceful_skip_preserved_under_original_key() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "tid": 42 });
        let token = make_hs256_token(&claims);
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap();
        assert_eq!(ctx.principal.tenant_id, None);
        assert_eq!(ctx.claims.custom.get("tid"), Some(&json!(42)));
        assert!(!ctx.claims.custom.contains_key("tenant_id"));
    }

    #[test]
    fn tenant_id_takes_precedence_over_tid_when_both_present() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "tenant_id": "primary", "tid": "secondary" });
        let token = make_hs256_token(&claims);
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap();
        assert_eq!(
            ctx.principal.tenant_id.as_ref().map(TenantId::as_str),
            Some("primary")
        );
        // tid must remain in custom since it was not consumed
        assert_eq!(ctx.claims.custom.get("tid"), Some(&json!("secondary")));
        assert!(!ctx.claims.custom.contains_key("tenant_id"));
    }

    // -----------------------------------------------------------------------
    // FR-024: past exp → accepted
    // -----------------------------------------------------------------------

    #[test]
    fn past_nbf_is_accepted() {
        let nbf_secs = past_ts(300);
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "u1");
    }

    // -----------------------------------------------------------------------
    // iss validation
    // -----------------------------------------------------------------------

    #[test]
    fn iss_mismatch_returns_invalid_token() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "iss": "wrong" });
        let token = make_hs256_token(&claims);
        let params = ValidationParams { expected_iss: Some("expected-iss"), expected_aud: None, leeway_seconds: None };
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn iss_absent_with_expected_returns_invalid_token() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let params =
            ValidationParams { expected_iss: Some("expected-iss"), expected_aud: None, leeway_seconds: None };
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // aud validation
    // -----------------------------------------------------------------------

    #[test]
    fn aud_mismatch_returns_invalid_token() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "aud": ["other-api"] });
        let token = make_hs256_token(&claims);
        let expected_aud = vec!["my-api".to_string()];
        let params =
            ValidationParams { expected_iss: None, expected_aud: Some(&expected_aud), leeway_seconds: None };
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    // -----------------------------------------------------------------------
    // W-4: nbf == now → accepted (boundary: not yet valid means strictly after now)
    // -----------------------------------------------------------------------

    #[test]
    fn nbf_equal_to_now_is_valid() {
        let now = Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let nbf_secs = now.timestamp();
        // exp must be after `now` (use +1h)
        let exp_secs = (now + chrono::Duration::hours(1)).timestamp();
        let claims = json!({ "sub": "u1", "exp": exp_secs, "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        // nbf == now: the check is `now < nbf_dt`, so equal → accepted
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            fixed_clock(now).as_ref(),
        )
        .unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "u1");
    }

    // -----------------------------------------------------------------------
    // W-4: aud scalar string is accepted
    // -----------------------------------------------------------------------

    #[test]
    fn aud_scalar_string_is_accepted() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "aud": "my-api" });
        let token = make_hs256_token(&claims);
        let expected_aud = vec!["my-api".to_string()];
        let params = ValidationParams { expected_iss: None, expected_aud: Some(&expected_aud), leeway_seconds: None };
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            now_clock().as_ref(),
        )
        .unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "u1");
    }

    // -----------------------------------------------------------------------
    // W-4: aud wrong type (numeric) returns InvalidToken
    // -----------------------------------------------------------------------

    #[test]
    fn aud_wrong_type_returns_invalid_token() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "aud": 42 });
        let token = make_hs256_token(&claims);
        let expected_aud = vec!["my-api".to_string()];
        let params = ValidationParams { expected_iss: None, expected_aud: Some(&expected_aud), leeway_seconds: None };
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "expected InvalidToken for numeric aud, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // W-4: roles null → empty set, raw claim preserved
    // -----------------------------------------------------------------------

    #[test]
    fn roles_null_produces_empty_set_raw_preserved() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "roles": null });
        let token = make_hs256_token(&claims);
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap();
        assert!(ctx.principal.roles.is_empty(), "roles should be empty for null value");
        assert_eq!(
            ctx.claims.custom.get("roles"),
            Some(&serde_json::Value::Null),
            "null roles should be preserved in custom claims"
        );
    }

    // -----------------------------------------------------------------------
    // B-1: expected_iss None — token with iss accepted (permissive, observable)
    // -----------------------------------------------------------------------

    #[test]
    fn expected_iss_none_does_not_validate_iss() {
        // Token carries a random issuer; expected_iss is None.
        // The token MUST be accepted — the warn! path is exercised, not rejection.
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "iss": "https://random.issuer.example" });
        let token = make_hs256_token(&claims);
        let params = ValidationParams { expected_iss: None, expected_aud: None, leeway_seconds: None };
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            now_clock().as_ref(),
        )
        .unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "u1");
    }

    // -----------------------------------------------------------------------
    // B-2: expected_aud None — token with aud accepted (permissive, observable)
    // -----------------------------------------------------------------------

    #[test]
    fn expected_aud_none_does_not_validate_aud() {
        // Token carries an audience; expected_aud is None.
        // The token MUST be accepted — the warn! path is exercised, not rejection.
        let claims = json!({ "sub": "u1", "exp": future_ts(3600), "aud": "some-unvalidated-service" });
        let token = make_hs256_token(&claims);
        let params = ValidationParams { expected_iss: None, expected_aud: None, leeway_seconds: None };
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            now_clock().as_ref(),
        )
        .unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "u1");
    }

    // -----------------------------------------------------------------------
    // leeway_seconds: boundary — expired by exactly leeway → rejected
    // -----------------------------------------------------------------------

    #[test]
    fn expired_by_exact_leeway_is_rejected() {
        // exp = now - leeway → exp_dt + leeway == now → now >= exp_dt + leeway → true → ExpiredToken
        let now = chrono::Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let leeway_secs: i64 = 5;
        let exp_secs = (now - chrono::Duration::seconds(leeway_secs)).timestamp();
        let claims = json!({ "sub": "u1", "exp": exp_secs });
        let token = make_hs256_token(&claims);
        let params = ValidationParams {
            expected_iss: None,
            expected_aud: None,
            leeway_seconds: Some(leeway_secs as u64),
        };
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            fixed_clock(now).as_ref(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            AuthenticationError::ExpiredToken,
            "token expired by exactly leeway seconds must be rejected"
        );
    }

    #[test]
    fn expired_by_leeway_minus_one_is_accepted() {
        // exp = now - (leeway - 1) → exp_dt + leeway == now + 1 > now → accepted
        let now = chrono::Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let leeway_secs: i64 = 5;
        let exp_secs = (now - chrono::Duration::seconds(leeway_secs - 1)).timestamp();
        let claims = json!({ "sub": "u1", "exp": exp_secs });
        let token = make_hs256_token(&claims);
        let params = ValidationParams {
            expected_iss: None,
            expected_aud: None,
            leeway_seconds: Some(leeway_secs as u64),
        };
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            fixed_clock(now).as_ref(),
        )
        .unwrap();
        assert_eq!(
            ctx.principal.subject_id.as_str(),
            "u1",
            "token expired by leeway-1 seconds must be accepted"
        );
    }

    // -----------------------------------------------------------------------
    // B-3: nbf leeway — leeway_seconds applies to nbf as well
    // -----------------------------------------------------------------------

    #[test]
    fn token_not_yet_valid_by_less_than_leeway_is_accepted() {
        // nbf = now + 3s, leeway = 5s → now + 5 >= now + 3 → accepted
        let now = chrono::Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let nbf_secs = (now + chrono::Duration::seconds(3)).timestamp();
        let exp_secs = (now + chrono::Duration::hours(1)).timestamp();
        let claims = json!({ "sub": "u1", "exp": exp_secs, "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        let params = ValidationParams {
            expected_iss: None,
            expected_aud: None,
            leeway_seconds: Some(5),
        };
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            fixed_clock(now).as_ref(),
        )
        .unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "u1");
    }

    #[test]
    fn token_not_yet_valid_by_more_than_leeway_is_rejected() {
        // nbf = now + 10s, leeway = 5s → now + 5 < now + 10 → rejected
        let now = chrono::Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let nbf_secs = (now + chrono::Duration::seconds(10)).timestamp();
        let exp_secs = (now + chrono::Duration::hours(1)).timestamp();
        let claims = json!({ "sub": "u1", "exp": exp_secs, "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        let params = ValidationParams {
            expected_iss: None,
            expected_aud: None,
            leeway_seconds: Some(5),
        };
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            fixed_clock(now).as_ref(),
        )
        .unwrap_err();
        assert!(matches!(&err, AuthenticationError::InvalidToken(msg) if msg.contains("token not yet valid")));
    }

    // -----------------------------------------------------------------------
    // Invalid signature
    // -----------------------------------------------------------------------

    #[test]
    fn wrong_key_returns_invalid_signature() {
        let claims = json!({ "sub": "u1", "exp": future_ts(3600) });
        let token = make_hs256_token(&claims);
        let wrong_key = DecodingKey::from_secret(b"wrong-key");
        let err = JwtValidationEngine::validate(
            &token,
            &wrong_key,
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            now_clock().as_ref(),
        )
        .unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    // -----------------------------------------------------------------------
    // leeway_seconds: token expired by 1s accepted with 5s leeway
    // -----------------------------------------------------------------------

    #[test]
    fn expired_by_one_second_accepted_within_clock_skew() {
        // exp is exactly 1 second before "now" — without leeway this would be rejected.
        let now = chrono::Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let exp_secs = (now - chrono::Duration::seconds(1)).timestamp();
        let claims = json!({ "sub": "u1", "exp": exp_secs });
        let token = make_hs256_token(&claims);
        // 5 seconds of leeway: exp+5 > now, so token is still valid
        let params = ValidationParams {
            expected_iss: None,
            expected_aud: None,
            leeway_seconds: Some(5),
        };
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            fixed_clock(now).as_ref(),
        )
        .unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "u1");
    }

    #[test]
    fn nbf_equal_to_now_plus_leeway_boundary_is_accepted() {
        // nbf = now + leeway → now + leeway >= now + leeway → accepted (inclusive boundary)
        let now = chrono::Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let leeway_secs: i64 = 5;
        let nbf_secs = (now + chrono::Duration::seconds(leeway_secs)).timestamp();
        let exp_secs = (now + chrono::Duration::hours(1)).timestamp();
        let claims = json!({ "sub": "u1", "exp": exp_secs, "nbf": nbf_secs });
        let token = make_hs256_token(&claims);
        let params = ValidationParams {
            expected_iss: None,
            expected_aud: None,
            leeway_seconds: Some(leeway_secs as u64),
        };
        let ctx = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            params,
            fixed_clock(now).as_ref(),
        )
        .unwrap();
        assert_eq!(
            ctx.principal.subject_id.as_str(),
            "u1",
            "nbf == now + leeway is the inclusive boundary and must be accepted"
        );
    }

    #[test]
    fn exp_equal_to_now_no_skew_is_rejected() {
        // exp == now with no clock skew → expired
        let now = chrono::Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap();
        let exp_secs = now.timestamp();
        let claims = json!({ "sub": "u1", "exp": exp_secs });
        let token = make_hs256_token(&claims);
        let err = JwtValidationEngine::validate(
            &token,
            &hs256_key(),
            jsonwebtoken::Algorithm::HS256,
            no_params(),
            fixed_clock(now).as_ref(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            AuthenticationError::ExpiredToken,
            "exp == now with no skew must be rejected"
        );
    }
}
