use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;
use kitlogger::KITLogger;

use crate::interceptor::InterceptorChain;
use crate::registry::ServiceRegistry;
use crate::runtime::logger::TeardownStack;
use crate::runtime::runtime_builder::{DependencyTable, RuntimeInner};
use crate::runtime::tenant::{TenantEnforcementMode, TenantResolver};
use crate::runtime::RuntimeInfraError;

/// The pair of security providers registered with a [`Runtime`].
pub type SecurityProviders = (Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>);

/// Builder for constructing a [`Runtime`] with optional security providers.
///
/// `RuntimeBuilder` has no configurable scalar fields of its own beyond
/// [`RuntimeBuilder::with_tenant_enforcement_mode`] (CORE-008A) — all other
/// runtime tunables (mailbox capacity, concurrency budget, passivation
/// timeout, and the **persistence-side** `single_tenant_mode` / `tenant_id`)
/// belong to the entity-level runtime and are configured via
/// [`persistent_entity::EntityRuntimeBuilder::from_value`]. Pass a
/// `serde_json::Value` obtained from `kit_config::ConfigLoader` to that
/// builder, not to this one.
///
/// **Naming disambiguation (CORE-008A AD-012):** the persistence-side tenant
/// mode above (`EntityRuntimeBuilder`, CORE-016) is a distinct concept from
/// the enforcement-side [`TenantEnforcementMode`] configured here via
/// [`RuntimeBuilder::with_tenant_enforcement_mode`]. Neither this builder nor
/// its docs reuse the bare phrase "tenant mode" for the enforcement concept.
pub struct RuntimeBuilder {
    registry: ServiceRegistry,
    interceptor_chain: Arc<InterceptorChain>,
    authn: Option<Arc<dyn AuthenticationProvider>>,
    authz: Option<Arc<dyn AuthorizationProvider>>,
    logger: Option<Arc<KITLogger>>,
    adapters: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    configs: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    tenant_enforcement_mode: TenantEnforcementMode,
}

impl RuntimeBuilder {
    /// Creates a new `RuntimeBuilder` with default (empty) configuration.
    pub fn new() -> Self {
        Self {
            registry: ServiceRegistry::new(),
            interceptor_chain: Arc::new(InterceptorChain::new()),
            authn: None,
            authz: None,
            logger: None,
            adapters: HashMap::new(),
            configs: HashMap::new(),
            tenant_enforcement_mode: TenantEnforcementMode::AuthenticatedOnly,
        }
    }

    /// Registers authentication and authorization providers for this runtime.
    ///
    /// The providers are stored and exposed via [`Runtime::security_providers`].
    /// The runtime does not automatically enforce authentication — callers are
    /// responsible for invoking the provider and populating `ServiceContext` on each request.
    pub fn with_security(
        self,
        authn: Arc<dyn AuthenticationProvider>,
        authz: Arc<dyn AuthorizationProvider>,
    ) -> Self {
        Self {
            authn: Some(authn),
            authz: Some(authz),
            ..self
        }
    }

