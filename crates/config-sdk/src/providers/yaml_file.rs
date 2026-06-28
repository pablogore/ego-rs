//! YAML file configuration provider backed by kit-config.

use kit_config::sources::YamlFileSource;
use kit_config::ConfigurationSource as KitSource;

use crate::error::ConfigurationError;
use crate::provider::ConfigurationProvider;
use crate::source::ConfigurationSource;

use super::convert::value_to_config_map;

/// Loads configuration from a YAML file.
///
/// Nested mappings are flattened to dotted-path keys:
/// `server:\n  host: localhost` → key `"server.host"`.
///
/// # Optional files
///
/// Use [`YamlFileProvider::optional`] for files that may not exist.
#[derive(Debug)]
pub struct YamlFileProvider {
    name: String,
    source: YamlFileSource,
}

impl YamlFileProvider {
    /// Creates a provider for the YAML file at `path`.
    pub fn new(path: impl Into<String>) -> Self {
        let path = path.into();
        let name = format!("yaml:{path}");
        Self { name, source: YamlFileSource::new(path, false) }
    }

    /// Creates an optional provider — a missing file yields an empty source.
    pub fn optional(path: impl Into<String>) -> Self {
        let path = path.into();
        let name = format!("yaml:{path}");
        Self { name, source: YamlFileSource::new(path, true) }
    }
}

impl ConfigurationProvider for YamlFileProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn load(&self) -> Result<ConfigurationSource, ConfigurationError> {
        let value = KitSource::load(&self.source).map_err(|e| ConfigurationError::ProviderLoad {
            provider_name: self.name.clone(),
            cause: e.to_string(),
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
        let mut f = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn loads_flat_key_values() {
        let f = write_temp("host: localhost\nport: 8080\n");
        let src = YamlFileProvider::new(f.path().to_str().unwrap())
            .load()
            .expect("should load");
        assert_eq!(src.get("host"), Some(&crate::value::ConfigValue::Str("localhost".into())));
        assert_eq!(src.get("port"), Some(&crate::value::ConfigValue::Int(8080)));
    }

    #[test]
    fn flattens_nested_mapping() {
        let f = write_temp("server:\n  host: localhost\n  port: 9000\n");
        let src = YamlFileProvider::new(f.path().to_str().unwrap())
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
        let err = YamlFileProvider::new("/no/such/file.yaml")
            .load()
            .expect_err("missing file must error");
        assert!(matches!(err, ConfigurationError::ProviderLoad { .. }));
    }

    #[test]
    fn missing_file_optional_returns_empty_source() {
        let src = YamlFileProvider::optional("/no/such/file.yaml")
            .load()
            .expect("optional missing must not error");
        assert_eq!(src.keys().count(), 0);
    }

    #[test]
    fn integrates_with_configuration_builder() {
        let f = write_temp("logging:\n  level: debug\n  format: json\n");
        let cfg = ConfigurationBuilder::new()
            .add_source(10, YamlFileProvider::new(f.path().to_str().unwrap()))
            .build()
            .expect("should build");
        assert_eq!(cfg.get("logging.level").unwrap(), "debug");
        assert_eq!(cfg.get("logging.format").unwrap(), "json");
    }
}
