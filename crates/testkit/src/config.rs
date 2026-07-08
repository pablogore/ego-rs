//! Test configuration builder — [`TestConfig`] (CORE-022 Phase 6, design.md AD-5).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use ego_service_sdk::ConfigurationProvider;

/// Collects test configuration along two separate, non-overlapping views:
/// a typed collection (keyed by [`TypeId`], matching the container shape
/// `RuntimeInner`'s `DependencyTable` uses for `resolve_config::<C>()`) and a
/// JSON-subtree view exposed through the real [`ConfigurationProvider`].
///
/// `.with_value` and `.set` never touch each other's storage.
///
/// Container-shape match is not, by itself, an integration path: production's
/// only config-registration entry point, `RuntimeBuilder::with_config::<C>()`,
/// is generic and needs a concrete `C` at the call site — a type-erased
/// `HashMap<TypeId, Arc<dyn Any>>` cannot be drained into it without knowing
/// `C` per entry. Phase 8 (AD-9 fixture wiring) must resolve this — e.g. by
/// capturing a `Box<dyn FnOnce(RuntimeBuilder) -> RuntimeBuilder>` per call to
/// `with_value::<C>()` instead of a type-erased value — before this typed
/// collection can actually reach a real `Runtime`.
pub struct TestConfig {
    root: serde_json::Value,
    typed: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl TestConfig {
    /// Starts with an empty JSON root and no typed values registered.
    pub fn new() -> Self {
        Self {
            root: serde_json::Value::Object(serde_json::Map::new()),
            typed: HashMap::new(),
        }
    }

    /// Registers a typed config value resolvable via `resolve_config::<C>()`
    /// once fed into `RuntimeBuilder::with_config` (Phase 8). Distinct types
    /// coexist; registering the same type twice overwrites the prior value.
    pub fn with_value<C: Send + Sync + 'static>(mut self, value: C) -> Self {
        self.typed.insert(TypeId::of::<C>(), Arc::new(value));
        self
    }

    /// Sets a key in the JSON-subtree view. Never touches the typed
    /// collection — this is `ConfigurationProvider`'s contract, not
    /// `resolve_config::<C>()`'s.
    pub fn set(mut self, key: impl Into<String>, value: impl serde::Serialize) -> Self {
        let value = serde_json::to_value(value).expect("TestConfig::set value must serialize");
        self.root
            .as_object_mut()
            .expect("TestConfig root is always a JSON object")
            .insert(key.into(), value);
        self
    }

    /// Real `ConfigurationProvider` over the JSON root accumulated by `.set`.
    /// Reflects nothing registered via `.with_value`.
    pub fn provider(&self) -> ConfigurationProvider {
        ConfigurationProvider::from_value(self.root.clone())
    }
}

impl Default for TestConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn with_value_collects_distinct_types_without_loss() {
        let config = TestConfig::new()
            .with_value(42u32)
            .with_value("s".to_string());

        assert_eq!(config.typed.len(), 2);
        let stored_u32 = config.typed[&TypeId::of::<u32>()]
            .clone()
            .downcast::<u32>()
            .expect("u32 value stored under its own TypeId");
        assert_eq!(*stored_u32, 42);
        let stored_string = config.typed[&TypeId::of::<String>()]
            .clone()
            .downcast::<String>()
            .expect("String value stored under its own TypeId");
        assert_eq!(*stored_string, "s");
    }

    #[test]
    fn with_value_same_type_twice_overwrites_prior_value() {
        let config = TestConfig::new().with_value(1u32).with_value(2u32);

        assert_eq!(config.typed.len(), 1);
        let stored = config.typed[&TypeId::of::<u32>()]
            .clone()
            .downcast::<u32>()
            .expect("u32 value stored under its own TypeId");
        assert_eq!(*stored, 2);
    }

    #[test]
    fn set_is_reflected_in_provider_json_view() {
        let config = TestConfig::new().set(
            "logging",
            json!({ "enabled": false, "format": "text" }),
        );

        // Round-trips through the REAL `ConfigurationProvider` — proves
        // `.set()` actually reaches the JSON root the provider reads from,
        // not just an internal field.
        let settings = config.provider().logging().expect("valid logging view");
        assert!(!settings.enabled);
        assert_eq!(settings.format, ego_service_sdk::LogFormatSetting::Text);
    }

    #[test]
    fn set_alone_leaves_typed_collection_empty() {
        let config = TestConfig::new().set("k", json!("v"));
        assert!(config.typed.is_empty());
    }

    #[test]
    fn with_value_alone_leaves_root_untouched() {
        let config = TestConfig::new().with_value(42u32);
        assert_eq!(config.root, serde_json::Value::Object(serde_json::Map::new()));
    }

    #[test]
    fn provider_reflects_only_set_accumulated_json() {
        let config = TestConfig::new()
            .set("logging", json!({ "enabled": true, "format": "json" }))
            .with_value(42u32);

        // Direct comparison catches a leak under ANY key `.with_value` might
        // write — `ConfigurationProvider`'s own accessors (e.g. `.logging()`)
        // only ever read one key, so they can't detect a leak elsewhere.
        assert_eq!(
            config.root,
            json!({ "logging": { "enabled": true, "format": "json" } })
        );

        // Still a real `ConfigurationProvider`, readable through its own
        // accessor — not just an internal field wrapper.
        let settings = config.provider().logging().expect("valid logging view");
        assert!(settings.enabled);
        assert_eq!(settings.format, ego_service_sdk::LogFormatSetting::Json);
    }
}