    /// Registers a fully-constructed logger for this runtime.
    ///
    /// Mirrors [`RuntimeBuilder::with_security`]: the logger is constructed and
    /// initialized by the **host** (via `build_logger`), before `RuntimeBuilder::new()`
    /// is ever called (CORE-016). `RuntimeBuilder` never constructs it — it only
    /// takes ownership and registers it for ordered teardown on [`Runtime::shutdown`].
    pub fn with_logger(mut self, logger: Arc<KITLogger>) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Registers a host-constructed adapter, resolvable via `resolve_adapter::<A>()`.
    /// Last-write-wins: registering another value of the same concrete type `A`
    /// REPLACES the previous one; only the most recent value per type is retained.
    pub fn with_adapter<A: Send + Sync + 'static>(mut self, adapter: Arc<A>) -> Self {
        self.adapters.insert(TypeId::of::<A>(), adapter as Arc<dyn Any + Send + Sync>);
        self
    }

    /// Registers a host-constructed config value, resolvable via `resolve_config::<C>()`.
    /// Last-write-wins (same semantics as `with_adapter`). CORE-016: accepts only an
    /// already-constructed `Arc<C>`, never a raw config source/loader.
    pub fn with_config<C: Send + Sync + 'static>(mut self, value: Arc<C>) -> Self {
        self.configs.insert(TypeId::of::<C>(), value as Arc<dyn Any + Send + Sync>);
        self
    }

    /// Registers the tenant enforcement policy for this runtime (CORE-008A
    /// AD-012). Default is [`TenantEnforcementMode::AuthenticatedOnly`] —
    /// unauthenticated tenant-scoped calls fail closed with `MissingContext`.
    /// Selected once at construction; there is no setter to change it after
    /// `build()` — construct a new `Runtime` with a different mode instead.
    ///
    /// Distinct from the persistence-side tenant mode on
    /// `persistent_entity::EntityRuntimeBuilder` (CORE-016) — see this
    /// struct's type-level doc.
    pub fn with_tenant_enforcement_mode(mut self, mode: TenantEnforcementMode) -> Self {
        self.tenant_enforcement_mode = mode;
        self
    }

    /// Consumes the builder and produces a [`Runtime`].
    ///
    /// Always succeeds — security and the logger are both optional. By the
    /// time this runs, the logger (if supplied) is already constructed and
    /// initialized by the host; this only pushes it onto the teardown stack.
    pub fn build(self) -> Runtime {
        let security_providers = match (self.authn, self.authz) {
            (Some(authn), Some(authz)) => Some((authn, authz)),
            _ => None,
        };
        let mut teardown = TeardownStack::new();
        if let Some(logger) = &self.logger {
            teardown.push(logger.clone());
        }
        Runtime {
            inner: Arc::new(RuntimeInner::new_with_logger(
                self.registry,
                self.interceptor_chain,
                security_providers,
                DependencyTable::with_registrations(self.adapters, self.configs),
                self.logger,
                Mutex::new(teardown),
                TenantResolver::new(self.tenant_enforcement_mode),
            )),
        }
    }
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A configured runtime handle wrapping shared [`RuntimeInner`] state.
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl Runtime {
    /// Returns a reference to the inner [`RuntimeInner`].
    pub fn inner(&self) -> &Arc<RuntimeInner> {
        &self.inner
    }

    /// Returns the registered security providers, if any.
    pub fn security_providers(
        &self,
    ) -> Option<&SecurityProviders> {
        self.inner.security_providers.as_ref()
    }

    /// Returns the registered logger, if any.
    pub fn logger(&self) -> Option<&Arc<KITLogger>> {
        self.inner.logger()
    }

    /// Drains initialized infrastructure in reverse construction order.
    ///
    /// For the console exporter, `shutdown()` flushes (`OnShutdownFlush`) then
    /// closes. Idempotent: a second call on an already-drained stack returns
    /// `Ok(())`. A poisoned lock (only possible if a prior `shutdown()`
    /// panicked mid-drain) is treated as a hard error rather than silently
    /// recovered — consistent with "no degraded mode."
    pub fn shutdown(&self) -> Result<(), RuntimeInfraError> {
        self.inner
            .teardown
            .lock()
            .expect("teardown mutex poisoned")
            .drain()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ego_security_sdk::authentication::AuthenticationProvider;
    use ego_security_sdk::authorization::{
        AuthorizationDecision, AuthorizationProvider,
    };
    use ego_security_sdk::context::SecurityContext;
    use ego_security_sdk::credential::Credential;
    use ego_security_sdk::error::SecurityError;
    use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};
    use ego_security_sdk::AuthenticationError;
    use kitlogger::KITLogger;

    use super::{Runtime, RuntimeBuilder};
    use crate::runtime::RuntimeError;

    struct StubAuthn;

    impl AuthenticationProvider for StubAuthn {
        fn authenticate(&self, _credential: &Credential) -> Result<SecurityContext, AuthenticationError> {
            let subject = SubjectId::new("user:stub").unwrap();
            let principal = Principal::new(PrincipalKind::User, subject);
            Ok(SecurityContext::empty(principal))
        }
    }

    struct StubAuthz;

    #[async_trait]
    impl AuthorizationProvider for StubAuthz {
        async fn authorize(
            &self,
            _principal: &Principal,
            _request: &ego_security_sdk::authorization::AccessRequest,
            _ctx: &ego_security_sdk::context::SecurityContext,
        ) -> Result<AuthorizationDecision, SecurityError> {
            Ok(AuthorizationDecision::Allow)
        }
    }

    #[test]
    fn build_without_security_succeeds() {
        let rt = RuntimeBuilder::new().build();
        assert!(rt.security_providers().is_none());
    }

    #[test]
    fn build_with_security_succeeds() {
        let rt = RuntimeBuilder::new()
            .with_security(Arc::new(StubAuthn), Arc::new(StubAuthz))
            .build();
        assert!(rt.security_providers().is_some());
    }

    #[test]
    fn runtime_inner_is_accessible() {
        let rt = RuntimeBuilder::new().build();
        let _inner = rt.inner();
    }

    #[test]
    fn runtime_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Runtime>();
    }

    // -- CORE-017: logger wiring ---------------------------------------

    #[test]
    fn build_without_logger_has_none_logger() {
        let rt = RuntimeBuilder::new().build();
        assert!(rt.logger().is_none());
    }

    /// Real host bootstrap always calls `KITLogger::init()` before handing the
    /// logger to `.with_logger(..)` (that's `build_logger`'s job). Tests that
    /// exercise `Runtime::shutdown()` need an initialized logger too — an
    /// un-initialized `KITLogger`'s exporter is `Uninitialized`, and its
    /// `shutdown()` is an invalid lifecycle transition (`Uninitialized ->
    /// Flushing`), so it would fail for reasons unrelated to what these tests
    /// verify.
    fn initialized_logger() -> Arc<KITLogger> {
        let logger = KITLogger::default();
        logger.init().expect("logger initializes");
        Arc::new(logger)
    }

    #[test]
    fn build_with_logger_has_some_logger() {
        let rt = RuntimeBuilder::new()
            .with_logger(Arc::new(KITLogger::default()))
            .build();
        assert!(rt.logger().is_some());
    }

    #[test]
    fn shutdown_with_logger_succeeds_and_is_idempotent() {
        let rt = RuntimeBuilder::new().with_logger(initialized_logger()).build();
        assert!(rt.shutdown().is_ok());
        assert!(rt.shutdown().is_ok());
    }

    #[test]
    fn shutdown_without_logger_succeeds() {
        let rt = RuntimeBuilder::new().build();
        assert!(rt.shutdown().is_ok());
    }

    /// Ownership test: asserts the *contract* — something else holds the
    /// logger after `.build()`, and `.shutdown()` releases at least one of
    /// those references — not an exact count, so this doesn't couple to
    /// `TeardownStack`'s current `Vec<Arc<_>>` shape.
    ///
    /// Deviation from tasks.md's literal TASK-016 wording, documented here:
    /// tasks.md describes the post-shutdown count as "== 1 (back to only the
    /// test's reference)". Tracing design.md's own frozen `build()` snippet
    /// shows that's not achievable as written: `RuntimeInner` retains its own
    /// permanent `logger: Option<Arc<KITLogger>>` field (for the `.logger()`
    /// accessor) *in addition to* the separate clone `TeardownStack` holds
    /// for ordered teardown — two independent owners by design (File Changes:
    /// "RuntimeInner gains `logger: Option<Arc<KITLogger>>` + `Mutex<TeardownStack>`").
    /// `shutdown()` only drains the stack's clone; `RuntimeInner.logger` keeps
    /// its own reference alive after shutdown so the accessor keeps working.
    /// So the true post-shutdown count is 2 (test's + `RuntimeInner.logger`),
    /// not 1. This test asserts the part of the contract that actually holds:
    /// shutdown strictly reduces the count from its post-build value.
    #[test]
    fn shutdown_releases_teardown_stack_ownership_of_logger() {
        let logger = initialized_logger();
        let rt = RuntimeBuilder::new().with_logger(logger.clone()).build();

        let count_after_build = Arc::strong_count(&logger);
        assert!(
            count_after_build > 1,
            "the runtime should now also hold at least one reference"
        );

        rt.shutdown().expect("shutdown succeeds");

        let count_after_shutdown = Arc::strong_count(&logger);
        assert!(
            count_after_shutdown < count_after_build,
            "shutdown must release the teardown stack's own reference"
        );
    }

    // -- CORE-120: with_adapter / with_config -------------------------------

    #[derive(Debug, PartialEq)]
    struct StubAdapter(u32);

    #[derive(Debug, PartialEq)]
    struct StubConfig(String);

    #[test]
    fn with_adapter_registers_and_resolves() {
        let rt = RuntimeBuilder::new()
            .with_adapter(Arc::new(StubAdapter(7)))
            .build();

        let resolved = rt.inner().resolve_adapter::<StubAdapter>();
        assert!(resolved.is_ok());
        assert_eq!(*resolved.unwrap(), StubAdapter(7));
    }

    #[test]
    fn with_config_registers_and_resolves() {
        let rt = RuntimeBuilder::new()
            .with_config(Arc::new(StubConfig("hello".to_string())))
            .build();

        let resolved = rt.inner().resolve_config::<StubConfig>();
        assert!(resolved.is_ok());
        assert_eq!(*resolved.unwrap(), StubConfig("hello".to_string()));
    }

    #[test]
    fn with_adapter_last_write_wins() {
        let rt = RuntimeBuilder::new()
            .with_adapter(Arc::new(StubAdapter(1)))
            .with_adapter(Arc::new(StubAdapter(2)))
            .build();

        let resolved = rt.inner().resolve_adapter::<StubAdapter>().unwrap();
        assert_eq!(*resolved, StubAdapter(2));
    }

    #[test]
    fn with_config_last_write_wins() {
        let rt = RuntimeBuilder::new()
            .with_config(Arc::new(StubConfig("first".to_string())))
            .with_config(Arc::new(StubConfig("second".to_string())))
            .build();

        let resolved = rt.inner().resolve_config::<StubConfig>().unwrap();
        assert_eq!(*resolved, StubConfig("second".to_string()));
    }

    // -- CORE-120: chained registration --------------------------------------

    #[derive(Debug, PartialEq)]
    struct StubAdapterB(u32);

    #[derive(Debug, PartialEq)]
    struct StubConfigD(String);

    #[test]
    fn chained_registration_multiple_types() {
        let rt = RuntimeBuilder::new()
            .with_adapter(Arc::new(StubAdapter(1)))
            .with_config(Arc::new(StubConfig("c".to_string())))
            .with_adapter(Arc::new(StubAdapterB(2)))
            .with_config(Arc::new(StubConfigD("d".to_string())))
            .build();

        assert_eq!(*rt.inner().resolve_adapter::<StubAdapter>().unwrap(), StubAdapter(1));
        assert_eq!(*rt.inner().resolve_adapter::<StubAdapterB>().unwrap(), StubAdapterB(2));
        assert_eq!(*rt.inner().resolve_config::<StubConfig>().unwrap(), StubConfig("c".to_string()));
        assert_eq!(*rt.inner().resolve_config::<StubConfigD>().unwrap(), StubConfigD("d".to_string()));
    }

    // -- CORE-120: unregistered type unchanged behavior ----------------------

    #[test]
    fn resolve_adapter_unregistered_returns_dependency_not_found() {
        let rt = RuntimeBuilder::new().build();
        let result = rt.inner().resolve_adapter::<StubAdapter>();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    #[test]
    fn resolve_config_unregistered_returns_dependency_not_found() {
        let rt = RuntimeBuilder::new().build();
        let result = rt.inner().resolve_config::<StubConfig>();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound)));
    }

    // -- CORE-120: identity preservation (no clone-on-resolve) ---------------

    #[test]
    fn with_adapter_preserves_arc_identity() {
        let original = Arc::new(StubAdapter(7));
        let rt = RuntimeBuilder::new().with_adapter(original.clone()).build();

        let resolved = rt.inner().resolve_adapter::<StubAdapter>().unwrap();
        assert!(
            std::ptr::eq(&*original, &*resolved),
            "resolve_adapter must return the exact registered instance, not a clone"
        );
    }

    #[test]
    fn with_config_preserves_arc_identity() {
        let original = Arc::new(StubConfig("hello".to_string()));
        let rt = RuntimeBuilder::new().with_config(original.clone()).build();

        let resolved = rt.inner().resolve_config::<StubConfig>().unwrap();
        assert!(
            std::ptr::eq(&*original, &*resolved),
            "resolve_config must return the exact registered instance, not a clone"
        );
    }

    // -- CORE-120: adapter/config namespace isolation ------------------------

    #[derive(Debug, PartialEq)]
    struct SharedType(u32);

    #[test]
    fn adapter_and_config_of_same_concrete_type_do_not_collide() {
        let rt = RuntimeBuilder::new()
            .with_adapter(Arc::new(SharedType(1)))
            .with_config(Arc::new(SharedType(2)))
            .build();

        assert_eq!(*rt.inner().resolve_adapter::<SharedType>().unwrap(), SharedType(1));
        assert_eq!(*rt.inner().resolve_config::<SharedType>().unwrap(), SharedType(2));
    }
}
