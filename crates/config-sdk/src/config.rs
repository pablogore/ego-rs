//! Resolved, immutable configuration handle.

use std::collections::HashMap;

use serde_json::Value;

use crate::convert::value_to_config_map;
use crate::error::ConfigurationError;
use crate::value::ConfigValue;

/// A resolved, immutable snapshot of all configuration values.
///
/// Build from a `serde_json::Value` (e.g. produced by `kit_config::ConfigLoader::load()`).
/// `Configuration` is `Send + Sync` — it can be wrapped in `Arc` and shared across threads.
#[derive(Debug)]
pub struct Configuration {
    entries: HashMap<String, ConfigValue>,
}

impl Configuration {
    /// Build from a `serde_json::Value` produced by `kit_config::ConfigLoader::load()`.
    pub fn from_value(value: Value) -> Self {
        Self { entries: value_to_config_map(value) }
    }

    /// Get a value by dotted key.
    pub fn get(&self, key: &str) -> Result<&ConfigValue, ConfigurationError> {
        self.entries
            .get(key)
            .ok_or_else(|| ConfigurationError::Missing { key: key.to_string() })
    }

    /// Get a typed value. `T` must implement `FromStr`.
    pub fn get_typed<T: std::str::FromStr>(&self, key: &str) -> Result<T, ConfigurationError>
    where
        T::Err: std::fmt::Display,
    {
        let v = self.get(key)?;
        let s = match v {
            ConfigValue::Str(s) => s.clone(),
            ConfigValue::Int(n) => n.to_string(),
            ConfigValue::Float(f) => f.to_string(),
            ConfigValue::Bool(b) => b.to_string(),
            ConfigValue::List(_) => {
                return Err(ConfigurationError::TypeMismatch {
                    key: key.to_string(),
                    expected: std::any::type_name::<T>(),
                    found: "list".to_string(),
                })
            }
        };
        s.parse::<T>().map_err(|e| ConfigurationError::TypeMismatch {
            key: key.to_string(),
            expected: std::any::type_name::<T>(),
            found: e.to_string(),
        })
    }

    /// Validate that all required keys are present.
    ///
    /// Returns all missing keys as a single [`ConfigurationError::Multiple`] error when
    /// more than one key is absent; a single [`ConfigurationError::Missing`] otherwise.
    pub fn require(&self, keys: &[&str]) -> Result<(), ConfigurationError> {
        let missing: Vec<ConfigurationError> = keys
            .iter()
            .filter(|&&k| !self.entries.contains_key(k))
            .map(|&k| ConfigurationError::Missing { key: k.to_string() })
            .collect();
        match missing.len() {
            0 => Ok(()),
            1 => Err(missing.into_iter().next().unwrap()),
            _ => Err(ConfigurationError::Multiple(missing)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_value_nested_json() {
        let v = json!({ "server": { "host": "localhost", "port": 8080 } });
        let cfg = Configuration::from_value(v);
        assert!(cfg.get("server.host").is_ok());
        assert!(cfg.get("server.port").is_ok());
    }

    #[test]
    fn get_hit() {
        let v = json!({ "key": "value" });
        let cfg = Configuration::from_value(v);
        assert!(cfg.get("key").is_ok());
    }

    #[test]
    fn get_miss() {
        let cfg = Configuration::from_value(json!({}));
        let err = cfg.get("missing").unwrap_err();
        assert!(matches!(err, ConfigurationError::Missing { .. }));
    }

    #[test]
    fn get_typed_u16() {
        let v = json!({ "port": "3000" });
        let cfg = Configuration::from_value(v);
        let port: u16 = cfg.get_typed("port").unwrap();
        assert_eq!(port, 3000);
    }

    #[test]
    fn get_typed_bool() {
        let v = json!({ "enabled": "true" });
        let cfg = Configuration::from_value(v);
        let enabled: bool = cfg.get_typed("enabled").unwrap();
        assert!(enabled);
    }

    #[test]
    fn require_all_present() {
        let v = json!({ "a": "1", "b": "2" });
        let cfg = Configuration::from_value(v);
        assert!(cfg.require(&["a", "b"]).is_ok());
    }

    #[test]
    fn require_one_missing() {
        let v = json!({ "a": "1" });
        let cfg = Configuration::from_value(v);
        let err = cfg.require(&["a", "missing"]).unwrap_err();
        assert!(matches!(err, ConfigurationError::Missing { .. }));
    }

    #[test]
    fn require_multiple_missing() {
        let cfg = Configuration::from_value(json!({}));
        let err = cfg.require(&["x", "y"]).unwrap_err();
        assert!(matches!(err, ConfigurationError::Multiple(_)));
    }
}
