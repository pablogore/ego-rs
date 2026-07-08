//! Test configuration builder — [`TestConfig`] (CORE-022 Phase 6, design.md AD-5;
//! drain mechanism resolved in Phase 8 — see the note on [`TestConfig::drain_into`]).

use ego_service_sdk::{ConfigurationProvider, RuntimeBuilder};

/// Collects test configuration along two separate, non-overlapping views: a
/// typed collection observable through `resolve_config::<C>()` once drained
/// into a real `Runtime` (Phase 8, AD-9), and a JSON-subtree view exposed
/// through the real [`ConfigurationProvider`].
///
/// `.with_value` and `.set` never touch each other's storage.
pub struct TestConfig {
    root: serde_json::Value,
    typed: Vec<Box<dyn FnOnce(RuntimeBuilder) -> RuntimeBuilder + Send>>,
}

impl TestConfig {
    /// Starts with an empty JSON root and no typed values registered.
    pub fn new() -> Self {
        Self {
            root: serde_json::Value::Object(serde_json::Map::new()),
            typed: Vec::new(),
        }
    }

    /// Registers a typed config value resolvable via `resolve_config::<C>()`
    /// once the fixture drains this `TestConfig` into a `RuntimeBuilder`.
    /// Distinct types coexist; registering the same
    /// type twice overwrites the prior value (matches
    /// `RuntimeBuilder::with_config`'s own last-write-wins semantics).
    pub fn with_value<C: Send + Sync + 'static>(mut self, value: C) -> Self {
        self.typed.push(Box::new(move |builder: RuntimeBuilder| {
            builder.with_config(std::sync::Arc::new(value))
        }));
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

    /// Applies every registered `.with_value::<C>()` closure to `builder`, in
    /// insertion order, and returns the resulting builder.
    ///
    /// Resolved (Phase 8, design.md AD-5 "Open item"): production's only
    /// config-registration entry point, `RuntimeBuilder::with_config::<C>()`,
    /// is generic and needs a concrete `C` at the call site — a type-erased
    /// `HashMap<TypeId, Arc<dyn Any>>` cannot be drained into it without
    /// already knowing `C` per entry, and `DependencyTable`/its
    /// `with_registrations` constructor are `pub(super)` in `service-sdk`
    /// (unreachable from `testkit`). Each `with_value::<C>()` call now
    /// captures a closure that already knows its own concrete `C`, so
    /// draining never needs to inspect a type-erased map. Because
    /// `RuntimeBuilder::with_config` itself does last-write-wins by
    /// `TypeId` (`self.configs.insert(TypeId::of::<C>(), ..)`), folding these
    /// closures in insertion order reproduces the exact same last-write-wins
    /// semantics as calling `with_config` directly multiple times.
    pub(crate) fn drain_into(self, builder: RuntimeBuilder) -> RuntimeBuilder {
        self.typed.into_iter().fold(builder, |b, apply| apply(b))
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

        // Stronger than the old direct-`.typed`-inspection test: drains into a
        // real `RuntimeBuilder` and observes both values through the real
        // `resolve_config::<C>()` seam a service would use.
        let rt = config.drain_into(RuntimeBuilder::new()).build();
        assert_eq!(*rt.inner().resolve_config::<u32>().unwrap(), 42);
        assert_eq!(
            *rt.inner().resolve_config::<String>().unwrap(),
            "s".to_string()
        );
    }

    #[test]
    fn with_value_same_type_twice_overwrites_prior_value() {
        let config = TestConfig::new().with_value(1u32).with_value(2u32);

        let rt = config.drain_into(RuntimeBuilder::new()).build();
        assert_eq!(*rt.inner().resolve_config::<u32>().unwrap(), 2);
    }

    #[test]
    fn set_is_reflected_in_provider_json_view() {
        let config =
            TestConfig::new().set("logging", json!({ "enabled": false, "format": "text" }));

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
        assert_eq!(
            config.root,
            serde_json::Value::Object(serde_json::Map::new())
        );
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
