//! Error types for configuration operations.

use thiserror::Error;

/// Errors that can occur during configuration loading and access.
#[derive(Debug, Error)]
pub enum ConfigurationError {
    /// A provider failed to load its configuration.
    #[error("provider '{provider_name}' failed to load: {cause}")]
    ProviderLoad {
        /// Provider identifier.
        provider_name: String,
        /// Human-readable cause.
        cause: String,
    },

    /// A key produced by a provider is not a valid dotted-path identifier.
    #[error("provider '{provider_name}' produced invalid key '{key}'")]
    InvalidKey {
        /// The invalid key.
        key: String,
        /// Provider that produced the key.
        provider_name: String,
    },

    /// A required key was not found in any provider.
    #[error("required key '{key}' not found (searched: {searched_providers:?})")]
    Missing {
        /// The missing key.
        key: String,
        /// Providers that were searched.
        searched_providers: Vec<String>,
    },

    /// The same key was found in multiple equal-priority providers under `Strict` policy.
    #[error("key '{key}' found in multiple equal-priority providers: {sources:?}")]
    Conflict {
        /// The conflicting key.
        key: String,
        /// Equal-priority providers that each have this key.
        sources: Vec<String>,
    },

    /// A value could not be coerced to the requested type.
    #[error("key '{key}': expected type '{expected}', found '{found}'")]
    TypeMismatch {
        /// The key whose value could not be coerced.
        key: String,
        /// Requested type name — compile-time constant.
        expected: &'static str,
        /// Actual value string from the config source — runtime-owned.
        found: String,
    },

    /// Multiple errors collected during a single build operation.
    #[error("{} configuration error(s) occurred", .0.len())]
    Multiple(Vec<ConfigurationError>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn configuration_error_is_send_sync() {
        assert_send_sync::<ConfigurationError>();
    }

    #[test]
    fn provider_load_display_contains_provider_name() {
        let e = ConfigurationError::ProviderLoad {
            provider_name: "env:APP_".to_string(),
            cause: "permission denied".to_string(),
        };
        let s = e.to_string();
        assert!(s.contains("env:APP_"), "display: {s}");
        assert!(s.contains("permission denied"), "display: {s}");
    }

    #[test]
    fn invalid_key_display_contains_key_and_provider() {
        let e = ConfigurationError::InvalidKey {
            key: ".bad".to_string(),
            provider_name: "cli".to_string(),
        };
        let s = e.to_string();
        assert!(s.contains(".bad"), "display: {s}");
        assert!(s.contains("cli"), "display: {s}");
    }

    #[test]
    fn missing_display_contains_key_and_providers() {
        let e = ConfigurationError::Missing {
            key: "server.port".to_string(),
            searched_providers: vec!["env".to_string(), "toml".to_string()],
        };
        let s = e.to_string();
        assert!(s.contains("server.port"), "display: {s}");
        assert!(s.contains("env"), "display: {s}");
        assert!(s.contains("toml"), "display: {s}");
    }

    #[test]
    fn conflict_display_contains_key() {
        let e = ConfigurationError::Conflict {
            key: "db.url".to_string(),
            sources: vec!["toml:/a.toml".to_string(), "yaml:/b.yaml".to_string()],
        };
        let s = e.to_string();
        assert!(s.contains("db.url"), "display: {s}");
    }

    #[test]
    fn type_mismatch_display_contains_expected_and_found_value() {
        let e = ConfigurationError::TypeMismatch {
            key: "timeout".to_string(),
            expected: "u64",
            found: "not-a-number".to_string(),
        };
        let s = e.to_string();
        assert!(s.contains("timeout"), "display: {s}");
        assert!(s.contains("u64"), "display: {s}");
        assert!(s.contains("not-a-number"), "display: {s}");
    }

    #[test]
    fn type_mismatch_different_type() {
        let e = ConfigurationError::TypeMismatch {
            key: "enabled".to_string(),
            expected: "bool",
            found: "maybe".to_string(),
        };
        let s = e.to_string();
        assert!(s.contains("bool"), "display: {s}");
        assert!(s.contains("maybe"), "display: {s}");
    }

    #[test]
    fn multiple_variant_wraps_errors() {
        let e = ConfigurationError::Multiple(vec![
            ConfigurationError::Missing {
                key: "a".to_string(),
                searched_providers: vec![],
            },
            ConfigurationError::Missing {
                key: "b".to_string(),
                searched_providers: vec![],
            },
        ]);
        let s = e.to_string();
        assert!(s.contains("2"), "display should contain error count: {s}");
    }
}
