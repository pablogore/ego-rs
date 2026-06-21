//! Runtime state shared by generated service proxies.
//!
//! `RuntimeInner` is the shared state held by all generated proxies via
//! `Weak<RuntimeInner>`. It owns projection, adapter, and config instances
//! for dependency injection.
//!
//! NOTE: `RuntimeBuilder`, `Runtime`, graph validation, and tenant enforcement
//! are deferred to TASK-013 / TASK-014. This module only contains what the
//! `#[service]` macro (TASK-009/010/011) needs at compile time.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use crate::context::ServiceContext;
use crate::di::{AdapterRef, ConfigValue, ProjectionRef};
use crate::interceptor::InterceptorChain;
use crate::registry::ServiceRegistry;

// ---------------------------------------------------------------------------
// Shared runtime state
// ---------------------------------------------------------------------------

/// Shared state held by all generated proxies via `Weak<RuntimeInner>`.
///
/// The `RuntimeBuilder` (TASK-013) is responsible for constructing this
/// struct with registered instances. Until then, the resolve methods return
/// `DependencyNotFound` when no instances have been registered.
#[derive(Debug)]
pub struct RuntimeInner {
    /// The type-keyed service registry holding raw implementations.
    pub registry: ServiceRegistry,
    /// The interceptor chain applied to every resolved proxy.
    pub interceptor_chain: Arc<InterceptorChain>,
    /// Registered projection instances for dependency injection.
    projection_instances: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Registered adapter instances for dependency injection.
    adapter_instances: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Registered config instances for dependency injection.
    config_instances: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl RuntimeInner {
    /// Creates a new `RuntimeInner`.
    pub fn new(registry: ServiceRegistry, interceptor_chain: Arc<InterceptorChain>) -> Self {
        Self {
            registry,
            interceptor_chain,
            projection_instances: HashMap::new(),
            adapter_instances: HashMap::new(),
            config_instances: HashMap::new(),
        }
    }

    /// Resolves a registered `ProjectionRef<T>` by type.
    ///
    /// Returns `DependencyNotFound` if no instance was registered for `T`.
    pub fn resolve_projection<T: 'static + Send + Sync>(
        &self,
    ) -> Result<ProjectionRef<T>, RuntimeError> {
        self.projection_instances
            .get(&TypeId::of::<T>())
            .and_then(|arc| arc.clone().downcast::<T>().ok())
            .map(ProjectionRef::new)
            .ok_or(RuntimeError::DependencyNotFound)
    }

    /// Resolves a registered `AdapterRef<A>` by type.
    ///
    /// Returns `DependencyNotFound` if no instance was registered for `A`.
    pub fn resolve_adapter<A: 'static + Send + Sync>(&self) -> Result<AdapterRef<A>, RuntimeError> {
        self.adapter_instances
            .get(&TypeId::of::<A>())
            .and_then(|arc| arc.clone().downcast::<A>().ok())
            .map(AdapterRef::new)
            .ok_or(RuntimeError::DependencyNotFound)
    }

    /// Resolves a registered `ConfigValue<C>` by type.
    ///
    /// Returns `DependencyNotFound` if no instance was registered for `C`.
    pub fn resolve_config<C: 'static + Send + Sync>(&self) -> Result<ConfigValue<C>, RuntimeError> {
        self.config_instances
            .get(&TypeId::of::<C>())
            .and_then(|arc| arc.clone().downcast::<C>().ok())
            .map(ConfigValue::new)
            .ok_or(RuntimeError::DependencyNotFound)
    }

    /// Enforces tenant isolation. Currently a no-op until TASK-014.
    pub fn enforce_tenant(&self, _ctx: &ServiceContext) {}
}

impl Default for RuntimeInner {
    fn default() -> Self {
        Self {
            registry: ServiceRegistry::new(),
            interceptor_chain: Arc::new(InterceptorChain::new()),
            projection_instances: HashMap::new(),
            adapter_instances: HashMap::new(),
            config_instances: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime errors
// ---------------------------------------------------------------------------

/// Errors that can occur during proxy resolution or dependency injection.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeError {
    /// The requested service was not found in the registry.
    ServiceNotFound,
    /// A dependency was not found during resolution.
    DependencyNotFound,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_inner_default_creates_empty() {
        let rt = RuntimeInner::default();
        assert!(matches!(
            rt.resolve_projection::<()>(),
            Err(RuntimeError::DependencyNotFound)
        ));
    }

    #[test]
    fn resolve_projection_returns_not_found_for_unregistered() {
        let rt = RuntimeInner::default();
        let result: Result<ProjectionRef<()>, RuntimeError> = rt.resolve_projection();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    #[test]
    fn resolve_adapter_returns_not_found_for_unregistered() {
        let rt = RuntimeInner::default();
        let result: Result<AdapterRef<()>, RuntimeError> = rt.resolve_adapter();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    #[test]
    fn resolve_config_returns_not_found_for_unregistered() {
        let rt = RuntimeInner::default();
        let result: Result<ConfigValue<()>, RuntimeError> = rt.resolve_config();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }
}
