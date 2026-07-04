//! Database configuration domain (CORE-016).
//!
//! `DatabaseConfig` is the reusable configuration subtree for the
//! persistence layer. It implements [`ego_domain::Validate`] so
//! applications can enforce its invariants without any dependency on
//! `kit-config` — structural validation (parsing, required fields) stays
//! owned by kit-config (KIT-001); this only checks domain invariants.

use serde::Deserialize;

/// Configuration for a PostgreSQL connection pool.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DatabaseConfig {
    /// The database connection URL.
    pub url: String,
    /// The maximum number of pooled connections.
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "postgres://localhost:5432/ego".to_string(),
            max_connections: 10,
        }
    }
}

impl ego_domain::Validate for DatabaseConfig {
    fn validate(&self) -> Result<(), ego_domain::ConfigError> {
        if self.url.is_empty() {
            return Err(ego_domain::ConfigError::not_empty("url"));
        }
        if self.max_connections == 0 {
            return Err(ego_domain::ConfigError::non_zero("max_connections"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod database_config_validate_tests {
    use super::*;
    use ego_domain::{ConfigError, Validate};

    #[test]
    fn default_config_is_valid() {
        assert!(DatabaseConfig::default().validate().is_ok());
    }

    #[test]
    fn empty_url_is_invalid() {
        let config = DatabaseConfig {
            url: String::new(),
            ..Default::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "url".to_string(),
                reason: "must not be empty".to_string(),
            })
        );
    }

    #[test]
    fn zero_max_connections_is_invalid() {
        let config = DatabaseConfig {
            max_connections: 0,
            ..Default::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "max_connections".to_string(),
                reason: "must be non-zero".to_string(),
            })
        );
    }
}
