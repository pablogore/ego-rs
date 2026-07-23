//! Executor registry, one owner per `effect_type` (CORE-019 Phase 2).
//!
//! Fail-closed at registration time (spec: "ExternalEffectExecutor Registry —
//! One Owner Per Type") — no last-wins, first-wins, or multicast. One
//! executor instance MAY be registered under more than one `effect_type` key.

use std::collections::HashMap;
use std::sync::Arc;

use thiserror::Error;

use super::executor::ExternalEffectExecutor;

/// Registration failed because `effect_type` already has an owner.
#[derive(Debug, Error)]
pub enum DuplicateEffectType {
    /// The `effect_type` that already has a registered executor.
    #[error("executor already registered for effect_type '{0}'")]
    AlreadyRegistered(String),
}

/// Maps each `effect_type` to its sole registered executor.
///
/// Builder-level sugar for registering one executor under several keys at
/// once (design.md §6.4) is out of scope for this slice; call
/// [`register`](Self::register) once per key with clones of the same `Arc`.
#[derive(Default, Clone)]
pub struct ExecutorRegistry {
    executors: HashMap<String, Arc<dyn ExternalEffectExecutor>>,
}

impl ExecutorRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether no executor has been registered at all (CORE-019 Phase 9,
    /// design.md §8/§20: this is the zero-cost gate — a builder never
    /// constructs a [`super::acceptor::RuntimeEffectAcceptor`]/store/queue/
    /// spawned drain task when this is `true`).
    pub fn is_empty(&self) -> bool {
        self.executors.is_empty()
    }

    /// Registers `executor` as the sole owner of `effect_type`.
    ///
    /// Fails if another executor already owns this `effect_type`; the first
    /// registration remains untouched.
    pub fn register(
        &mut self,
        effect_type: impl Into<String>,
        executor: Arc<dyn ExternalEffectExecutor>,
    ) -> Result<(), DuplicateEffectType> {
        let effect_type = effect_type.into();
        if self.executors.contains_key(&effect_type) {
            return Err(DuplicateEffectType::AlreadyRegistered(effect_type));
        }
        self.executors.insert(effect_type, executor);
        Ok(())
    }

    /// Looks up the registered executor for `effect_type`, if any.
    pub fn get(&self, effect_type: &str) -> Option<Arc<dyn ExternalEffectExecutor>> {
        self.executors.get(effect_type).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::executor::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
    use async_trait::async_trait;
    use ego_domain::ExternalEffectDescription;
    use std::sync::Arc;

    struct NoopExecutor;

    #[async_trait]
    impl ExternalEffectExecutor for NoopExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            AttemptOutcome::Success
        }
    }

    #[test]
    fn duplicate_registration_for_same_effect_type_fails() {
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(NoopExecutor))
            .unwrap();

        let err = registry
            .register("invoice.created", Arc::new(NoopExecutor))
            .unwrap_err();

        assert!(
            matches!(&err, DuplicateEffectType::AlreadyRegistered(t) if t == "invoice.created")
        );
        // The first registration remains the sole owner.
        assert!(registry.get("invoice.created").is_some());
    }

    #[test]
    fn one_executor_instance_may_own_multiple_effect_types() {
        let mut registry = ExecutorRegistry::new();
        let executor: Arc<dyn ExternalEffectExecutor> = Arc::new(NoopExecutor);

        registry.register("s3.put", executor.clone()).unwrap();
        registry.register("s3.delete", executor.clone()).unwrap();

        assert!(registry.get("s3.put").is_some());
        assert!(registry.get("s3.delete").is_some());
    }

    #[test]
    fn unregistered_effect_type_returns_none() {
        let registry = ExecutorRegistry::new();
        assert!(registry.get("unknown.type").is_none());
    }
}
