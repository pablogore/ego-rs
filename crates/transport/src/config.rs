//! gRPC server configuration domain (CORE-016).
//!
//! `GrpcServerConfig` is the reusable configuration subtree for the
//! transport layer. It implements [`ego_domain::Validate`] so
//! applications can enforce its invariants without any dependency on
//! `kit-config` — structural validation (parsing, required fields) stays
//! owned by kit-config (KIT-001); this only checks domain invariants.

use serde::Deserialize;

/// Configuration for a gRPC server bind endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GrpcServerConfig {
    /// The address the server binds to.
    pub bind_address: String,
    /// The TCP port the server listens on.
    pub port: u16,
}

impl Default for GrpcServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0".to_string(),
            port: 50051,
        }
    }
}

impl ego_domain::Validate for GrpcServerConfig {
    fn validate(&self) -> Result<(), ego_domain::ConfigError> {
        if self.bind_address.is_empty() {
            return Err(ego_domain::ConfigError::not_empty("bind_address"));
        }
        if self.port == 0 {
            return Err(ego_domain::ConfigError::non_zero("port"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod grpc_server_config_validate_tests {
    use super::*;
    use ego_domain::{ConfigError, Validate};

    #[test]
    fn default_config_is_valid() {
        assert!(GrpcServerConfig::default().validate().is_ok());
    }

    #[test]
    fn empty_bind_address_is_invalid() {
        let config = GrpcServerConfig {
            bind_address: String::new(),
            ..Default::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "bind_address".to_string(),
                reason: "must not be empty".to_string(),
            })
        );
    }

    #[test]
    fn zero_port_is_invalid() {
        let config = GrpcServerConfig {
            port: 0,
            ..Default::default()
        };
        assert_eq!(
            config.validate(),
            Err(ConfigError::Invalid {
                field: "port".to_string(),
                reason: "must be non-zero".to_string(),
            })
        );
    }
}
