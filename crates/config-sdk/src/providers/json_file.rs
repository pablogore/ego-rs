//! JSON file configuration provider backed by kit-config.

use kit_config::sources::JsonFileSource;
use kit_config::ConfigurationSource as KitSource;

use crate::error::ConfigurationError;
use crate::provider::ConfigurationProvider;
use crate::source::ConfigurationSource;

use super::convert::value_to_config_map;

/// Loads configuration from a JSON file.
///
/// Nested objects are flattened to dotted-path keys:
/// `{"server": {"host": "localhost"}}` → key `"server.host"`.
///
/// # Optional files
///
/// Use [`JsonFileProvider::optional`] for files that may not exist.
#[derive(Debug)]
pub struct JsonFileProvider {
    name: String,
    source: JsonFileSource,
}

impl JsonFileProvider {
    /// Creates a provider for the JSON file at `path`.
    pub fn new(path: impl Into<String>) -> Self {
        let path = path.into();
        let name = format!("json:{path}");
        Self { name, source: JsonFileSource::new(path, false) }
    }

    /// Creates an optional provider — a missing file yields an empty source.
    pub fn optional(path: impl Into<String>) -> Self {
        let path = path.into();
        let name = format!("json:{path}");
        Self { name, source: JsonFileSource::new(path, true) }
    }
}

impl ConfigurationProvider for JsonFileProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn load(&self) -> Result<ConfigurationSource, ConfigurationError> {
        let value = KitSource::load(&self.source).map_err(|e| ConfigurationError::ProviderLoad {
            provider_name: self.name.clone(),
            cause: Box::new(e),
        })?;
        let map = value_to_config_map(value);
        ConfigurationSource::new(map, self.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::ConfigurationBuilder;
    use std::io::Write as _;

    fn write_temp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn loads_flat_key_values() {
        let f = write_temp(r#"{"host": "localhost", "port": 8080}"#);
        let src = JsonFileProvider::new(f.path().to_str().unwrap())
            .load()
            .expect("should load");
        assert_eq!(src.get("host"), Some(&crate::value::ConfigValue::Str("localhost".into())));
        assert_eq!(src.get("port"), Some(&crate::value::ConfigValue::Int(8080)));
    }

    #[test]
    fn flattens_nested_object() {
        let f = write_temp(r#"{"server": {"host": "localhost", "port": 9000}}"#);
        let src = JsonFileProvider::new(f.path().to_str().unwrap())
            .load()
            .expect("should load");
        assert_eq!(
            src.get("server.host"),
            Some(&crate::value::ConfigValue::Str("localhost".into()))
        );
        assert_eq!(src.get("server.port"), Some(&crate::value::ConfigValue::Int(9000)));
    }

    #[test]
    fn missing_file_required_returns_provider_load_error() {
        let err = JsonFileProvider::new("/no/such/file.json")
            .load()
            .expect_err("missing file must error");
        assert!(matches!(err, ConfigurationError::ProviderLoad { .. }));
    }

    #[test]
    fn missing_file_optional_returns_empty_source() {
        let src = JsonFileProvider::optional("/no/such/file.json")
            .load()
            .expect("optional missing must not error");
        assert_eq!(src.keys().count(), 0);
    }

    #[test]
    fn integrates_with_configuration_builder() {
        let f = write_temp(r#"{"app": {"name": "myapp", "version": "1.0"}}"#);
        let cfg = ConfigurationBuilder::new()
            .add_source(10, JsonFileProvider::new(f.path().to_str().unwrap()))
            .build()
            .expect("should build");
        assert_eq!(cfg.get("app.name").unwrap(), "myapp");
        assert_eq!(cfg.get("app.version").unwrap(), "1.0");
    }
}
