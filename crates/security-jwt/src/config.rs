//! JWT configuration types — algorithm selection and validation parameters.

/// The signing algorithm used to verify JWTs.
///
/// This is a pure marker enum — key material has been moved to
/// [`crate::VerificationKey`] inside a [`crate::KeyResolver`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize)]
pub enum JwtAlgorithm {
    /// HMAC-SHA256.
    Hs256,

    /// RSA-PKCS1-SHA256. Only the public key is needed for verification.
    Rs256,

    /// ECDSA-P256-SHA256. Only the public key is needed for verification.
    Es256,
}

/// Shared validation configuration for the single-algorithm providers.
///
/// Holds an optional issuer constraint and a REQUIRED audience constraint. Key
/// material lives in the injected [`crate::KeyResolver`] — not here. The algorithm
/// is encoded at the type level by each provider, so no `algorithm` field is needed.
///
/// # `expected_aud` is required, `expected_iss` is not
///
/// [`ego_domain::Validate::validate`] rejects a config whose `expected_aud` is
/// `None` or empty: an audience constraint is the primary defense against
/// audience-confusion / token-reuse (accepting a token minted for another service).
///
/// `expected_iss` remains optional. The OIDC provider only requires the issuer
/// conditionally (when `jwks_uri` is set) and explicitly permits a discovery path
/// with `iss` absent (advisory warn, not a hard error — see `oidc_config.rs`).
/// The single-algorithm config has no analogous conditional trigger, so
/// unconditionally requiring `iss` here would be stricter than the codebase's own
/// OIDC baseline and would break legitimately issuer-agnostic deployments.
/// Pinning `iss` is tracked as a follow-up; `validate()` still rejects an empty
/// `expected_iss` when one is provided.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, Default)]
pub struct JwtProviderConfig {
    /// If `Some`, the token's `iss` claim MUST equal this value. Optional (see type docs).
    pub expected_iss: Option<String>,
    /// REQUIRED: the token's `aud` claim MUST contain at least one of these values.
    /// A config with `expected_aud = None` (or an empty list) is rejected by
    /// [`ego_domain::Validate::validate`].
    pub expected_aud: Option<Vec<String>>,
    /// Leeway in seconds applied to `exp` and `nbf` checks.
    ///
    /// Tokens expired by fewer than this many seconds are still accepted (effective validity
    /// window extends past `exp` by this amount). Recommended: small values (≤ 30s) to avoid
    /// weakening revocation — this is guidance, not an enforced limit. The hard-enforced ceiling
    /// is [`MAX_LEEWAY_SECONDS`] (300s); values above it are rejected by [`Validate`]. This is
    /// NOT symmetric clock-skew tolerance — only `exp` and `nbf` are affected.
    pub leeway_seconds: Option<u64>,
}

/// Leeway above this bound is rejected by [`Validate`] — avoids weakening
/// revocation via an overly generous `exp`/`nbf` grace window.
const MAX_LEEWAY_SECONDS: u64 = 300;

impl ego_domain::Validate for JwtProviderConfig {
    fn validate(&self) -> Result<(), ego_domain::ConfigError> {
        if let Some(leeway) = self.leeway_seconds {
            if leeway > MAX_LEEWAY_SECONDS {
                return Err(ego_domain::ConfigError::Invalid {
                    field: "leeway_seconds".to_string(),
                    reason: format!("must not exceed {MAX_LEEWAY_SECONDS} seconds"),
                });
            }
        }
        if let Some(iss) = &self.expected_iss {
            if iss.is_empty() {
                return Err(ego_domain::ConfigError::not_empty("expected_iss"));
            }
        }
        // Security: expected_aud is REQUIRED. Without an audience constraint, any
        // signature-valid token is accepted regardless of its `aud` claim, enabling
        // audience-confusion and token-reuse attacks (a token minted for another service
        // is accepted here). This mirrors the OIDC provider, which already refuses to run
        // without issuer/audience binding (see oidc_config.rs). `expected_iss` remains
        // optional for now — see the module note below.
        match &self.expected_aud {
            None => {
                return Err(ego_domain::ConfigError::Invalid {
                    field: "expected_aud".to_string(),
                    reason: "is required — without it, a signature-valid token for any \
                             audience is accepted (audience-confusion / token-reuse risk)"
                        .to_string(),
                });
            }
            Some(aud) => {
                if aud.is_empty() {
                    return Err(ego_domain::ConfigError::Invalid {
                        field: "expected_aud".to_string(),
                        reason: "must not be empty when set".to_string(),
                    });
                }
                if aud.iter().any(|s| s.is_empty()) {
                    return Err(ego_domain::ConfigError::Invalid {
                        field: "expected_aud".to_string(),
                        reason: "must not contain empty string entries".to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// Type alias for clarity at call sites using [`crate::Hs256AuthenticationProvider`].
pub type Hs256Config = JwtProviderConfig;
/// Type alias for clarity at call sites using [`crate::Rs256AuthenticationProvider`].
pub type Rs256Config = JwtProviderConfig;
/// Type alias for clarity at call sites using [`crate::Es256AuthenticationProvider`].
pub type Es256Config = JwtProviderConfig;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod jwt_provider_config_validate_tests {
    use super::*;
    use ego_domain::{ConfigError, Validate};

    #[test]
    fn default_config_is_invalid_without_expected_aud() {
        // Intended behavior change: the all-`None` default no longer validates, because
        // `expected_aud` is now required (audience-confusion / token-reuse defense).
        assert_eq!(
            JwtProviderConfig::default().validate(),
            Err(ConfigError::Invalid {
                field: "expected_aud".to_string(),
                reason: "is required — without it, a signature-valid token for any \
                         audience is accepted (audience-confusion / token-reuse risk)"
                    .to_string(),
            })
        );
    }

    #[test]
    fn leeway_within_bound_is_valid() {
        let config = JwtProviderConfig {
            leeway_seconds: Some(30),
            expected_aud: Some(vec!["api".to_string()]),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn leeway_above_bound_is_invalid() {
        let config = JwtProviderConfig {
            leeway_seconds: Some(301),
            ..Default::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "leeway_seconds".to_string(),
                reason: format!("must not exceed {MAX_LEEWAY_SECONDS} seconds"),
            })
        );
    }

    #[test]
    fn none_expected_aud_is_invalid() {
        // Security: expected_aud is REQUIRED. Without it, any signature-valid token is
        // accepted regardless of audience (audience-confusion / token-reuse risk).
        let config = JwtProviderConfig {
            expected_aud: None,
            ..Default::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "expected_aud".to_string(),
                reason: "is required — without it, a signature-valid token for any \
                         audience is accepted (audience-confusion / token-reuse risk)"
                    .to_string(),
            })
        );
    }

    #[test]
    fn empty_expected_aud_is_invalid() {
        let config = JwtProviderConfig {
            expected_aud: Some(vec![]),
            ..Default::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "expected_aud".to_string(),
                reason: "must not be empty when set".to_string(),
            })
        );
    }

    #[test]
    fn expected_aud_with_empty_string_entry_is_invalid() {
        let config = JwtProviderConfig {
            expected_aud: Some(vec!["".to_string()]),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn non_empty_expected_aud_is_valid() {
        let config = JwtProviderConfig {
            expected_aud: Some(vec!["api".to_string()]),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn empty_expected_iss_is_invalid() {
        let config = JwtProviderConfig {
            expected_iss: Some(String::new()),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
