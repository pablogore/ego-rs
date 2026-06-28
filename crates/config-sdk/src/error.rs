//! Error types for configuration operations.

use thiserror::Error;

fn format_multiple(errors: &[ConfigurationError]) -> String {
    errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ")
}

/// Errors that can occur during configuration access.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum ConfigurationError {
    /// A required key was not found.
    #[error("required key '{key}' not found")]
    Missing { key: String },

    /// A value could not be coerced to the requested type.
    #[error("key '{key}': expected '{expected}', found '{found}'")]
    TypeMismatch { key: String, expected: &'static str, found: String },

    /// Multiple errors collected during a single operation.
    #[error("{}", format_multiple(.0))]
    Multiple(Vec<ConfigurationError>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn config_error_is_send_and_sync() {
        assert_send_sync::<ConfigurationError>();
    }

    #[test]
    fn missing_display() {
        let e = ConfigurationError::Missing { key: "foo".into() };
        assert_eq!(e.to_string(), "required key 'foo' not found");
    }

    #[test]
    fn type_mismatch_display() {
        let e = ConfigurationError::TypeMismatch {
            key: "port".into(),
            expected: "u16",
            found: "list".into(),
        };
        assert_eq!(e.to_string(), "key 'port': expected 'u16', found 'list'");
    }

    #[test]
    fn multiple_display() {
        let e = ConfigurationError::Multiple(vec![
            ConfigurationError::Missing { key: "a".into() },
            ConfigurationError::Missing { key: "b".into() },
        ]);
        assert_eq!(
            e.to_string(),
            "required key 'a' not found; required key 'b' not found"
        );
    }
}
