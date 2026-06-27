//! Runtime state shared by generated service proxies.
//!
//! `RuntimeInner` is the shared state held by all generated proxies via
//! `Weak<RuntimeInner>`. It is a façade over smaller internal structs that
//! each own a distinct responsibility.
//!
//! NOTE: `RuntimeBuilder`, `Runtime`, graph validation, and tenant enforcement
//! are deferred to TASK-013 / TASK-014. This module only contains what the
//! `#[service]` macro (TASK-009/010/011) needs at compile time.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;

use crate::context::ServiceContext;
use crate::di::{AdapterRef, ConfigValue, ProjectionRef};
use crate::interceptor::InterceptorChain;
use crate::registry::ServiceRegistry;
use super::permit::CrossTenantPermit;

// ---------------------------------------------------------------------------
// Internal: grouped resolved-instance tables
// ---------------------------------------------------------------------------

/// Owns the resolved instances for all three dependency kinds.
///
/// Kept as a private field of `RuntimeInner` so the three maps are
/// packaged together rather than scattered across the parent struct.
#[derive(Debug)]
struct DependencyTable {
    projections: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    adapters: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    configs: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl DependencyTable {
    fn new() -> Self {
        Self {
            projections: HashMap::new(),
            adapters: HashMap::new(),
            configs: HashMap::new(),
        }
    }

    fn resolve_projection<T: 'static + Send + Sync>(
        &self,
    ) -> Result<ProjectionRef<T>, RuntimeError> {
        self.projections
            .get(&TypeId::of::<T>())
            .and_then(|arc| arc.clone().downcast::<T>().ok())
            .map(ProjectionRef::new)
            .ok_or(RuntimeError::DependencyNotFound)
    }

    fn resolve_adapter<A: 'static + Send + Sync>(&self) -> Result<AdapterRef<A>, RuntimeError> {
        self.adapters
            .get(&TypeId::of::<A>())
            .and_then(|arc| arc.clone().downcast::<A>().ok())
            .map(AdapterRef::new)
            .ok_or(RuntimeError::DependencyNotFound)
    }

    fn resolve_config<C: 'static + Send + Sync>(&self) -> Result<ConfigValue<C>, RuntimeError> {
        self.configs
            .get(&TypeId::of::<C>())
            .and_then(|arc| arc.clone().downcast::<C>().ok())
            .map(ConfigValue::new)
            .ok_or(RuntimeError::DependencyNotFound)
    }
}

// ---------------------------------------------------------------------------
// Shared runtime state
// ---------------------------------------------------------------------------

/// Shared state held by all generated proxies via `Weak<RuntimeInner>`.
///
/// This is a façade that delegates to smaller internal structs:
///
/// | Responsibility          | Owned by                    |
/// |-------------------------|-----------------------------|
/// | service registry        | `registry`                  |
/// | interceptor chain       | `interceptor_chain`         |
/// | resolved DI instances   | `resolved` (DependencyTable) |
/// | tenant enforcement      | `enforce_tenant()` method   |
///
/// The `RuntimeBuilder` (TASK-013) will construct this struct with
/// registered instances. Until then, the resolve methods return
/// `DependencyNotFound`.
pub struct RuntimeInner {
    // Populated by RuntimeBuilder (TASK-013); not yet read within this crate.
    #[allow(dead_code)]
    pub(crate) registry: ServiceRegistry,
    #[allow(dead_code)]
    pub(crate) interceptor_chain: Arc<InterceptorChain>,
    /// Optional security providers (authn + authz) installed via RuntimeBuilder.
    pub(crate) security_providers:
        Option<(Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>)>,
    /// Resolved instances for projection, adapter, and config injection.
    resolved: DependencyTable,
}

impl std::fmt::Debug for RuntimeInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeInner")
            .field("registry", &self.registry)
            .field("interceptor_chain", &self.interceptor_chain)
            .field("resolved", &self.resolved)
            .finish_non_exhaustive()
    }
}

impl RuntimeInner {
    /// Creates a new `RuntimeInner`.
    ///
    /// # TASK-014 note
    ///
    /// Once the runtime authorization check is active inside
    /// `issue_cross_tenant_permit`, consider restricting `RuntimeInner`
    /// construction to `pub(crate)` or forcing it through `RuntimeBuilder`
    /// to prevent rogue instances with custom `security_providers` from
    /// bypassing the authorization check.
    pub fn new(
        registry: ServiceRegistry,
        interceptor_chain: Arc<InterceptorChain>,
        security_providers: Option<
            (Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>),
        >,
    ) -> Self {
        Self {
            registry,
            interceptor_chain,
            security_providers,
            resolved: DependencyTable::new(),
        }
    }

