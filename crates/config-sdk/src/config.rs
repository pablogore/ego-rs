//! Resolved, immutable configuration handle.

use std::collections::HashMap;

use crate::error::ConfigurationError;
use crate::value::SourceAttribution;

/// A resolved, immutable snapshot of all configuration values.
///
/// Constructed by [`ConfigurationBuilder::build`](crate::builder::ConfigurationBuilder::build).
/// `Configuration` is `Send + Sync` — it can be wrapped in `Arc` and shared across threads.
/// All values are stored as UTF-8 strings; type coercion happens at access time.
#[derive(Debug)]
pub struct Configuration {
    /// key → (string value, source attribution)
    entries: HashMap<String, (String, SourceAttribution)>,
    /// Names of all successfully loaded providers — used in `Missing` errors.
    provider_names: Vec<String>,
}

impl Configuration {
    /// Creates a new configuration from a resolved entry map.
    ///
    /// Intended for use by [`ConfigurationBuilder`](crate::builder::ConfigurationBuilder) only.
    pub(crate) fn new(
        entries: HashMap<String, (String, SourceAttribution)>,
        provider_names: Vec<String>,
    ) -> Self {
        Self { entries, provider_names }
    }

    /// Returns the string value for `key`.
    ///
    /// Returns [`ConfigurationError::Missing`] when the key is not present.
    pub fn get(&self, key: &str) -> Result<&str, ConfigurationError> {
        self.entries
            .get(key)
            .map(|(v, _)| v.as_str())
            .ok_or_else(|| ConfigurationError::Missing {
                key: key.to_owned(),
                searched_providers: self.provider_names.clone(),
            })
    }

    /// Returns the string value for `key`, or `default` when the key is absent.
    pub fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.entries
            .get(key)
            .map(|(v, _)| v.as_str())
            .unwrap_or(default)
    }

    /// Returns the value for `key` parsed as `T`.
    ///
    /// Returns [`ConfigurationError::TypeMismatch`] when parsing fails.  `found` is the raw
    /// string value that could not be parsed; `expected` is the Rust type name of `T`.
    pub fn get_typed<T: std::str::FromStr>(&self, key: &str) -> Result<T, ConfigurationError> {
        let value = self.get(key)?;
        value.parse::<T>().map_err(|_| ConfigurationError::TypeMismatch {
            key: key.to_owned(),
            expected: std::any::type_name::<T>(),
            found: value.to_owned(),
        })
    }

    /// Iterates over all resolved keys.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Returns the resolution trace for `key`.
    ///
    /// Each entry identifies which provider contributed the value.
    /// Returns an empty `Vec` when the key is not present.
    pub fn debug_trace(&self, key: &str) -> Vec<SourceAttribution> {
        self.entries
            .get(key)
            .map(|(_, attr)| vec![attr.clone()])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::ConfigurationBuilder;
    use crate::error::ConfigurationError;
    use crate::provider::ConfigurationProvider;
    use crate::source::ConfigurationSource;
    use crate::value::ConfigValue;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct FixedProvider {
        name: String,
        data: HashMap<String, ConfigValue>,
    }

    impl ConfigurationProvider for FixedProvider {
        fn name(&self) -> &str {
            &self.name
        }
        fn load(&self) -> Result<ConfigurationSource, ConfigurationError> {
            ConfigurationSource::new(self.data.clone(), self.name.clone())
        }
    }

    fn config_with(pairs: &[(&str, &str)]) -> Configuration {
        let data = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), ConfigValue::Str(v.to_string())))
            .collect();
        ConfigurationBuilder::new()
            .add_source(10, FixedProvider { name: "test".into(), data })
            .build()
            .expect("should succeed")
    }

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn configuration_is_send_sync() {
        assert_send_sync::<Configuration>();
    }

    #[test]
    fn s06_get_or_absent_key_returns_default() {
        let cfg = config_with(&[]);
        assert_eq!(cfg.get_or("missing.key", "default-val"), "default-val");
    }

    #[test]
    fn s07_get_typed_u64_success() {
        let cfg = config_with(&[("server.port", "8080")]);
        let port: u64 = cfg.get_typed("server.port").expect("should parse");
        assert_eq!(port, 8080u64);
    }

    #[test]
    fn s08_get_typed_type_mismatch_found_field() {
        let cfg = config_with(&[("timeout", "not-a-number")]);
        let err = cfg.get_typed::<u64>("timeout").expect_err("should fail to parse");
        assert!(
            matches!(&err, ConfigurationError::TypeMismatch { found, .. } if found == "not-a-number"),
            "expected TypeMismatch with found='not-a-number', got: {err:?}"
        );
    }

    #[test]
    fn s09_arc_configuration_shared_across_threads() {
        let cfg = Arc::new(config_with(&[("x", "42")]));
        let c1 = Arc::clone(&cfg);
        let c2 = Arc::clone(&cfg);
        let t1 = std::thread::spawn(move || c1.get("x").unwrap().to_owned());
        let t2 = std::thread::spawn(move || c2.get("x").unwrap().to_owned());
        assert_eq!(t1.join().unwrap(), "42");
        assert_eq!(t2.join().unwrap(), "42");
    }

    #[test]
    fn s11_debug_trace_returns_attribution() {
        let cfg = config_with(&[("db.url", "postgres://localhost")]);
        let trace = cfg.debug_trace("db.url");
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].key, "db.url");
        assert_eq!(trace[0].provider_name, "test");
    }

    #[test]
    fn keys_returns_all_resolved_keys() {
        let cfg = config_with(&[("a", "1"), ("b", "2")]);
        let mut keys: Vec<&str> = cfg.keys().collect();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn get_returns_err_for_absent_key() {
        let cfg = config_with(&[]);
        let err = cfg.get("absent.key").expect_err("absent key must error");
        assert!(matches!(err, ConfigurationError::Missing { .. }));
    }

    #[test]
    fn get_typed_bool_success() {
        let cfg = config_with(&[("feature.enabled", "true")]);
        let v: bool = cfg.get_typed("feature.enabled").expect("should parse bool");
        assert!(v);
    }
}
