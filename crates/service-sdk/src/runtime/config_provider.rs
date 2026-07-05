//! Thin host-boundary role for CORE-017's config + logger bootstrap.
//!
//! `ConfigurationProvider` wraps a `serde_json::Value` already materialized by
//! `kit_config::ConfigLoader` (host side, per CORE-016) and exposes the
//! narrow logging view CORE-017 actually consumes. It owns no sources, merge,
//! or parse logic — kit-config remains the owner of the full configuration
//! model and its structural validation. Called by the HOST, before
//! `RuntimeBuilder::new()` — never by `RuntimeBuilder` itself.

use serde::Deserialize;

use super::error::RuntimeInfraError;

/// Narrow consumer view of the logging configuration subtree.
///
/// NOT a reimplementation of kit-config's `LoggingConfig` — only the fields
/// CORE-017 actually consumes. kit-config owns and validates the full model.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LoggingSettings {
    /// Gates whether `build_logger` runs at all — the only "off" mechanism
    /// kitlogger's API supports (see design.md's `enabled` decision).
    pub enabled: bool,
    pub format: LogFormatSetting,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            format: LogFormatSetting::default(),
        }
    }
}

/// Mirrors config-models' logging format strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormatSetting {
    Json,
    Pretty,
    Compact,
    Text,
}

impl Default for LogFormatSetting {
    fn default() -> Self {
        LogFormatSetting::Json
    }
}

/// Holds the config already materialized by `kit_config::ConfigLoader`
/// (host side). Owns no sources/merge/parse — it only exposes the consumed
/// logging view.
pub struct ConfigurationProvider {
    root: serde_json::Value,
}

impl ConfigurationProvider {
    pub fn from_value(root: serde_json::Value) -> Self {
        Self { root }
    }

    /// Deserialize the consumed logging view. Any structural error is fail-fast.
    pub fn logging(&self) -> Result<LoggingSettings, RuntimeInfraError> {
        let node = self.root.get("logging").cloned().unwrap_or_default();
        serde_json::from_value(node)
            .map_err(|e| RuntimeInfraError::ConfigInvalid { reason: e.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn logging_parses_valid_view() {
        let provider = ConfigurationProvider::from_value(json!({
            "logging": { "enabled": true, "format": "json" }
        }));

        let settings = provider.logging().expect("valid logging config");
        assert!(settings.enabled);
        assert_eq!(settings.format, LogFormatSetting::Json);
    }

    #[test]
    fn logging_malformed_format_is_config_invalid() {
        let provider = ConfigurationProvider::from_value(json!({
            "logging": { "enabled": true, "format": "not-a-real-format" }
        }));

        let result = provider.logging();
        assert!(matches!(result, Err(RuntimeInfraError::ConfigInvalid { .. })));
    }

    #[test]
    fn logging_malformed_type_is_config_invalid() {
        // `enabled` must be a bool, not a string.
        let provider = ConfigurationProvider::from_value(json!({
            "logging": { "enabled": "yes", "format": "json" }
        }));

        let result = provider.logging();
        assert!(matches!(result, Err(RuntimeInfraError::ConfigInvalid { .. })));
    }

    #[test]
    fn logging_missing_subtree_is_config_invalid() {
        // `#[serde(default)]` on `LoggingSettings` fills missing *fields*
        // within a map — it does not rescue deserialization when the
        // top-level node itself is `Value::Null` (what `.get("logging")`
        // yields when the key is entirely absent, per
        // `unwrap_or_default()`). An absent `logging` subtree is therefore
        // `ConfigInvalid`, not "all defaults" — hosts must supply at least
        // an empty `"logging": {}` object to get defaults.
        let provider = ConfigurationProvider::from_value(json!({}));
        let result = provider.logging();
        assert!(matches!(result, Err(RuntimeInfraError::ConfigInvalid { .. })));
    }

    #[test]
    fn logging_empty_object_uses_defaults() {
        let provider = ConfigurationProvider::from_value(json!({ "logging": {} }));
        let settings = provider.logging().expect("empty object applies field defaults");
        assert!(settings.enabled);
        assert_eq!(settings.format, LogFormatSetting::Json);
    }
}