    /// Resolves a registered `ProjectionRef<T>` by type.
    ///
    /// Returns `DependencyNotFound` if no instance was registered for `T`.
    pub fn resolve_projection<T: 'static + Send + Sync>(
        &self,
    ) -> Result<ProjectionRef<T>, RuntimeError> {
        self.resolved.resolve_projection::<T>()
    }

    /// Resolves a registered `AdapterRef<A>` by type.
    ///
    /// Returns `DependencyNotFound` if no instance was registered for `A`.
    pub fn resolve_adapter<A: 'static + Send + Sync>(&self) -> Result<AdapterRef<A>, RuntimeError> {
        self.resolved.resolve_adapter::<A>()
    }

    /// Resolves a registered `ConfigValue<C>` by type.
    ///
    /// Returns `DependencyNotFound` if no instance was registered for `C`.
    pub fn resolve_config<C: 'static + Send + Sync>(&self) -> Result<ConfigValue<C>, RuntimeError> {
        self.resolved.resolve_config::<C>()
    }

    /// Returns the configured authorization provider, if any.
    ///
    /// # Accessibility contract (AD-7)
    ///
    /// This accessor is `pub` solely to satisfy Rust's visibility rules for
    /// code generated by proc-macros. It is **not** part of the application
    /// programming model — application code must not call it directly. Any
    /// future public accessor on `RuntimeInner` requires an explicit ADR.
    pub fn authorization_provider(&self) -> Option<Arc<dyn AuthorizationProvider>> {
        self.security_providers
            .as_ref()
            .map(|(_, authz)| Arc::clone(authz))
    }

    /// No-op stub — runtime tenant enforcement is pending TASK-014.
    pub fn enforce_tenant(&self, _ctx: &ServiceContext) {}

    /// Mints a cross-tenant permit. No-op authorization today; TASK-014 will run
    /// the AuthorizationProvider check here and change this to a fallible signature.
    ///
    /// Compile-time gate only. TASK-014 adds the runtime authorization check.
    // SAFETY: must remain pub(crate) — widening to pub would let external crates
    // mint CrossTenantPermit without authorization. TASK-014 changes the body and
    // signature, not the visibility.
    // Used only in tests until TASK-014 wires up the runtime authorization check.
    #[allow(dead_code)]
    pub(crate) fn issue_cross_tenant_permit(&self) -> CrossTenantPermit {
        CrossTenantPermit::new()
    }
}

