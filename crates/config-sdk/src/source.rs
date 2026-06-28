//! Post-load configuration source snapshot.

use std::collections::HashMap;

use crate::error::ConfigurationError;
use crate::value::ConfigValue;

/// An immutable snapshot of key-value pairs loaded from a single provider.
///
/// Produced by [`ConfigurationProvider::load`](crate::provider::ConfigurationProvider::load).
/// Immutable after construction; `Send + Sync`.
#[derive(Debug)]
pub struct ConfigurationSource {
    entries: HashMap<String, ConfigValue>,
    provider_name: String,
}

impl ConfigurationSource {
    /// Creates a new source snapshot.
    ///
    /// Returns [`ConfigurationError::InvalidKey`] if any key fails dotted-path validation.
    pub fn new(
        entries: HashMap<String, ConfigValue>,
        provider_name: String,
    ) -> Result<Self, ConfigurationError> {
        for key in entries.keys() {
            if !is_valid_key(key) {
                return Err(ConfigurationError::InvalidKey {
                    key: key.clone(),
                    provider_name: provider_name.clone(),
                });
            }
        }
        Ok(Self { entries, provider_name })
    }

    /// Returns the value for `key`, or `None` if not present.
    pub fn get(&self, key: &str) -> Option<&ConfigValue> {
        self.entries.get(key)
    }

    /// Iterates over all keys in this snapshot.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Returns the name of the provider that produced this snapshot.
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }
}

/// Returns `true` if `key` is a valid dotted-path identifier.
///
/// Each dot-separated segment must start with an alphanumeric character or `_`,
/// followed by alphanumeric characters, `_`, or `-`. Empty segments (leading/trailing
/// dots or consecutive dots) are rejected.
fn is_valid_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    for segment in key.split('.') {
        if segment.is_empty() {
            return false;
        }
        let mut chars = segment.chars();
        match chars.next() {
            Some(c) if c.is_alphanumeric() || c == '_' => {}
            _ => return false,
        }
        if !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::ConfigValue;
    use std::collections::HashMap;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn source_is_send_sync() {
        assert_send_sync::<ConfigurationSource>();
    }

    #[test]
    fn get_returns_some_for_present_key() {
        let mut data = HashMap::new();
        data.insert("server.port".to_string(), ConfigValue::Str("8080".to_string()));
        let src = ConfigurationSource::new(data, "toml:/app.toml".to_string())
            .expect("should succeed");
        assert_eq!(
            src.get("server.port"),
            Some(&ConfigValue::Str("8080".to_string()))
        );
    }

    #[test]
    fn get_returns_none_for_absent_key() {
        let src = ConfigurationSource::new(HashMap::new(), "stub".to_string())
            .expect("should succeed");
        assert!(src.get("missing").is_none());
    }

    #[test]
    fn keys_iterates_all_keys() {
        let mut data = HashMap::new();
        data.insert("a".to_string(), ConfigValue::Bool(true));
        data.insert("b".to_string(), ConfigValue::Int(42));
        let src = ConfigurationSource::new(data, "stub".to_string())
            .expect("should succeed");
        let mut keys: Vec<&str> = src.keys().collect();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn provider_name_returns_name() {
        let src = ConfigurationSource::new(HashMap::new(), "env:APP_".to_string())
            .expect("should succeed");
        assert_eq!(src.provider_name(), "env:APP_");
    }

    #[test]
    fn provider_name_with_path_style_name() {
        // Realistic provider names contain colons and slashes (e.g. "toml:/etc/app.toml").
        let src = ConfigurationSource::new(HashMap::new(), "toml:/etc/app.toml".to_string())
            .expect("should succeed");
        assert_eq!(src.provider_name(), "toml:/etc/app.toml");
    }

    #[test]
    fn empty_key_returns_invalid_key_error() {
        let mut data = HashMap::new();
        data.insert("".to_string(), ConfigValue::Bool(true));
        let err = ConfigurationSource::new(data, "stub".to_string())
            .expect_err("empty key must be rejected");
        assert!(
            matches!(err, ConfigurationError::InvalidKey { .. }),
            "expected InvalidKey, got: {err:?}"
        );
    }

    #[test]
    fn leading_dot_returns_invalid_key_error() {
        let mut data = HashMap::new();
        data.insert(".foo".to_string(), ConfigValue::Bool(true));
        let err = ConfigurationSource::new(data, "stub".to_string())
            .expect_err("leading dot must be rejected");
        assert!(matches!(err, ConfigurationError::InvalidKey { .. }));
    }

    #[test]
    fn trailing_dot_returns_invalid_key_error() {
        let mut data = HashMap::new();
        data.insert("foo.".to_string(), ConfigValue::Bool(true));
        let err = ConfigurationSource::new(data, "stub".to_string())
            .expect_err("trailing dot must be rejected");
        assert!(matches!(err, ConfigurationError::InvalidKey { .. }));
    }

    #[test]
    fn consecutive_dots_return_invalid_key_error() {
        let mut data = HashMap::new();
        data.insert("foo..bar".to_string(), ConfigValue::Bool(true));
        let err = ConfigurationSource::new(data, "stub".to_string())
            .expect_err("consecutive dots must be rejected");
        assert!(matches!(err, ConfigurationError::InvalidKey { .. }));
    }

    #[test]
    fn hyphen_first_char_returns_invalid_key_error() {
        // Leading hyphen is invalid — first char must be [a-zA-Z0-9_].
        let mut data = HashMap::new();
        data.insert("-foo".to_string(), ConfigValue::Bool(true));
        let err = ConfigurationSource::new(data, "stub".to_string())
            .expect_err("leading hyphen must be rejected");
        assert!(matches!(err, ConfigurationError::InvalidKey { .. }));
    }

    #[test]
    fn valid_dotted_path_succeeds() {
        let mut data = HashMap::new();
        data.insert("server.port".to_string(), ConfigValue::Int(8080));
        data.insert("a-b_c.d".to_string(), ConfigValue::Bool(true));
        ConfigurationSource::new(data, "stub".to_string())
            .expect("valid dotted paths must succeed");
    }
}
