use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ego_domain::Observability;
use ego_runtime::effects::{
    DeliveryConfig, DuplicateEffectType, EffectDedupStore, EffectStateStore, ExecutorRegistry,
    ExternalEffectExecutor, InMemoryEffectStore, RuntimeEffectAcceptor,
};
use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;
use kitlogger::KITLogger;
use persistent_entity::effect_acceptor::EffectAcceptor;

use crate::contract::{ServiceContract, VersionConstraint};
use crate::di::Injectable;
use crate::interceptor::InterceptorChain;
use crate::registry::{RegistryError, ServiceRegistry};
use crate::runtime::logger::TeardownStack;
use crate::runtime::runtime_builder::{DependencyTable, RuntimeError, RuntimeInner};
use crate::runtime::tenant::{TenantEnforcementMode, TenantResolver};
use crate::runtime::{Resolvable, ResolvableContainer, RuntimeInfraError};

/// Default deadline `Runtime::shutdown_async` waits for the external-effects
/// `Deferred` drain loop before forcibly recovering any still-`InFlight`
/// effect back to `Pending` for a future run (CORE-019 Phase 9, design.md
/// §8). Overridable via [`RuntimeBuilder::with_effect_drain_deadline`].
const DEFAULT_EFFECT_DRAIN_DEADLINE: Duration = Duration::from_secs(5);

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
///
/// `Clone` (every field is: `ServiceRegistry` derives it; the rest are
/// `Arc`/`Copy`/`Vec<Copy>`) — added so a caller can snapshot a builder
/// before a fallible operation (e.g. `with_service`, which consumes `self`
/// and drops it on `Err`) and still observe the pre-call state afterward.
#[derive(Clone)]
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
    /// Observability sink for macro-guard security denials (CORE-012A
    /// AD-2). Default `None` — behaviorally identical to today, before this
    /// change existed.
    observability: Option<Arc<dyn Observability>>,
    /// Executors registered via [`RuntimeBuilder::register_effect_executor`]
    /// (CORE-019 Phase 9). Empty by default — the zero-cost gate `build()`
    /// checks to decide whether to construct the external-effects subsystem
    /// at all.
    effect_executors: ExecutorRegistry,
    /// Delivery pipeline configuration for the external-effects subsystem.
    /// Only meaningful once at least one executor is registered.
    delivery_config: DeliveryConfig,
    /// How long graceful shutdown waits for the `Deferred` drain loop before
    /// forcing remaining in-flight effects back to `Pending` (design.md §8).
    effect_drain_deadline: Duration,
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
            observability: None,
            effect_executors: ExecutorRegistry::new(),
            delivery_config: DeliveryConfig::default(),
            effect_drain_deadline: DEFAULT_EFFECT_DRAIN_DEADLINE,
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

    /// Registers an `Observability` implementor to receive macro-guard
    /// security-denial events (CORE-012A). When never called, the runtime
    /// defaults to `None` (AD-2) — recording is a silent no-op and behavior
    /// is byte-for-byte identical to before this change existed.
    pub fn with_observability(mut self, obs: Arc<dyn Observability>) -> Self {
        self.observability = Some(obs);
        self
    }

    /// Registers `executor` as the sole owner of every `effect_type` in
    /// `effect_types` (CORE-019 Phase 9, design.md §6.4's builder sugar).
    /// Fails closed on a duplicate `effect_type` — the already-shipped
    /// `ExecutorRegistry`'s "one owner per type" contract; the first
    /// registration is left untouched.
    ///
    /// Registering at least one executor is what makes [`RuntimeBuilder::build`]
    /// construct a real external-effects delivery pipeline (see
    /// [`Runtime::effect_acceptor`]). A `Runtime` that never calls this method
    /// keeps the whole subsystem at zero cost: no store, no queue, no spawned
    /// drain task (spec: "zero/near-zero cost when no external effects are
    /// used").
    pub fn register_effect_executor(
        mut self,
        effect_types: impl IntoIterator<Item = impl Into<String>>,
        executor: Arc<dyn ExternalEffectExecutor>,
    ) -> Result<Self, DuplicateEffectType> {
        for effect_type in effect_types {
            self.effect_executors.register(effect_type, executor.clone())?;
        }
        Ok(self)
    }

    /// Configures the [`DeliveryConfig`] used by the external-effects delivery
    /// pipeline. Defaults to [`DeliveryConfig::default`] (AD-5 backoff,
    /// `Deferred` runner mode) — only meaningful once at least one executor
    /// is registered via [`RuntimeBuilder::register_effect_executor`].
    pub fn with_delivery_config(mut self, config: DeliveryConfig) -> Self {
        self.delivery_config = config;
        self
    }

    /// Configures how long [`Runtime::shutdown_async`] waits for the
    /// external-effects `Deferred` drain loop to finish in-flight deliveries
    /// before forcing them back to `Pending` for a future run (design.md §8;
    /// AD-9's "acceptance retries respect the drain deadline, don't block
    /// shutdown forever" applied to in-flight delivery too). Defaults to 5
    /// seconds. Only meaningful once at least one executor is registered.
    pub fn with_effect_drain_deadline(mut self, deadline: Duration) -> Self {
        self.effect_drain_deadline = deadline;
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

        // CORE-019 Phase 9 zero-cost gate (design.md §8/§20): construct the
        // external-effects store/queue/runner ONLY when at least one
        // executor was registered. `effect_wiring` keeps the concrete
        // `RuntimeEffectAcceptor` (needed for its `drain` method below);
        // `effect_acceptor` below is the trait-object handle stored on
        // `Runtime`/exposed to callers.
        let effect_wiring = if self.effect_executors.is_empty() {
            None
        } else {
            let store = Arc::new(InMemoryEffectStore::new());
            let (acceptor, shutdown_tx) = RuntimeEffectAcceptor::new(
                store.clone() as Arc<dyn EffectStateStore>,
                store as Arc<dyn EffectDedupStore>,
                Arc::new(self.effect_executors),
                self.delivery_config,
            );
            Some((Arc::new(acceptor), shutdown_tx))
        };
        let effect_acceptor: Option<Arc<dyn EffectAcceptor>> = effect_wiring
            .as_ref()
            .map(|(acceptor, _)| acceptor.clone() as Arc<dyn EffectAcceptor>);

        let rt = Runtime {
            inner: Arc::new(RuntimeInner::new_with_logger(
                self.registry,
                self.interceptor_chain,
                security_providers,
                DependencyTable::with_registrations(self.adapters, self.configs),
                self.logger,
                Mutex::new(teardown),
                TenantResolver::new(self.tenant_enforcement_mode),
                self.observability,
                effect_acceptor,
            )),
        };

        // Only when ≥1 executor was registered: signal the drain deadline via
        // `register_async_teardown` so `shutdown_async` recovers any
        // still-in-flight effect back to `Pending` rather than losing it or
        // blocking shutdown forever (AD-9). No hook is registered at all in
        // the zero-cost path above.
        if let Some((acceptor, shutdown_tx)) = effect_wiring {
            let deadline = self.effect_drain_deadline;
            rt.register_async_teardown(async move {
                let recovered = acceptor.drain(deadline, shutdown_tx).await;
                if recovered > 0 {
                    return Err(RuntimeInfraError::Teardown {
                        reason: format!(
                            "external-effects drain deadline reached with {recovered} \
                             effect(s) still in flight; recovered to Pending for a \
                             future run (drain_incomplete)"
                        ),
                    });
                }
                Ok(())
            });
        }

        rt
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

    /// Returns the external-effects [`EffectAcceptor`] wired by
    /// [`RuntimeBuilder::register_effect_executor`], if at least one executor
    /// was registered (CORE-019 Phase 9). `None` is the zero-cost path
    /// (design.md §8/§20) — no store, no queue, no spawned drain task was
    /// ever constructed, not merely an unused acceptor.
    ///
    /// This is the seam a host wires into its entity-level runtime (e.g.
    /// `persistent_entity::builder::EntityRuntimeBuilder`) so spawned actors
    /// stop silently discarding described effects — that host-side plumbing
    /// is out of `ego-service-sdk`'s scope (it lives wherever the host
    /// constructs its `EntityRuntimeBuilder`/`EntityRuntime`).
    pub fn effect_acceptor(&self) -> Option<&Arc<dyn EffectAcceptor>> {
        self.inner.effect_acceptor.as_ref()
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

    /// Registers an async teardown hook (Finding 6), run by
    /// [`Runtime::shutdown_async`] before the existing sync `shutdown()`
    /// drains the logger/security teardown stack. Hooks run in registration
    /// order — e.g. a read-side scheduler's stop-and-drain, registered
    /// first, always completes before the next hook or the sync stack runs.
    ///
    /// The hook's `Result` is not decorative (post-review Finding F-02): a
    /// hook that fails to drain (e.g. its spawned task panicked) must be
    /// distinguishable from one that drained cleanly, so callers of
    /// [`Runtime::shutdown_async`] can tell "shutdown finished" apart from
    /// "shutdown finished, but something didn't drain."
    ///
    /// Purely additive: a `Runtime` that never calls this has an empty hook
    /// list, and `shutdown()` behaves exactly as before this method existed.
    /// Registered post-build (via `&self`, not on `RuntimeBuilder`) because
    /// the real motivating case — a spawned read-side scheduler's `stop()`
    /// future — is itself only constructible after the `Runtime` is already
    /// built.
    pub fn register_async_teardown<F>(&self, hook: F)
    where
        F: Future<Output = Result<(), RuntimeInfraError>> + Send + 'static,
    {
        self.inner
            .async_teardown
            .lock()
            .expect("async teardown mutex poisoned")
            .push(Box::pin(hook));
    }

    /// Additive async counterpart to [`Runtime::shutdown`] (Finding 6).
    ///
    /// Awaits every hook registered via [`Runtime::register_async_teardown`]
    /// in registration order — ALL of them, even after one fails, so one
    /// broken subsystem's teardown never prevents another's from running —
    /// then calls the existing sync `shutdown()` regardless, to avoid
    /// leaking logger/security resources even when a hook failed. Returns
    /// the FIRST hook error if any hook failed, else whatever `shutdown()`
    /// returns. Existing callers of the sync `shutdown()` are completely
    /// unaffected — this method only adds a new opt-in entry point.
    pub async fn shutdown_async(&self) -> Result<(), RuntimeInfraError> {
        let hooks = std::mem::take(
            &mut *self
                .inner
                .async_teardown
                .lock()
                .expect("async teardown mutex poisoned"),
        );
        let mut first_hook_err = None;
        for hook in hooks {
            if let Err(e) = hook.await {
                if first_hook_err.is_none() {
                    first_hook_err = Some(e);
                }
            }
        }
        let sync_result = self.shutdown();
        match first_hook_err {
            Some(e) => Err(e),
            None => sync_result,
        }
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
    use crate::runtime::{RuntimeError, RuntimeInfraError};

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

    use crate::di::{AdapterRef, ConfigValue, DepKey, Injectable};
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

    /// A second `Injectable` fixture with a *different* missing-dependency
    /// kind (config, not adapter) — needed so the "first-of-multiple, in
    /// registration order" guarantee can be distinguished from "always
    /// reports whichever kind happens to be checked first."
    struct NeedsConfig {
        #[allow(dead_code)]
        limit: ConfigValue<u32>,
    }

    impl Injectable for NeedsConfig {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Config(TypeId::of::<u32>(), std::any::type_name::<u32>())]
        }

        fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(Self { limit: rt.resolve_config::<u32>()? })
        }
    }

    /// Spec scenario: "Multiple missing dependencies report only the first,
    /// in registration order" (Fail-Fast Dependency Validation requirement,
    /// AD-3). Registers two `Injectable` services, both with a genuinely
    /// missing dependency, and asserts the reported error names only the
    /// FIRST-registered one — then flips the registration order and confirms
    /// the reported service flips too, proving this is driven by
    /// registration order (the `Vec` + linear scan in `try_build`), not by
    /// coincidence or dependency kind.
    #[test]
    fn try_build_reports_only_the_first_registered_service_when_multiple_are_missing_dependencies() {
        let err = match RuntimeBuilder::new()
            .with_injectable::<NeedsAdapter>()
            .with_injectable::<NeedsConfig>()
            .try_build()
        {
            Err(e) => e,
            Ok(_) => panic!("try_build must fail when multiple recorded dependencies are missing"),
        };
        match err {
            RuntimeError::DependencyNotFound { type_name, service_name } => {
                assert_eq!(type_name, std::any::type_name::<StubAdapter>());
                assert_eq!(service_name, Some(std::any::type_name::<NeedsAdapter>()));
            }
            other => panic!(
                "expected DependencyNotFound naming the first-registered NeedsAdapter, got {other:?}"
            ),
        }

        let err = match RuntimeBuilder::new()
            .with_injectable::<NeedsConfig>()
            .with_injectable::<NeedsAdapter>()
            .try_build()
        {
            Err(e) => e,
            Ok(_) => panic!("try_build must fail when multiple recorded dependencies are missing"),
        };
        match err {
            RuntimeError::DependencyNotFound { type_name, service_name } => {
                assert_eq!(type_name, std::any::type_name::<u32>());
                assert_eq!(service_name, Some(std::any::type_name::<NeedsConfig>()));
            }
            other => panic!(
                "expected DependencyNotFound naming the first-registered NeedsConfig, got {other:?}"
            ),
        }
    }

    // -- CORE-012A Phase 3 (TASK-006/007): with_observability wiring --------

    use crate::runtime::runtime_builder::SecurityDenialKind;
    use crate::test_support::RecordingObservability;

    #[test]
    fn with_observability_wiring_reaches_runtime_inner() {
        let obs = Arc::new(RecordingObservability::new());
        let rt = RuntimeBuilder::new().with_observability(obs.clone()).build();

        rt.inner()
            .record_security_denial("Svc", "op", SecurityDenialKind::AuthorizationDenied);

        assert_eq!(
            obs.events.lock().unwrap().len(),
            1,
            "with_observability must wire the supplied implementor through to RuntimeInner"
        );
    }

    #[test]
    fn build_without_with_observability_preserves_existing_behavior() {
        // Default (AD-2): observability is None. Existing allowed/denied guard
        // behavior (authorization_integration.rs, tenant_scoped_codegen.rs —
        // neither calls with_observability) is unaffected by this change;
        // this test proves the None-path plumbing itself never panics.
        let rt = RuntimeBuilder::new().build();
        rt.inner()
            .record_security_denial("Svc", "op", SecurityDenialKind::MissingContext);
    }

    // -- Finding 6 (post-CORE-018 review): async teardown hooks -------------
    //
    // Additive: registers async hooks (e.g. a read-side scheduler's stop())
    // to run, in order, before the existing sync teardown stack drains —
    // without making the existing sync `shutdown()` async.

    use kitlogger_log_domain::Severity;

    #[tokio::test]
    async fn shutdown_async_runs_hooks_in_registration_order_before_sync_stack_drains() {
        let logger = initialized_logger();
        let rt = RuntimeBuilder::new().with_logger(logger.clone()).build();

        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let order_a = order.clone();
        let order_b = order.clone();

        rt.register_async_teardown(async move {
            order_a.lock().unwrap().push("first");
            Ok(())
        });
        rt.register_async_teardown(async move {
            order_b.lock().unwrap().push("second");
            Ok(())
        });

        rt.shutdown_async().await.expect("shutdown_async succeeds");

        assert_eq!(*order.lock().unwrap(), vec!["first", "second"]);
        // The sync (logger) teardown stack was also drained by shutdown_async.
        assert!(logger.log(Severity::Info, "after-shutdown").is_err());
    }

    #[tokio::test]
    async fn shutdown_async_with_no_registered_hooks_still_drains_sync_stack() {
        let logger = initialized_logger();
        let rt = RuntimeBuilder::new().with_logger(logger.clone()).build();

        rt.shutdown_async().await.expect("shutdown_async succeeds");
        assert!(logger.log(Severity::Info, "after-shutdown").is_err());
    }

    #[tokio::test]
    async fn shutdown_async_is_idempotent() {
        let rt = RuntimeBuilder::new().build();
        assert!(rt.shutdown_async().await.is_ok());
        assert!(rt.shutdown_async().await.is_ok());
    }

    /// Post-review Finding F-02: a hook that fails to drain must make
    /// `shutdown_async` report failure, not silently succeed as if nothing
    /// went wrong — while still draining the sync (logger) stack, so a
    /// failed subsystem never leaks unrelated infrastructure.
    #[tokio::test]
    async fn shutdown_async_surfaces_a_failing_hook_but_still_drains_the_sync_stack() {
        let logger = initialized_logger();
        let rt = RuntimeBuilder::new().with_logger(logger.clone()).build();

        rt.register_async_teardown(async move {
            Err(RuntimeInfraError::Teardown {
                reason: "simulated read-side scheduler task failure".to_string(),
            })
        });

        let result = rt.shutdown_async().await;
        assert!(result.is_err(), "a failing hook must surface as an Err, not a silent Ok(())");

        // The sync stack drained regardless of the hook's failure — no
        // resource leak just because one subsystem's teardown failed.
        assert!(logger.log(Severity::Info, "after-shutdown").is_err());
    }

    /// Only the FIRST hook's error is surfaced, but every hook still runs —
    /// a second, later hook's own teardown must not be skipped just because
    /// an earlier one failed.
    #[tokio::test]
    async fn shutdown_async_runs_every_hook_even_after_an_earlier_one_fails() {
        let rt = RuntimeBuilder::new().build();
        let second_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let second_ran_for_closure = second_ran.clone();

        rt.register_async_teardown(async move {
            Err(RuntimeInfraError::Teardown {
                reason: "first hook fails".to_string(),
            })
        });
        rt.register_async_teardown(async move {
            second_ran_for_closure.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });

        let result = rt.shutdown_async().await;
        assert!(result.is_err(), "the first hook's error must still surface");
        assert!(
            second_ran.load(std::sync::atomic::Ordering::SeqCst),
            "the second hook must still have run despite the first hook's failure"
        );
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

    // -- CORE-019 Phase 9 (RED 9.1 / GREEN 9.2): register_effect_executor,
    // DeliveryConfig option, conditional runner spawn, drain-on-shutdown ----

    use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
    use ego_runtime::effects::{
        AttemptOutcome, DeliveryConfig, DuplicateEffectType, EffectContext, ExternalEffectExecutor,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use tokio::sync::Notify;

    fn effect_description(effect_type: &str, key: &str) -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new(key).unwrap(),
            effect_type: effect_type.to_string(),
            payload: vec![1, 2, 3],
            destination: "https://example.com".to_string(),
        }
    }

    fn effect_tenant() -> TenantId {
        TenantId::new("tenant-a").unwrap()
    }

    struct AlwaysSucceedsExecutor {
        calls: AtomicUsize,
    }

    impl AlwaysSucceedsExecutor {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ExternalEffectExecutor for AlwaysSucceedsExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            AttemptOutcome::Success
        }
    }

    /// Blocks inside `execute` until `gate` is notified — used to prove a
    /// stuck delivery is what triggers `drain_incomplete`, never a silently
    /// swallowed failure.
    struct GatedExecutor {
        gate: Arc<Notify>,
    }

    #[async_trait]
    impl ExternalEffectExecutor for GatedExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            self.gate.notified().await;
            AttemptOutcome::Success
        }
    }

    #[test]
    fn build_without_registering_any_effect_executor_wires_no_acceptor() {
        // Zero-cost path (design.md §8/§20): no executor registered means
        // `build()` never constructs a store/queue/runner at all — proven
        // here by the absence of the acceptor itself, not merely an unused
        // `Some`.
        let rt = RuntimeBuilder::new().build();
        assert!(
            rt.effect_acceptor().is_none(),
            "no executor was registered, so no RuntimeEffectAcceptor may exist"
        );
    }

    #[tokio::test]
    async fn shutdown_async_with_no_registered_effect_executor_completes_instantly() {
        // Companion proof of the zero-cost path: with nothing registered, no
        // async teardown hook exists for the effects subsystem either, so
        // shutdown_async has nothing effects-related to await.
        let rt = RuntimeBuilder::new().build();
        let started = Instant::now();
        rt.shutdown_async().await.expect("shutdown_async succeeds");
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "no effect executor was registered — shutdown must not wait on anything effects-related"
        );
    }

    #[test]
    fn register_effect_executor_duplicate_effect_type_fails_closed() {
        // `RuntimeBuilder` doesn't implement `Debug` (matches `with_service`'s
        // existing Result-returning pattern above), so match manually rather
        // than `.expect_err`.
        let executor: Arc<dyn ExternalEffectExecutor> = Arc::new(AlwaysSucceedsExecutor::new());
        let err = match RuntimeBuilder::new()
            .register_effect_executor(["invoice.created"], executor.clone())
            .expect("first registration succeeds")
            .register_effect_executor(["invoice.created"], executor)
        {
            Err(e) => e,
            Ok(_) => panic!("a second executor for the same effect_type must fail closed"),
        };

        assert!(matches!(
            err,
            DuplicateEffectType::AlreadyRegistered(t) if t == "invoice.created"
        ));
    }

    #[tokio::test]
    async fn register_effect_executor_makes_build_wire_a_real_acceptor() {
        let executor: Arc<dyn ExternalEffectExecutor> = Arc::new(AlwaysSucceedsExecutor::new());
        let rt = RuntimeBuilder::new()
            .register_effect_executor(["invoice.created"], executor)
            .unwrap()
            .build();

        assert!(
            rt.effect_acceptor().is_some(),
            "registering >=1 executor must make build() wire a real RuntimeEffectAcceptor"
        );
    }

    #[tokio::test]
    async fn accepted_effects_are_actually_delivered_through_the_wired_acceptor() {
        // Proves the acceptor build() wires is not inert: an accepted effect
        // really reaches the registered executor — this is the exact gap
        // PR3 flagged (effects silently lost when no acceptor is configured).
        let executor = Arc::new(AlwaysSucceedsExecutor::new());
        let rt = RuntimeBuilder::new()
            .register_effect_executor(["invoice.created"], executor.clone())
            .unwrap()
            .build();

        let acceptor = rt.effect_acceptor().unwrap().clone();
        acceptor
            .accept(
                &effect_tenant(),
                vec![effect_description("invoice.created", "uow-1:0")],
            )
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            while executor.call_count() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the accepted effect is delivered through the spawned Deferred runner");
    }

    #[tokio::test]
    async fn shutdown_async_drains_cleanly_when_delivery_completes_before_the_deadline() {
        let executor = Arc::new(AlwaysSucceedsExecutor::new());
        let rt = RuntimeBuilder::new()
            .register_effect_executor(["invoice.created"], executor.clone())
            .unwrap()
            .with_effect_drain_deadline(Duration::from_millis(200))
            .build();

        let acceptor = rt.effect_acceptor().unwrap().clone();
        acceptor
            .accept(
                &effect_tenant(),
                vec![effect_description("invoice.created", "uow-1:0")],
            )
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            while executor.call_count() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("effect delivered before shutdown");

        rt.shutdown_async()
            .await
            .expect("nothing was stuck in flight, so drain must report a clean shutdown");
    }

    #[tokio::test]
    async fn shutdown_async_surfaces_drain_incomplete_when_the_deadline_is_hit() {
        // RED 9.1's core proof: an effect stuck mid-delivery when the drain
        // deadline elapses must (a) not block shutdown forever and (b) make
        // shutdown_async report failure — the documented `drain_incomplete`
        // signal — rather than silently discarding it.
        let gate = Arc::new(Notify::new());
        let executor = Arc::new(GatedExecutor { gate: gate.clone() });
        let rt = RuntimeBuilder::new()
            .register_effect_executor(["invoice.created"], executor)
            .unwrap()
            .with_delivery_config(DeliveryConfig::default())
            .with_effect_drain_deadline(Duration::from_millis(30))
            .build();

        let acceptor = rt.effect_acceptor().unwrap().clone();
        acceptor
            .accept(
                &effect_tenant(),
                vec![effect_description("invoice.created", "uow-1:0")],
            )
            .await
            .unwrap();

        // Let the spawned Deferred loop dequeue and start executing — it
        // will now block forever inside the gated executor since `gate` is
        // deliberately never notified.
        tokio::time::sleep(Duration::from_millis(20)).await;

        let started = Instant::now();
        let result = rt.shutdown_async().await;
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "graceful shutdown must respect the configured drain deadline, never block forever \
             on a stuck effect (AD-9)"
        );
        assert!(
            result.is_err(),
            "a still-in-flight effect at the deadline must surface as drain_incomplete, not a \
             silent success"
        );
    }
}
