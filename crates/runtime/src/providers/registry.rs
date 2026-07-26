//! Provider registry, one owner per `provider_id` (CORE-019A Phase 2,
//! AD-002/AD-005).
//!
//! Fail-closed at registration time (spec: "Duplicate Registration Fails At
//! Registration Time") — no last-wins, first-wins, or multicast. Direct
//! mirror of `crate::effects::registry::ExecutorRegistry`.

use std::collections::HashMap;
use std::sync::Arc;

use thiserror::Error;

use super::provider::ExternalDataProvider;

/// Registration failed because `provider_id` already has an owner.
#[derive(Debug, Error)]
pub enum DuplicateProviderId {
    /// The `provider_id` that already has a registered provider.
    #[error("provider already registered for provider_id '{0}'")]
    AlreadyRegistered(String),
}

/// Maps each `provider_id` to its sole registered provider.
#[derive(Default, Clone)]
pub struct ExternalDataProviderRegistry {
    providers: HashMap<String, Arc<dyn ExternalDataProvider>>,
}

impl ExternalDataProviderRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether no provider has been registered at all (spec: "Zero Runtime
    /// Overhead When Unused" — a builder never constructs the facade/
    /// chokepoint when this is `true`).
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Registers `provider` as the sole owner of `provider_id`.
    ///
    /// Fails if another provider already owns this `provider_id`; the first
    /// registration remains untouched.
    pub fn register(
        &mut self,
        provider_id: impl Into<String>,
        provider: Arc<dyn ExternalDataProvider>,
    ) -> Result<(), DuplicateProviderId> {
        let provider_id = provider_id.into();
        if self.providers.contains_key(&provider_id) {
            return Err(DuplicateProviderId::AlreadyRegistered(provider_id));
        }
        self.providers.insert(provider_id, provider);
        Ok(())
    }

    /// Looks up the registered provider for `provider_id`, if any.
    pub fn get(&self, provider_id: &str) -> Option<Arc<dyn ExternalDataProvider>> {
        self.providers.get(provider_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::provider::ExternalDataProvider;
    use async_trait::async_trait;
    use persistent_entity::data_provider_access::{DataProviderError, DataRequest, DataResponse};
    use std::sync::Arc;

    struct NoopProvider;

    #[async_trait]
    impl ExternalDataProvider for NoopProvider {
        async fn fetch(&self, _request: DataRequest) -> Result<DataResponse, DataProviderError> {
            Ok(DataResponse {
                payload: vec![],
                cache_hit: false,
            })
        }
    }

    #[test]
    fn duplicate_registration_for_same_provider_id_fails() {
        let mut registry = ExternalDataProviderRegistry::new();
        registry
            .register("pricing", Arc::new(NoopProvider))
            .unwrap();

        let err = registry
            .register("pricing", Arc::new(NoopProvider))
            .unwrap_err();

        assert!(matches!(&err, DuplicateProviderId::AlreadyRegistered(id) if id == "pricing"));
        // The first registration remains the sole owner.
        assert!(registry.get("pricing").is_some());
    }

    #[test]
    fn distinct_provider_ids_register_and_resolve_independently() {
        let mut registry = ExternalDataProviderRegistry::new();

        registry
            .register("pricing", Arc::new(NoopProvider))
            .unwrap();
        registry.register("jwks", Arc::new(NoopProvider)).unwrap();

        assert!(registry.get("pricing").is_some());
        assert!(registry.get("jwks").is_some());
    }

    /// "Explicit, Non-Reflective Registration" scenario: a provider type
    /// exists (compiles) but was never registered — resolution fails
    /// exactly as an unregistered key would.
    #[test]
    fn compiled_but_unregistered_provider_id_resolves_to_none() {
        let registry = ExternalDataProviderRegistry::new();
        assert!(registry.get("never-registered").is_none());
    }
}
