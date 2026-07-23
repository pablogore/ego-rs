//! Configuration validation contract (CORE-016).
//!
//! `ego-domain` owns the depend-free `Validate` contract so every
//! infrastructure crate's configuration domain (`RuntimeConfig`,
//! `JwtProviderConfig`, `EventBusConfig`, ...) can expose domain-invariant
//! validation without any crate depending on `kit-config`.
//!
//! kit-config remains responsible for structural validation, loading, and
//! materialization (KIT-001). This trait only carries the "does this
//! subtree satisfy its own invariants" contract.

use thiserror::Error;

/// Domain-invariant validation contract for a configuration subtree.
///
/// Implementors check their own field invariants only (e.g. non-zero
/// capacities, bounded values). Structural validation (types, required
/// fields, parsing) is owned by kit-config (KIT-001); cross-domain rules
/// are owned by the application's root configuration type.
pub trait Validate {
    /// Validate this configuration subtree's invariants.
    ///
    /// Returns `Ok(())` when all invariants hold, or `Err(ConfigError)`
    /// describing the first violated invariant.
    fn validate(&self) -> Result<(), ConfigError>;
}

/// Error returned when a configuration subtree fails validation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    /// A single field violated its invariant.
    #[error("invalid config field `{field}`: {reason}")]
    Invalid {
        /// The name of the offending field.
        field: String,
        /// A human-readable description of the violated invariant.
        reason: String,
    },
}

impl ConfigError {
    /// Builds an [`Invalid`](ConfigError::Invalid) error for a field that must be non-zero.
    pub fn non_zero(field: &str) -> Self {
        Self::Invalid {
            field: field.to_string(),
            reason: "must be non-zero".to_string(),
        }
    }

    /// Builds an [`Invalid`](ConfigError::Invalid) error for a field that must not be empty.
    pub fn not_empty(field: &str) -> Self {
        Self::Invalid {
            field: field.to_string(),
            reason: "must not be empty".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixtureConfig {
        capacity: u32,
    }

    impl Validate for FixtureConfig {
        fn validate(&self) -> Result<(), ConfigError> {
            if self.capacity == 0 {
                return Err(ConfigError::Invalid {
                    field: "capacity".to_string(),
                    reason: "must be non-zero".to_string(),
                });
            }
            Ok(())
        }
    }

    #[test]
    fn valid_config_passes() {
        let config = FixtureConfig { capacity: 1 };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn invalid_config_fails() {
        let config = FixtureConfig { capacity: 0 };
        let err = config.validate().unwrap_err();
        assert_eq!(
            err,
            ConfigError::Invalid {
                field: "capacity".to_string(),
                reason: "must be non-zero".to_string(),
            }
        );
    }

    #[test]
    fn config_error_implements_std_error() {
        fn assert_error<E: std::error::Error>() {}
        assert_error::<ConfigError>();
    }

    #[test]
    fn config_error_display() {
        let err = ConfigError::Invalid {
            field: "capacity".to_string(),
            reason: "must be non-zero".to_string(),
        };
        assert_eq!(
            format!("{}", err),
            "invalid config field `capacity`: must be non-zero"
        );
    }

    #[test]
    fn non_zero_helper_builds_invalid_error() {
        assert_eq!(
            ConfigError::non_zero("capacity"),
            ConfigError::Invalid {
                field: "capacity".to_string(),
                reason: "must be non-zero".to_string(),
            }
        );
    }

    #[test]
    fn not_empty_helper_builds_invalid_error() {
        assert_eq!(
            ConfigError::not_empty("url"),
            ConfigError::Invalid {
                field: "url".to_string(),
                reason: "must not be empty".to_string(),
            }
        );
    }
}
