//! TOML file configuration provider backed by kit-config.

use kit_config::sources::TomlFileSource;
use kit_config::ConfigurationSource as KitSource;

use crate::error::ConfigurationError;
use crate::provider::ConfigurationProvider;
use crate::source::ConfigurationSource;

use super::convert::value_to_config_map;

/// Loads configuration from a TOML file.
///
/// Nested tables are flattened to dotted-path keys:
/// `[server]\nhost = "localhost"` → key `"server.host"`.
///
/// # Optional files
///
/// By default, a missing or unreadable file produces a [`ConfigurationError::ProviderLoad`].
/// Use [`TomlFileProvider::optional`] to silently return an empty source instead.
#[derive(Debug)]
pub struct TomlFileProvider {
    name: String,
    source: TomlFileSource,
}

impl TomlFileProvider {
    /// Creates a provider for the TOML file at `path`.
    ///
    /// Returns an error on load if the file is missing or unparseable.
    pub fn new(path: impl Into<String>) -> Self {
        let path = path.into();
        let name = format!("toml:{path}");
        Self { name, source: TomlFileSource::new(path, false) }
    }

    /// Creates an optional provider — a missing or unreadable file yields an empty source
    /// instead of a [`ConfigurationError::ProviderLoad`].
    pub fn optional(path: impl Into<String>) -> Self {
        let path = path.into();
        let name = format!("toml:{path}");
        Self { name, source: TomlFileSource::new(path, true) }
    }
}

impl ConfigurationProvider for TomlFileProvider {
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

    fn write_temp(content: &str, ext: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(ext).tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn loads_flat_key_values() {
        let f = write_temp("host = \"localhost\"\nport = 8080\n", ".toml");
        let p = TomlFileProvider::new(f.path().to_str().unwrap());
        let src = p.load().expect("should load");
        assert_eq!(src.get("host"), Some(&crate::value::ConfigValue::Str("localhost".into())));
    }

    #[test]
    fn flattens_nested_table() {
        let f = write_temp("[server]\nhost = \"localhost\"\nport = 8080\n", ".toml");
        let p = TomlFileProvider::new(f.path().to_str().unwrap());
        let src = p.load().expect("should load");
        assert_eq!(
            src.get("server.host"),
            Some(&crate::value::ConfigValue::Str("localhost".into()))
        );
        assert_eq!(src.get("server.port"), Some(&crate::value::ConfigValue::Int(8080)));
    }

    #[test]
    fn missing_file_required_returns_provider_load_error() {
        let p = TomlFileProvider::new("/nonexistent/path/config.toml");
        let err = p.load().expect_err("missing file must error");
        assert!(matches!(err, ConfigurationError::ProviderLoad { .. }));
    }

    #[test]
    fn missing_file_optional_returns_empty_source() {
        let p = TomlFileProvider::optional("/nonexistent/path/config.toml");
        let src = p.load().expect("optional missing file must not error");
        assert_eq!(src.keys().count(), 0);
    }

    #[test]
    fn integrates_with_configuration_builder() {
        let f = write_temp("[db]\nurl = \"postgres://localhost/test\"\n", ".toml");
        let cfg = ConfigurationBuilder::new()
            .add_source(10, TomlFileProvider::new(f.path().to_str().unwrap()))
            .build()
            .expect("should build");
        assert_eq!(cfg.get("db.url").unwrap(), "postgres://localhost/test");
    }
}