impl Default for RuntimeInner {
    // TASK-014: see the note on RuntimeInner::new() — once the authorization
    // check is active, Default may also need to be restricted or removed to
    // prevent rogue instances with no security_providers.
    fn default() -> Self {
        Self {
            registry: ServiceRegistry::new(),
            interceptor_chain: Arc::new(InterceptorChain::new()),
            security_providers: None,
            resolved: DependencyTable::new(),
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

    // -- DependencyTable unit tests -----------------------------------------

    #[test]
    fn dependency_table_new_is_empty() {
        let t = DependencyTable::new();
        assert!(t.projections.is_empty());
        assert!(t.adapters.is_empty());
        assert!(t.configs.is_empty());
    }

    // -- Missing registration (TypeId not found) ----------------------------

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

    // -- Successful downcast -------------------------------------------------

    /// Stub type for downcast testing.
    #[derive(Debug, PartialEq)]
    struct MyProjection(u32);

    #[test]
    fn resolve_projection_succeeds_for_registered_type() {
        let mut rt = RuntimeInner::default();
        let instance = Arc::new(MyProjection(42)) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .projections
            .insert(TypeId::of::<MyProjection>(), instance);

        let result = rt.resolve_projection::<MyProjection>();
        assert!(result.is_ok());
        assert_eq!(*result.unwrap(), MyProjection(42));
    }

    #[test]
    fn resolve_adapter_succeeds_for_registered_type() {
        let mut rt = RuntimeInner::default();
        let instance = Arc::new(MyProjection(99)) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .adapters
            .insert(TypeId::of::<MyProjection>(), instance);

        let result = rt.resolve_adapter::<MyProjection>();
        assert!(result.is_ok());
        assert_eq!(*result.unwrap(), MyProjection(99));
    }

    #[test]
    fn resolve_config_succeeds_for_registered_type() {
        let mut rt = RuntimeInner::default();
        let instance = Arc::new(String::from("config-value")) as Arc<dyn Any + Send + Sync>;
        rt.resolved.configs.insert(TypeId::of::<String>(), instance);

        let result = rt.resolve_config::<String>();
        assert!(result.is_ok());
        assert_eq!(*result.unwrap(), "config-value");
    }

    // -- Incorrect type downcast --------------------------------------------

    #[test]
    fn resolve_projection_returns_not_found_for_wrong_type() {
        let mut rt = RuntimeInner::default();
        // Register as String, request as MyProjection.
        let instance = Arc::new(String::from("not-a-projection")) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .projections
            .insert(TypeId::of::<String>(), instance);

        let result = rt.resolve_projection::<MyProjection>();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    #[test]
    fn resolve_adapter_returns_not_found_for_wrong_type() {
        let mut rt = RuntimeInner::default();
        let instance = Arc::new(String::from("not-an-adapter")) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .adapters
            .insert(TypeId::of::<String>(), instance);

        let result = rt.resolve_adapter::<MyProjection>();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    #[test]
    fn resolve_config_returns_not_found_for_wrong_type() {
        let mut rt = RuntimeInner::default();
        let instance = Arc::new(MyProjection(7)) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .configs
            .insert(TypeId::of::<MyProjection>(), instance);

        let result = rt.resolve_config::<String>();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    // -- Concurrent resolution -----------------------------------------------

    #[test]
    fn concurrent_resolution_succeeds() {
        let rt = Arc::new(RuntimeInner::default());

        let mut handles = Vec::new();
        for _ in 0..10 {
            let rt_clone = rt.clone();
            handles.push(std::thread::spawn(move || {
                let r1 = rt_clone.resolve_projection::<()>();
                let r2 = rt_clone.resolve_adapter::<()>();
                let r3 = rt_clone.resolve_config::<()>();
                (r1, r2, r3)
            }));
        }

        for h in handles {
            let (r1, r2, r3) = h.join().unwrap();
            assert!(matches!(r1, Err(RuntimeError::DependencyNotFound)));
            assert!(matches!(r2, Err(RuntimeError::DependencyNotFound)));
            assert!(matches!(r3, Err(RuntimeError::DependencyNotFound)));
        }
    }

    // -- Table of responsibility: DependencyTable fields ---------------------

    #[test]
    fn dependency_table_respects_kind_boundaries() {
        let mut t = DependencyTable::new();
        let val = Arc::new(42) as Arc<dyn Any + Send + Sync>;

        // Insert into projections — must NOT be resolvable via adapters or configs.
        t.projections.insert(TypeId::of::<i32>(), val.clone());
        assert!(t.resolve_projection::<i32>().is_ok());
        assert!(t.resolve_adapter::<i32>().is_err());
        assert!(t.resolve_config::<i32>().is_err());

        t.adapters.insert(TypeId::of::<i32>(), val.clone());
        assert!(t.resolve_adapter::<i32>().is_ok());

        t.configs.insert(TypeId::of::<i32>(), val);
        assert!(t.resolve_config::<i32>().is_ok());
    }

    // -- CrossTenantPermit issuer (S-2) ------------------------------------

    #[test]
    fn runtime_inner_issues_cross_tenant_permit() {
        let inner = RuntimeInner::default();
        let _permit = inner.issue_cross_tenant_permit();
    }

    // -- authorization_provider accessor (CORE-015 / AC-10) ----------------

    #[test]
    fn authorization_provider_returns_none_when_no_providers() {
        let rt = RuntimeInner::default();
        assert!(
            rt.authorization_provider().is_none(),
            "Expected None when security_providers is None"
        );
    }

    #[test]
    fn authorization_provider_returns_arc_when_providers_set() {
        use ego_security_sdk::authentication::AuthenticationProvider;
        use ego_security_sdk::authorization::AuthorizationProvider;
        use ego_security_sdk::principal::Principal;
        use ego_security_sdk::{AccessRequest, AuthorizationDecision, SecurityError};
        use async_trait::async_trait;
        use ego_security_sdk::context::SecurityContext;
        use ego_security_sdk::credential::Credential;
        use ego_security_sdk::AuthenticationError;

        struct StubAuthn;

        impl AuthenticationProvider for StubAuthn {
            fn authenticate(&self, _: &Credential) -> Result<SecurityContext, AuthenticationError> {
                unimplemented!("stub")
            }
        }

        struct StubAuthz;

        #[async_trait]
        impl AuthorizationProvider for StubAuthz {
            async fn authorize(
                &self,
                _: &Principal,
                _: &AccessRequest,
                _: &SecurityContext,
            ) -> Result<AuthorizationDecision, SecurityError> {
                unimplemented!("stub")
            }
        }

        let authn: Arc<dyn AuthenticationProvider> = Arc::new(StubAuthn);
        let authz: Arc<dyn AuthorizationProvider> = Arc::new(StubAuthz);
        let authz_ptr = Arc::as_ptr(&authz);

        let rt = RuntimeInner::new(
            ServiceRegistry::new(),
            Arc::new(InterceptorChain::new()),
            Some((authn, authz)),
        );

        let result = rt.authorization_provider();
        assert!(result.is_some(), "Expected Some when providers are set");
        assert_eq!(
            Arc::as_ptr(result.as_ref().unwrap()),
            authz_ptr,
            "Returned Arc must point to the same AuthorizationProvider"
        );
    }
}
