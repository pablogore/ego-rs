use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;
use kitlogger::KITLogger;

use crate::contract::{ServiceContract, VersionConstraint};
use crate::di::Injectable;
use crate::interceptor::InterceptorChain;
use crate::registry::{RegistryError, ServiceRegistry};
use crate::runtime::logger::TeardownStack;
use crate::runtime::runtime_builder::{DependencyTable, RuntimeError, RuntimeInner};
use crate::runtime::tenant::{TenantEnforcementMode, TenantResolver};
use crate::runtime::{Resolvable, ResolvableContainer, RuntimeInfraError};

/// The pair of security providers registered with a [`Runtime`].
pub type SecurityProviders = (Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>);

/// A recorded `(service_name, S::validate)` pair for `with_injectable`/`try_build` (AD-3).
type ValidatorEntry = (&'static str, fn(&RuntimeInner) -> Result<(), RuntimeError>);

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
    /// `(service_name, S::validate)` pairs recorded via `with_injectable`.
    /// Read only by `try_build()`; has no effect on `build()` (AD-3).
    validators: Vec<ValidatorEntry>,
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
            validators: Vec::new(),
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

    /// Registers a service implementation under `Tag`, resolvable later via
    /// `Runtime::resolve::<Tag>()` (AD-1/AD-2, F-01). The version is always
    /// derived from `<Tag as ServiceContract>::version()` — there is no
    /// caller-supplied version parameter. Registering the same `(Tag,
    /// version)` twice returns `Err(RegistryError::DuplicateService)`; unlike
    /// `with_adapter`/`with_config`'s last-write-wins, a duplicate service
    /// registration is surfaced, not silently replaced (see design.md AD-2).
    pub fn with_service<Tag>(mut self, svc: Arc<Tag::Service>) -> Result<Self, RegistryError>
    where
        Tag: Resolvable + 'static,
    {
        let raw: Arc<dyn Any + Send + Sync> = Arc::new(ResolvableContainer(svc));
        self.registry.register::<Tag>(<Tag as ServiceContract>::version(), raw)?;
        Ok(self)
    }

    /// Records `S::validate` — a pure `dependencies()` presence check that
    /// constructs nothing — to be run by `try_build()` (AD-3, F-02). Has
    /// zero effect on `build()`; the bookkeeping recorded here only takes
    /// effect when the caller later calls `try_build()` instead of `build()`.
    pub fn with_injectable<S: Injectable>(mut self) -> Self {
        self.validators.push((std::any::type_name::<S>(), S::validate));
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

    /// Consumes the builder and produces a [`Runtime`], first running every
    /// `with_injectable`-recorded validator against the freshly built
    /// runtime's resolved tables. Fails fast on the first missing
    /// dependency, naming both the missing type and the requesting service
    /// (AD-3/AD-4). Calls the existing infallible [`Self::build`] unchanged
    /// — `Injectable::build` is never invoked here, only `Injectable::validate`.
    pub fn try_build(mut self) -> Result<Runtime, RuntimeError> {
        let validators = std::mem::take(&mut self.validators);
        let rt = self.build();
        for (service_name, validate) in validators {
            if let Err(err) = validate(rt.inner()) {
                let err = match err {
                    RuntimeError::DependencyNotFound { type_name, .. } => {
                        RuntimeError::DependencyNotFound {
                            type_name,
                            service_name: Some(service_name),
                        }
                    }
                    other => other,
                };
                return Err(err);
            }
        }
        Ok(rt)
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

    /// Resolves `Tag` to its concrete macro-generated proxy — the canonical
    /// registration/resolution path (AD-1/AD-2, F-01). Internally identical
    /// to the hand-rolled `{Trait}Ref::new(inner, chain, weak)` path: same
    /// interceptor chain, same weak runtime handle, same generated
    /// `create_proxy` body — so the guard order it enforces is unchanged and
    /// not bypassable through `resolve`. Not cached: each call constructs a
    /// fresh proxy value wrapping the same registered `Arc`-backed instance.
    pub fn resolve<Tag>(&self) -> Result<Tag::Proxy, RuntimeError>
    where
        Tag: Resolvable + 'static,
    {
        let raw = self
            .inner
            .registry
            .resolve_raw::<Tag>(&VersionConstraint::Exact(<Tag as ServiceContract>::version()))
            .map_err(|_| RuntimeError::ServiceNotFound)?;
        Tag::create_proxy(raw, self.inner.interceptor_chain.clone(), Arc::downgrade(&self.inner))
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
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound { .. })));
    }

    #[test]
    fn resolve_config_unregistered_returns_dependency_not_found() {
        let rt = RuntimeBuilder::new().build();
        let result = rt.inner().resolve_config::<StubConfig>();
        assert!(matches!(result, Err(RuntimeError::DependencyNotFound { .. })));
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

    // -- CORE-025 TASK-015: with_injectable / try_build ----------------------
    //
    // Hand-rolled `Injectable` (mirrors testkit's `HandRolledService`
    // pattern) — the `#[service]` macro is a dev-dependency here and its
    // generated code references `ego_service_sdk::...` paths that don't
    // resolve from inside this crate's own unit tests, so services needing
    // the real `Resolvable`/`Tag` machinery (TASK-013/014) are tested as
    // integration tests instead (`tests/with_service_resolve.rs`).

    use std::any::TypeId;

    use crate::di::{AdapterRef, DepKey, Injectable};
    use crate::runtime::RuntimeInner;

    struct NeedsAdapter {
        adapter: AdapterRef<StubAdapter>,
    }

    impl Injectable for NeedsAdapter {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Adapter(
                TypeId::of::<StubAdapter>(),
                std::any::type_name::<StubAdapter>(),
            )]
        }

        fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(Self {
                adapter: rt.resolve_adapter::<StubAdapter>()?,
            })
        }
    }

    #[test]
    fn try_build_fails_fast_on_missing_dependency_naming_both_type_and_service() {
        // `Runtime` (the `Ok` type) doesn't implement `Debug`, so `expect_err`
        // isn't available here — match manually instead.
        let err = match RuntimeBuilder::new().with_injectable::<NeedsAdapter>().try_build() {
            Err(e) => e,
            Ok(_) => panic!("try_build must fail fast when a recorded dependency is missing"),
        };

        match err {
            RuntimeError::DependencyNotFound { type_name, service_name } => {
                assert_eq!(type_name, std::any::type_name::<StubAdapter>());
                assert_eq!(service_name, Some(std::any::type_name::<NeedsAdapter>()));
            }
            other => panic!("expected DependencyNotFound naming both type and service, got {other:?}"),
        }
    }

    #[test]
    fn try_build_succeeds_identically_to_build_when_all_dependencies_present() {
        let rt = RuntimeBuilder::new()
            .with_adapter(Arc::new(StubAdapter(7)))
            .with_injectable::<NeedsAdapter>()
            .try_build()
            .expect("all recorded dependencies present, try_build must succeed");

        let svc = NeedsAdapter::build(rt.inner())
            .expect("build() succeeds using the same resolved adapter try_build validated");
        assert_eq!(*svc.adapter, StubAdapter(7));
    }

    #[test]
    fn build_remains_infallible_and_untouched_by_with_injectable_bookkeeping() {
        // A required adapter is missing, but calling build() (not try_build())
        // must still succeed — with_injectable bookkeeping has no effect on
        // build(), matching the existing "build() Behavior Is Unchanged" contract.
        let rt = RuntimeBuilder::new().with_injectable::<NeedsAdapter>().build();
        assert!(rt.inner().resolve_adapter::<StubAdapter>().is_err());
    }
}
