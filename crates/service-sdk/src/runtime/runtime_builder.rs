//! Runtime state shared by generated service proxies.
//!
//! `RuntimeInner` is the shared state held by all generated proxies via
//! `Weak<RuntimeInner>`. It is a façade over smaller internal structs that
//! each own a distinct responsibility.
//!
//! NOTE: graph validation and tenant enforcement remain deferred to a future
//! change. The config + logger bootstrap construction flow this note used to
//! defer is resolved by CORE-017: `RuntimeBuilder::build()` (via
//! `new_with_logger`) is now the canonical way to construct `RuntimeInner`
//! with an optional logger and its teardown stack.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;
use ego_security_sdk::error::SecurityError;
use kitlogger::KITLogger;

use crate::context::ServiceContext;
use crate::di::{AdapterRef, ConfigValue, ProjectionRef};
use crate::interceptor::InterceptorChain;
use crate::registry::ServiceRegistry;
use super::logger::TeardownStack;
use super::permit::CrossTenantPermit;
#[cfg(test)]
use super::tenant::TenantEnforcementMode;
use super::tenant::TenantResolver;

// ---------------------------------------------------------------------------
// Internal: grouped resolved-instance tables
// ---------------------------------------------------------------------------

/// Owns the resolved instances for all three dependency kinds.
///
/// Kept as a private field of `RuntimeInner` so the three maps are
/// packaged together rather than scattered across the parent struct.
#[derive(Debug)]
pub(super) struct DependencyTable {
    projections: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    adapters: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    configs: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl DependencyTable {
    #[cfg(test)]
    fn new() -> Self {
        Self {
            projections: HashMap::new(),
            adapters: HashMap::new(),
            configs: HashMap::new(),
        }
    }

    /// Builds a table from host-registered adapters/configs (`RuntimeBuilder`),
    /// with no registered projections. Takes both maps as named parameters so
    /// they can't be silently transposed at the call site.
    pub(super) fn with_registrations(
        adapters: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
        configs: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    ) -> Self {
        Self { projections: HashMap::new(), adapters, configs }
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
    /// Resolves the canonical tenant for enforcement (CORE-008A AD-001/AD-009).
    /// Built from the [`TenantEnforcementMode`] configured via
    /// `RuntimeBuilder::with_tenant_enforcement_mode` (AD-012).
    tenant_resolver: TenantResolver,
    /// The logger constructed by the host and registered via `RuntimeBuilder::with_logger`.
    logger: Option<Arc<KITLogger>>,
    /// Infrastructure teardown stack, drained in reverse construction order on shutdown.
    ///
    /// `RuntimeInner` is always shared via `Arc` (generated proxies hold
    /// `Weak<RuntimeInner>`), so `Runtime::shutdown(&self)` needs interior
    /// mutability to drain it.
    pub(super) teardown: Mutex<TeardownStack>,
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
    /// Creates a new `RuntimeInner` with a logger and its teardown stack.
    ///
    /// Called by `RuntimeBuilder::build()`. The logger (if any) is already
    /// constructed and initialized by the host before this runs (CORE-016);
    /// this constructor only takes ownership and wires it into the teardown
    /// stack for `Runtime::shutdown()`.
    ///
    /// # TASK-014 note
    ///
    /// This is the sole constructor (`pub(super)`, CORE-018b) — closing the
    /// external bypass that would have let rogue instances with custom
    /// `security_providers` skip the authorization check. TASK-014 itself —
    /// making `issue_cross_tenant_permit` run a real `AuthorizationProvider`
    /// check — is still pending.
    pub(super) fn new_with_logger(
        registry: ServiceRegistry,
        interceptor_chain: Arc<InterceptorChain>,
        security_providers: Option<
            (Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>),
        >,
        resolved: DependencyTable,
        logger: Option<Arc<KITLogger>>,
        teardown: Mutex<TeardownStack>,
        tenant_resolver: TenantResolver,
    ) -> Self {
        Self {
            registry,
            interceptor_chain,
            security_providers,
            resolved,
            logger,
            teardown,
            tenant_resolver,
        }
    }

    /// Returns the registered logger, if any.
    ///
    /// # Accessibility contract (macro-visibility)
    ///
    /// This accessor is `pub` solely to satisfy Rust's visibility rules for
    /// code generated by the `ego-service-sdk-macros` proc-macro crate, which
    /// is a separate crate and therefore cannot access `pub(crate)` items.
    /// Application code MUST NOT call this method directly — use
    /// [`crate::runtime::Runtime::logger`] instead. It is not part of the
    /// public programming model; `#[doc(hidden)]` prevents it from appearing
    /// in rustdoc.
    #[doc(hidden)]
    pub fn logger(&self) -> Option<&Arc<KITLogger>> {
        self.logger.as_ref()
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
    /// # Accessibility contract (macro-visibility)
    ///
    /// This accessor is `pub` solely to satisfy Rust's visibility rules for
    /// code generated by the `ego-service-sdk-macros` proc-macro crate, which
    /// is a separate crate and therefore cannot access `pub(crate)` items.
    /// Application code MUST NOT call this method directly — it is not part
    /// of the public programming model. `#[doc(hidden)]` prevents it from
    /// appearing in rustdoc.
    #[doc(hidden)]
    pub fn authorization_provider(&self) -> Option<&Arc<dyn AuthorizationProvider>> {
        self.security_providers.as_ref().map(|(_, authz)| authz)
    }

    /// Resolves and enforces the canonical tenant for this call (CORE-008A
    /// AD-009). On success, writes the resolved [`super::tenant::CanonicalTenant`]
    /// into `ctx` via `ctx.set_resolved_tenant` and returns `Ok(())`. On
    /// failure, returns `Err` without mutating `ctx`.
    ///
    /// Still inert as of Phase 2: no `#[operation]` is marked
    /// `#[tenant_scoped]` yet, so the macro's generated unmarked-path call
    /// discards this `Result` (see `service-sdk-macros` TASK-009B). Phase 3
    /// wires a fallible `?` call for `#[tenant_scoped]` operations.
    pub fn enforce_tenant(&self, ctx: &mut ServiceContext) -> Result<(), SecurityError> {
        let security = ctx.security();
        let hint = ctx.tenant_hint();
        let canonical = self.tenant_resolver.resolve(security, hint)?;
        ctx.set_resolved_tenant(canonical);
        Ok(())
    }

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

    /// Test-only fixture equivalent to the removed `Default` impl.
    ///
    /// Inherent methods CAN be `pub(crate)` (unlike a trait impl, whose
    /// visibility follows the public `Default` trait), so this closes the
    /// external `::default()` bypass while keeping in-crate tests terse.
    ///
    /// Routes through [`Self::new_with_logger`] — the same constructor
    /// `RuntimeBuilder::build()` uses — so tests built on this fixture
    /// exercise the same construction path production code does. Always
    /// yields `security_providers: None`; a test that needs
    /// `Some((authn, authz))` calls `Self::new_with_logger` directly with
    /// explicit providers (see `authorization_provider_returns_arc_when_providers_set`
    /// below).
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self::for_test_with_mode(TenantEnforcementMode::AuthenticatedOnly)
    }

    /// Test fixture variant that lets a test configure the tenant enforcement
    /// mode explicitly (mirrors `RuntimeBuilder::with_tenant_enforcement_mode`
    /// in production). `for_test()`'s default is `AuthenticatedOnly` — there
    /// is no separate, more permissive default for tests (AD-012).
    #[cfg(test)]
    pub(crate) fn for_test_with_mode(mode: TenantEnforcementMode) -> Self {
        Self::new_with_logger(
            ServiceRegistry::new(),
            Arc::new(InterceptorChain::new()),
            None,
            DependencyTable::with_registrations(HashMap::new(), HashMap::new()),
            None,
            Mutex::new(TeardownStack::new()),
            TenantResolver::new(mode),
        )
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
        let rt = RuntimeInner::for_test();
        assert!(matches!(
            rt.resolve_projection::<()>(),
            Err(RuntimeError::DependencyNotFound)
        ));
    }

    #[test]
    fn resolve_projection_returns_not_found_for_unregistered() {
        let rt = RuntimeInner::for_test();
        let result: Result<ProjectionRef<()>, RuntimeError> = rt.resolve_projection();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    #[test]
    fn resolve_adapter_returns_not_found_for_unregistered() {
        let rt = RuntimeInner::for_test();
        let result: Result<AdapterRef<()>, RuntimeError> = rt.resolve_adapter();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    #[test]
    fn resolve_config_returns_not_found_for_unregistered() {
        let rt = RuntimeInner::for_test();
        let result: Result<ConfigValue<()>, RuntimeError> = rt.resolve_config();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    // -- Successful downcast -------------------------------------------------

    /// Stub type for downcast testing.
    #[derive(Debug, PartialEq)]
    struct MyProjection(u32);

    #[test]
    fn resolve_projection_succeeds_for_registered_type() {
        let mut rt = RuntimeInner::for_test();
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
        let mut rt = RuntimeInner::for_test();
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
        let mut rt = RuntimeInner::for_test();
        let instance = Arc::new(String::from("config-value")) as Arc<dyn Any + Send + Sync>;
        rt.resolved.configs.insert(TypeId::of::<String>(), instance);

        let result = rt.resolve_config::<String>();
        assert!(result.is_ok());
        assert_eq!(*result.unwrap(), "config-value");
    }

    // -- Incorrect type downcast --------------------------------------------

    #[test]
    fn resolve_projection_returns_not_found_for_wrong_type() {
        let mut rt = RuntimeInner::for_test();
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
        let mut rt = RuntimeInner::for_test();
        let instance = Arc::new(String::from("not-an-adapter")) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .adapters
            .insert(TypeId::of::<String>(), instance);

        let result = rt.resolve_adapter::<MyProjection>();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    #[test]
    fn resolve_config_returns_not_found_for_wrong_type() {
        let mut rt = RuntimeInner::for_test();
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
        let rt = Arc::new(RuntimeInner::for_test());

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
        let inner = RuntimeInner::for_test();
        let _permit = inner.issue_cross_tenant_permit();
    }

    // -- CORE-008A Phase 2 (TASK-008): fallible enforce_tenant --------------

    use ego_security_sdk::context::SecurityContext;
    use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};

    fn ctx_with_tenant(tenant: Option<&str>) -> ServiceContext {
        let mut principal = Principal::new(PrincipalKind::User, SubjectId::new("alice").unwrap());
        principal.tenant_id = tenant.map(|t| t.to_string());
        let security = SecurityContext::empty(principal);
        ServiceContext::new().with_security(Arc::new(security))
    }

    #[test]
    fn enforce_tenant_ok_sets_canonical_tenant_on_resolvable_context() {
        let rt = RuntimeInner::for_test();
        let mut ctx = ctx_with_tenant(Some("tenant-a"));

        let result = rt.enforce_tenant(&mut ctx);

        assert!(result.is_ok());
        assert!(ctx.canonical_tenant().is_some());
    }

    #[test]
    fn enforce_tenant_err_leaves_canonical_tenant_unset_on_unresolvable_context() {
        let rt = RuntimeInner::for_test();
        // Unauthenticated (no security attached) + default AuthenticatedOnly mode -> MissingContext.
        let mut ctx = ServiceContext::new();

        let result = rt.enforce_tenant(&mut ctx);

        assert!(matches!(result, Err(SecurityError::MissingContext)));
        assert!(ctx.canonical_tenant().is_none());
    }

    #[test]
    fn enforce_tenant_default_mode_is_authenticated_only() {
        let rt = RuntimeInner::for_test();
        // No security, but a supplied hint via tenant_id — must still fail
        // closed under the default AuthenticatedOnly mode.
        let mut ctx = ServiceContext::new().with_tenant_id("tenant-z");

        let result = rt.enforce_tenant(&mut ctx);

        assert!(matches!(result, Err(SecurityError::MissingContext)));
    }

    #[test]
    fn with_tenant_enforcement_mode_allow_system_internal_changes_resolution() {
        let rt = RuntimeInner::for_test_with_mode(TenantEnforcementMode::AllowSystemInternal);
        // No security, but AllowSystemInternal + a supplied hint -> resolves.
        let mut ctx = ServiceContext::new().with_tenant_id("tenant-internal");

        let result = rt.enforce_tenant(&mut ctx);

        assert!(result.is_ok());
        assert!(ctx.canonical_tenant().is_some());
    }

    // -- authorization_provider accessor (CORE-015 / AC-10) ----------------

    #[test]
    fn authorization_provider_returns_none_when_no_providers() {
        let rt = RuntimeInner::for_test();
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

        let rt = RuntimeInner::new_with_logger(
            ServiceRegistry::new(),
            Arc::new(InterceptorChain::new()),
            Some((authn, authz)),
            DependencyTable::with_registrations(HashMap::new(), HashMap::new()),
            None,
            Mutex::new(TeardownStack::new()),
            TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly),
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
