//! Trait for configuration providers.

use crate::error::ConfigurationError;
use crate::source::ConfigurationSource;

/// A source of configuration key-value pairs.
///
/// Implementors produce a [`ConfigurationSource`] snapshot when [`load`](Self::load)
/// is called. Loading is synchronous and happens exactly once at build time.
/// Async or remote providers must block internally or pre-fetch eagerly.
///
/// All implementors must be `Send + Sync`.
pub trait ConfigurationProvider: Send + Sync {
    /// Returns the provider's name, used in error messages and source attribution.
    fn name(&self) -> &str;

    /// Loads all key-value pairs from this provider into an immutable snapshot.
    ///
    /// Called exactly once during `ConfigurationBuilder::build`.
    /// Must not have side effects beyond reading the configuration source.
    fn load(&self) -> Result<ConfigurationSource, ConfigurationError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ConfigurationSource;
    use crate::error::ConfigurationError;
    use crate::value::ConfigValue;
    use std::collections::HashMap;

    fn assert_send_sync<T: Send + Sync + ?Sized>() {}

    /// Hand-rolled stub for trait tests.
    struct StubProvider {
        name: String,
        data: HashMap<String, ConfigValue>,
    }

    impl ConfigurationProvider for StubProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn load(&self) -> Result<ConfigurationSource, ConfigurationError> {
            ConfigurationSource::new(self.data.clone(), self.name.clone())
        }
    }

    #[test]
    fn provider_trait_object_is_send_sync() {
        assert_send_sync::<dyn ConfigurationProvider>();
    }

    #[test]
    fn stub_provider_name_returns_name() {
        let p = StubProvider {
            name: "stub".to_string(),
            data: HashMap::new(),
        };
        assert_eq!(p.name(), "stub");
    }

    #[test]
    fn stub_provider_load_returns_source() {
        let mut data = HashMap::new();
        data.insert("x".to_string(), ConfigValue::Str("1".to_string()));
        let p = StubProvider {
            name: "stub".to_string(),
            data,
        };
        let src = p.load().expect("load should succeed");
        assert_eq!(src.provider_name(), "stub");
        assert_eq!(
            src.get("x"),
            Some(&ConfigValue::Str("1".to_string()))
        );
    }

    #[test]
    fn stub_provider_different_provider_name() {
        let p = StubProvider {
            name: "env:APP_".to_string(),
            data: HashMap::new(),
        };
        assert_eq!(p.name(), "env:APP_");
    }
}
