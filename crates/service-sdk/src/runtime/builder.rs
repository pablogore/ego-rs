use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ego_domain::event::DomainEvent;
use ego_domain::{Observability, Tracer, TracerLifecycle};
use ego_runtime::effects::{
    DeliveryConfig, DuplicateEffectType, EffectDedupStore, EffectStateStore, ExecutorRegistry,
    ExternalEffectExecutor, InMemoryEffectStore, RuntimeEffectAcceptor,
};
use ego_runtime::providers::{
    DuplicateProviderId, ExternalDataProvider, ExternalDataProviderRegistry, ProviderAccessConfig,
    RuntimeDataProviderAccess,
};
use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;
use futures::FutureExt;
use kitlogger::KITLogger;
use persistent_entity::data_provider_access::DataProviderAccess;
use persistent_entity::effect_acceptor::EffectAcceptor;
use persistent_entity::persistent_entity::PersistentEntity;
use persistent_entity::runtime::EntityRuntime;

use crate::contract::{ServiceContract, VersionConstraint};
use crate::di::{DuplicateEntity, DuplicateProjection, Injectable};
use crate::health::{HealthAggregationConfig, HealthAggregator, HealthRegistry};
use crate::implementation::LifecycleManaged;
use crate::interceptor::{InterceptorChain, TracingInterceptor};
use crate::registry::{RegistryError, ServiceRegistry};
use crate::runtime::logger::TeardownStack;
use crate::runtime::runtime_builder::{
    DependencyTable, RegisteredDependencies, RuntimeError, RuntimeInner,
};
use ego_domain::operation::OperationReservationStore;

use crate::runtime::idempotency::IdempotencyEnforcementMode;
use crate::runtime::tenant::{TenantEnforcementMode, TenantResolver};
use crate::runtime::{Resolvable, ResolvableContainer, RuntimeInfraError};

/// Default deadline `Runtime::shutdown_async`'s registered teardown hook
/// waits for `EffectRuntimeHandle::shutdown_and_wait`'s lifecycle-gated
/// drain sequence (close admission, wait for in-flight acceptances, stop the
/// runner, force-abort anything still hung) to finish before reporting
/// `drain_incomplete` (CORE-019 Phase 9, design.md §8). Overridable via
/// [`RuntimeBuilder::with_effect_drain_deadline`].
const DEFAULT_EFFECT_DRAIN_DEADLINE: Duration = Duration::from_secs(5);

/// The pair of security providers registered with a [`Runtime`].
pub type SecurityProviders = (
    Arc<dyn AuthenticationProvider>,
    Arc<dyn AuthorizationProvider>,
);

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
    /// Projections registered via [`RuntimeBuilder::with_projection`]
    /// (CORE-028 Stage 2). Fail-closed on a duplicate type — unlike
    /// `adapters`/`configs`'s last-write-wins semantics (AD-1/AD-2).
    projections: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Entity runtimes registered via [`RuntimeBuilder::with_entity`]
    /// (CORE-028 Stage 2C), keyed by the aggregate type `E`, never `E::Event`
    /// (design.md AD-1). Fail-closed on a duplicate type, same posture as
    /// `projections` (AD-4).
    entities: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    tenant_enforcement_mode: TenantEnforcementMode,
    /// How a missing client-supplied operation key is treated.
    ///
    /// Defaults to the enforcing variant, and that default has teeth: under it a
    /// runtime cannot be built without an [`OperationReservationStore`], because a
    /// runtime that promises mandatory keys and has nowhere to reserve them cannot
    /// keep the promise.
    idempotency_enforcement_mode: IdempotencyEnforcementMode,
    /// The single registered reservation store, if any.
    idempotency_reservation_store: Option<Arc<dyn OperationReservationStore>>,
    /// Clock the reservation lease is computed from. `None` means the real one.
    /// Injectable so lease expiry is testable without wall time (AD-3i).
    reservation_clock: Option<Arc<dyn ego_domain::time::Clock>>,
    /// Identity this runtime reserves under. `None` means `build()` mints one.
    /// Injectable only so `OwnedInProgress`, `OtherInProgress` and `TakenOver`
    /// can be exercised deterministically; production must neither share it
    /// across instances nor persist it across restarts (AD-3i).
    reservation_owner_id: Option<ego_domain::operation::OwnerId>,
    /// How long a reservation's lease holds. Must exceed the longest a
    /// legitimate execution can take: when it expires another owner may take
    /// over *while the original is still running*, so until renewal exists a
    /// short lease permits overlap (AD-3i).
    reservation_lease_duration: std::time::Duration,
    /// `(service_name, S::validate)` pairs recorded via `with_injectable`.
    /// Read only by `try_build()`; has no effect on `build()` (AD-3).
    validators: Vec<ValidatorEntry>,
    /// Observability sink for macro-guard security denials (CORE-012A
    /// AD-2). Default `None` — behaviorally identical to today, before this
    /// change existed.
    observability: Option<Arc<dyn Observability>>,
    /// The `Tracer` used to drive spans (PROD-003 Phase 4). Default `None` —
    /// behaviorally identical to today: no `TracingInterceptor` is wired into
    /// the interceptor chain at all when this is `None` (not even a
    /// `NoopTracer`-backed one running for nothing). Set via
    /// [`RuntimeBuilder::with_tracer`] or [`RuntimeBuilder::with_traced`].
    tracer: Option<Arc<dyn Tracer>>,
    /// The `TracerLifecycle` owned SOLELY for a single `shutdown()` on
    /// teardown (design.md ADR-9) — `shutdown` is exporter/operational
    /// lifecycle, not a domain tracing call, so it is a deliberately separate
    /// field from `tracer` above: `NoopTracer` and lifecycle-less `Tracer`
    /// spies never need to supply one. Default `None` — no async teardown
    /// hook is registered at all when this is `None`. Set via
    /// [`RuntimeBuilder::with_tracer_lifecycle`] or [`RuntimeBuilder::with_traced`].
    tracer_lifecycle: Option<Arc<dyn TracerLifecycle>>,
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
    /// Providers registered via
    /// [`RuntimeBuilder::register_data_provider`] (CORE-019A Phase 4). Empty
    /// by default — the zero-cost gate `build()` checks to decide whether to
    /// construct the `RuntimeDataProviderAccess` facade at all (AD-006).
    data_provider_registry: ExternalDataProviderRegistry,
    /// Cross-cutting timeout/retry policy for the provider access chokepoint
    /// (issue #234), applied to `RuntimeDataProviderAccess` at `build()`.
    /// Defaults to [`ProviderAccessConfig::default`]; only meaningful once at
    /// least one provider is registered.
    provider_access_config: ProviderAccessConfig,
    /// Every provider registered via
    /// [`RuntimeBuilder::register_data_provider`], kept alongside the
    /// registry above purely so `build()` can drive each one's `shutdown()`
    /// exactly once through the single owning teardown path (spec:
    /// "Explicit, Single-Owner Lifecycle") — the registry itself is moved
    /// into `RuntimeDataProviderAccess` and is no longer iterable once
    /// `build()` constructs the facade.
    ///
    /// Deduplicated by `Arc::ptr_eq` (single-owner teardown): registering the
    /// same `Arc` under two different `provider_id`s stores it here only
    /// once. This is DELIBERATELY NOT the source `build()` uses for health
    /// contributors — see [`Self::provider_health_pairs`], which preserves
    /// every alias.
    data_providers_for_teardown: Vec<Arc<dyn ExternalDataProvider>>,
    /// Every `(provider_id, provider)` pair registered via
    /// [`RuntimeBuilder::register_data_provider`] (PROD-005 PR3 TASK-023),
    /// kept UNDEDUPLICATED — unlike [`Self::data_providers_for_teardown`],
    /// which collapses an aliased `Arc` to one teardown call, health is a
    /// per-registered-id contract: the same provider `Arc` registered under
    /// two distinct `provider_id`s must still produce two independent
    /// [`ego_runtime::providers::ProviderHealthContributor`]s, one per id.
    provider_health_pairs: Vec<(String, Arc<dyn ExternalDataProvider>)>,
    /// Lifecycle-managed components registered via
    /// [`RuntimeBuilder::with_lifecycle_component`] (PROD-005 PR2
    /// TASK-018/019). `build()` folds every registered component's
    /// `LifecycleManaged::health_contributors()`, together with a
    /// [`ego_runtime::providers::ProviderHealthContributor`] per entry in
    /// [`Self::provider_health_pairs`] (PROD-005 PR3 TASK-023), into ONE
    /// runtime-owned [`HealthRegistry`] — a component that contributes none
    /// (the trait's default) leaves aggregation unaffected.
    lifecycle_components: Vec<Arc<dyn LifecycleManaged>>,
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
            projections: HashMap::new(),
            entities: HashMap::new(),
            tenant_enforcement_mode: TenantEnforcementMode::AuthenticatedOnly,
            idempotency_enforcement_mode: IdempotencyEnforcementMode::default(),
            idempotency_reservation_store: None,
            reservation_clock: None,
            reservation_owner_id: None,
            reservation_lease_duration: std::time::Duration::from_secs(30),
            validators: Vec::new(),
            observability: None,
            tracer: None,
            tracer_lifecycle: None,
            effect_executors: ExecutorRegistry::new(),
            delivery_config: DeliveryConfig::default(),
            effect_drain_deadline: DEFAULT_EFFECT_DRAIN_DEADLINE,
            data_provider_registry: ExternalDataProviderRegistry::new(),
            provider_access_config: ProviderAccessConfig::default(),
            data_providers_for_teardown: Vec::new(),
            provider_health_pairs: Vec::new(),
            lifecycle_components: Vec::new(),
        }
    }

    /// Registers a lifecycle-managed component whose
    /// `LifecycleManaged::health_contributors()` are folded into the built
    /// runtime's single [`HealthRegistry`] (PROD-005 PR2 TASK-018/019). A
    /// component that never overrides `health_contributors()` (the trait's
    /// default, empty `Vec`) is safe to register — it simply contributes
    /// nothing to health aggregation.
    pub fn with_lifecycle_component(mut self, component: Arc<dyn LifecycleManaged>) -> Self {
        self.lifecycle_components.push(component);
        self
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
        self.adapters
            .insert(TypeId::of::<A>(), adapter as Arc<dyn Any + Send + Sync>);
        self
    }

    /// Registers a host-constructed config value, resolvable via `resolve_config::<C>()`.
    /// Last-write-wins (same semantics as `with_adapter`). CORE-016: accepts only an
    /// already-constructed `Arc<C>`, never a raw config source/loader.
    pub fn with_config<C: Send + Sync + 'static>(mut self, value: Arc<C>) -> Self {
        self.configs
            .insert(TypeId::of::<C>(), value as Arc<dyn Any + Send + Sync>);
        self
    }

    /// Registers a projection instance, resolvable via
    /// `RuntimeInner::resolve_projection::<P>()` (CORE-028 Stage 2 design.md
    /// AD-1). Fails closed on a duplicate registration for the same concrete
    /// type `P` — mirroring `register_effect_executor`/`register_data_provider`'s
    /// fallible, dedicated-error contract, not `with_adapter`/`with_config`'s
    /// last-write-wins. The first registration is left untouched on `Err`;
    /// there is no `replace_projection` escape hatch (AD-2) — a duplicate is
    /// always a bootstrap bug, never an intended override.
    pub fn with_projection<P: Send + Sync + 'static>(
        mut self,
        projection: Arc<P>,
    ) -> Result<Self, DuplicateProjection> {
        let type_id = TypeId::of::<P>();
        if self.projections.contains_key(&type_id) {
            return Err(DuplicateProjection {
                type_name: std::any::type_name::<P>(),
            });
        }
        self.projections
            .insert(type_id, projection as Arc<dyn Any + Send + Sync>);
        Ok(self)
    }

    /// Registers a host-constructed entity runtime, resolvable via
    /// `RuntimeInner::resolve_entity::<E>()` as `EntityRuntimeRef<E>`
    /// (CORE-028 Stage 2C design.md AD-2). The framework constructs
    /// nothing — `runtime` must already be built via the existing,
    /// unchanged `EntityRuntimeBuilder`. Keyed by the aggregate type `E`,
    /// never `E::Event` (AD-1): two distinct aggregates sharing one event
    /// type register and resolve independently. Fails closed on a
    /// duplicate registration for the same aggregate type — mirroring
    /// `with_projection`'s fail-closed contract exactly (AD-4); the first
    /// registration is left untouched on `Err`, and there is no
    /// `replace_entity` escape hatch.
    pub fn with_entity<E>(
        mut self,
        runtime: Arc<EntityRuntime<E::Event>>,
    ) -> Result<Self, DuplicateEntity>
    where
        E: PersistentEntity + 'static,
        E::Event: DomainEvent
            + Clone
            + serde::de::DeserializeOwned
            + serde::Serialize
            + Send
            + Sync
            + 'static,
    {
        let type_id = TypeId::of::<E>();
        if self.entities.contains_key(&type_id) {
            return Err(DuplicateEntity {
                type_name: std::any::type_name::<E>(),
            });
        }
        self.entities
            .insert(type_id, runtime as Arc<dyn Any + Send + Sync>);
        Ok(self)
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
        self.registry
            .register::<Tag>(<Tag as ServiceContract>::version(), raw)?;
        Ok(self)
    }

    /// Records `S::validate` — a pure `dependencies()` presence check that
    /// constructs nothing — to be run by `try_build()` (AD-3, F-02). Has
    /// zero effect on `build()`; the bookkeeping recorded here only takes
    /// effect when the caller later calls `try_build()` instead of `build()`.
    pub fn with_injectable<S: Injectable>(mut self) -> Self {
        self.validators
            .push((std::any::type_name::<S>(), S::validate));
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

    /// Sets how a missing client-supplied operation key is treated.
    ///
    /// The default is [`IdempotencyEnforcementMode::MandatoryKey`], and it is the
    /// reason building without a reservation store fails: a runtime that promises
    /// every mutating operation carries a client key has nowhere to reserve those
    /// keys, so it cannot keep the promise.
    ///
    /// [`IdempotencyEnforcementMode::Compatibility`] is the explicit way to say a
    /// deployment has not adopted enforcement yet. It is deliberately something a
    /// caller has to write: the alternative — treating an unconfigured builder as
    /// not-yet-adopted — would let a deployment end up unguarded without anyone
    /// deciding that, which is exactly what the fail-closed default exists to
    /// prevent.
    pub fn with_idempotency_enforcement_mode(mut self, mode: IdempotencyEnforcementMode) -> Self {
        self.idempotency_enforcement_mode = mode;
        self
    }

    /// Registers the single [`OperationReservationStore`] this runtime reserves
    /// operations through.
    ///
    /// Exactly one: a second call replaces the first rather than accumulating,
    /// because two stores would mean two places a key could be reserved and no
    /// answer to which one decides.
    pub fn with_operation_reservation_store(
        mut self,
        store: Arc<dyn OperationReservationStore>,
    ) -> Self {
        self.idempotency_reservation_store = Some(store);
        self
    }

    /// Overrides the clock the reservation lease is computed from.
    ///
    /// Defaults to the real system clock. It is injectable because lease expiry
    /// is otherwise only observable by waiting: `TakenOver` needs an expired
    /// lease, and a test that produces one by sleeping is a test that is slow
    /// when it passes and flaky when the machine is loaded. This is the same
    /// reason A4 generalised `Clock` out of auth.
    pub fn with_reservation_clock(mut self, clock: Arc<dyn ego_domain::time::Clock>) -> Self {
        self.reservation_clock = Some(clock);
        self
    }

    /// Overrides the identity this runtime reserves under.
    ///
    /// Normally left alone: `build()` mints a fresh UUID per instance, which is
    /// what makes a retry inside this runtime observable as `OwnedInProgress`
    /// while another replica sees `OtherInProgress`.
    ///
    /// It exists because those two outcomes, and `TakenOver`, cannot otherwise
    /// be exercised deterministically — a test needs to decide who owns what.
    /// **Production should neither share an owner across instances nor persist
    /// one across restarts.** Sharing would not let two replicas proceed
    /// (AD-3h blocks `OwnedInProgress` as well), but it would erase the
    /// difference between self-contention and external contention and would
    /// break lease renewal, which must only renew a lease this instance holds.
    pub fn with_reservation_owner_id(mut self, owner_id: ego_domain::operation::OwnerId) -> Self {
        self.reservation_owner_id = Some(owner_id);
        self
    }

    /// Sets how long a reservation's lease holds. Defaults to 30 seconds.
    ///
    /// **This must exceed the longest a legitimate execution can take.** When a
    /// lease expires another owner may take the reservation over *while the
    /// original is still running*, so until renewal exists a lease shorter than
    /// a real operation permits overlap — a correctness problem, not a tuning
    /// preference. The 30-second default is an operational policy, not a
    /// guarantee that any particular operation fits inside it.
    ///
    /// Zero is rejected when the runtime is built: a lease that expires the
    /// instant it is taken excludes nobody while appearing to work.
    pub fn with_reservation_lease_duration(mut self, lease: std::time::Duration) -> Self {
        self.reservation_lease_duration = lease;
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

    /// Registers the `Tracer` used to produce spans (PROD-003 Phase 4).
    /// `build()` wires a [`TracingInterceptor`] backed by `tracer` into the
    /// runtime's interceptor chain automatically — and ONLY when this method
    /// was called. When never called, the interceptor chain has no
    /// tracing interceptor at all (AD-2-style default): behavior is
    /// byte-for-byte identical to before this change existed, not merely
    /// "traces via a silent `NoopTracer`".
    ///
    /// This is the lifecycle-less path: pass `NoopTracer`, a test spy, or any
    /// `Tracer` implementor that has nothing to shut down. An implementor
    /// that ALSO owns exporter lifecycle (the OTLP adapter, PR5) should use
    /// [`RuntimeBuilder::with_traced`] instead, so its `shutdown()` is never
    /// forgotten.
    pub fn with_tracer(mut self, tracer: Arc<dyn Tracer>) -> Self {
        self.tracer = Some(tracer);
        self
    }

    /// Registers the `TracerLifecycle` owned for a single `shutdown()` on
    /// teardown (design.md ADR-9). `shutdown` is exporter/operational
    /// lifecycle, not a domain tracing call — this is deliberately a
    /// SEPARATE setter from [`RuntimeBuilder::with_tracer`] so `NoopTracer`,
    /// test spies, and any lifecycle-less `Tracer` are never forced to
    /// supply one. `build()` registers `lifecycle.shutdown()` as EXACTLY ONE
    /// async teardown hook (via the same `register_async_teardown` mechanism
    /// [`RuntimeBuilder::register_data_provider`] already uses) — never
    /// twice, even if `Runtime::shutdown_async` itself is called more than
    /// once. When never called, no hook is registered at all.
    pub fn with_tracer_lifecycle(mut self, lifecycle: Arc<dyn TracerLifecycle>) -> Self {
        self.tracer_lifecycle = Some(lifecycle);
        self
    }

    /// Convenience wiring for a single value that implements BOTH `Tracer`
    /// and `TracerLifecycle` — the OTLP adapter's shape (PR5). Sets both
    /// [`RuntimeBuilder::with_tracer`] and
    /// [`RuntimeBuilder::with_tracer_lifecycle`] from the SAME `Arc<T>`,
    /// guaranteeing the registered lifecycle shuts down exactly the tracer
    /// that created the spans — preventing the footgun of wiring a tracer
    /// but forgetting its matching lifecycle (two independent setter calls
    /// that could otherwise be pointed at two different instances).
    pub fn with_traced<T: Tracer + TracerLifecycle + 'static>(self, traced: Arc<T>) -> Self {
        self.with_tracer(traced.clone() as Arc<dyn Tracer>)
            .with_tracer_lifecycle(traced as Arc<dyn TracerLifecycle>)
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
            self.effect_executors
                .register(effect_type, executor.clone())?;
        }
        Ok(self)
    }

    /// Registers `provider` as the sole owner of `provider_id` (CORE-019A
    /// Phase 4, design.md §6). Fails closed on a duplicate `provider_id` —
    /// the already-shipped `ExternalDataProviderRegistry`'s "one owner per
    /// id" contract; the first registration is left untouched.
    ///
    /// Registering at least one provider is what makes [`RuntimeBuilder::build`]
    /// construct a real `RuntimeDataProviderAccess` facade (AD-006's
    /// zero-cost-when-unused gate: no registration → no registry, no facade
    /// constructed). Every *distinct* registered provider's `shutdown()` is
    /// driven, exactly once, by the single owning teardown path `build()`
    /// registers (spec: "Explicit, Single-Owner Lifecycle") — registering the
    /// same `Arc` under two different `provider_id`s (a valid aliasing
    /// pattern, e.g. during a migration) still tears it down only once,
    /// never twice.
    pub fn register_data_provider(
        mut self,
        provider_id: impl Into<String>,
        provider: Arc<dyn ExternalDataProvider>,
    ) -> Result<Self, DuplicateProviderId> {
        let provider_id = provider_id.into();
        self.data_provider_registry
            .register(provider_id.clone(), provider.clone())?;
        let already_tracked = self
            .data_providers_for_teardown
            .iter()
            .any(|tracked| Arc::ptr_eq(tracked, &provider));
        if !already_tracked {
            self.data_providers_for_teardown.push(provider.clone());
        }
        // PROD-005 PR3 TASK-023: recorded UNCONDITIONALLY, unlike the
        // dedup-by-identity teardown list above — health is per registered
        // `provider_id`, so an aliased `Arc` registered under a second id
        // still gets its own entry here.
        self.provider_health_pairs.push((provider_id, provider));
        Ok(self)
    }

    /// Configures the [`ProviderAccessConfig`] (per-attempt timeout + retry
    /// policy) applied uniformly at the provider access chokepoint (issue
    /// #234). Defaults to [`ProviderAccessConfig::default`]; only meaningful
    /// once at least one provider is registered via
    /// [`RuntimeBuilder::register_data_provider`].
    pub fn with_provider_access_config(mut self, config: ProviderAccessConfig) -> Self {
        self.provider_access_config = config;
        self
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

    /// Whether this configuration can honour the idempotency guarantee it declares.
    ///
    /// The **one** definition of that rule. `build` panics on its error and
    /// `try_build` returns it, so the two cannot come to disagree about what a valid
    /// configuration is.
    fn validate_idempotency(&self) -> Result<(), RuntimeError> {
        match self.idempotency_enforcement_mode {
            IdempotencyEnforcementMode::MandatoryKey
                if self.idempotency_reservation_store.is_none() =>
            {
                Err(RuntimeError::OperationReservationStoreNotRegistered)
            }
            _ => Ok(()),
        }
    }

    /// Consumes the builder and produces a [`Runtime`].
    ///
    /// # Panics
    ///
    /// Panics when [`IdempotencyEnforcementMode::MandatoryKey`] is in effect and no
    /// [`OperationReservationStore`] was registered.
    ///
    /// A panic rather than a `Result`, because this signature is what every host and
    /// test already calls and because the alternative is worse than a loud stop: a
    /// runtime that declares mandatory operation keys with nowhere to reserve them
    /// would accept traffic it cannot make idempotent. That is the fail-open the mode
    /// exists to prevent, and it would surface as duplicated business operations
    /// under retry rather than as a startup error. Bootstrap is the cheapest moment to
    /// refuse.
    ///
    /// [`Self::try_build`] returns the same condition as a structured error for a
    /// caller that would rather handle it.
    pub fn build(self) -> Runtime {
        if let Err(err) = self.validate_idempotency() {
            panic!("{err}");
        }
        let security_providers = match (self.authn, self.authz) {
            (Some(authn), Some(authz)) => Some((authn, authz)),
            _ => None,
        };
        let mut teardown = TeardownStack::new();
        if let Some(logger) = &self.logger {
            teardown.push(logger.clone());
        }

        // CORE-019 Phase 9 zero-cost gate (design.md §8/§20), re-separated at
        // this layer per PR4 review F-01: construct the external-effects
        // store/queue/acceptor ONLY when at least one executor was
        // registered — but never call `.start()` here. `build()` is a plain
        // synchronous method a caller may legitimately invoke during a sync
        // bootstrap phase, before any Tokio runtime exists yet;
        // `RuntimeEffectAcceptor::start()` performs a real `tokio::spawn`,
        // which panics with no active Tokio runtime context. Only
        // `RuntimeEffectAcceptor::new()` — safe outside Tokio, per PR3's own
        // `new`/`start` split — runs here. [`Runtime::start_effects`] is the
        // new, explicit async entry point a host calls once, after entering
        // Tokio, to actually spawn the `Deferred`-mode drain loop.
        let effect_acceptor_impl = if self.effect_executors.is_empty() {
            None
        } else {
            let store = Arc::new(InMemoryEffectStore::new());
            Some(Arc::new(RuntimeEffectAcceptor::new(
                store.clone() as Arc<dyn EffectStateStore>,
                store as Arc<dyn EffectDedupStore>,
                Arc::new(self.effect_executors),
                self.delivery_config,
            )))
        };

        // CORE-019A Phase 4 zero-cost gate (AD-006), mirroring the
        // effect-executors gate above: construct the
        // `RuntimeDataProviderAccess` facade ONLY when at least one provider
        // was registered. Unlike the effects acceptor, there is no separate
        // `start()` step to re-separate — `RuntimeDataProviderAccess` never
        // spawns a task, so it is immediately usable once built.
        let data_provider_access: Option<Arc<dyn DataProviderAccess>> =
            if self.data_provider_registry.is_empty() {
                None
            } else {
                Some(Arc::new(RuntimeDataProviderAccess::with_config(
                    self.data_provider_registry,
                    self.provider_access_config,
                )))
            };
        let data_providers_for_teardown = self.data_providers_for_teardown;

        // PROD-003 Phase 4 (TASK-013/014): wire a `TracingInterceptor` into
        // the interceptor chain ONLY when a `tracer` was registered. Omitted
        // ⇒ the chain built here is exactly the same empty
        // `InterceptorChain::new()` `RuntimeBuilder::new()` already produces
        // (there is no other way to populate `self.interceptor_chain` before
        // `build()` today) — byte-identical to before this change existed,
        // not a `NoopTracer`-backed interceptor running for nothing.
        let interceptor_chain = if let Some(tracer) = &self.tracer {
            let mut chain = InterceptorChain::new();
            chain.add_interceptor(Arc::new(TracingInterceptor::new(tracer.clone())));
            Arc::new(chain)
        } else {
            self.interceptor_chain
        };
        let tracer_lifecycle = self.tracer_lifecycle;

        // PROD-005 PR2 (TASK-018/019): fold every registered lifecycle
        // component's health contributors into ONE runtime-owned registry.
        // A component contributing none (the `LifecycleManaged` default)
        // leaves aggregation unaffected — this is the zero-cost path when no
        // component ever registers a contributor.
        let mut health_contributors: Vec<Arc<dyn ego_domain::health::HealthContributor>> = self
            .lifecycle_components
            .iter()
            .flat_map(|component| component.health_contributors())
            .collect();
        // PROD-005 PR3 TASK-023 (single registration authority): every
        // `register_data_provider` call also contributes a
        // `ProviderHealthContributor`, keyed by the SAME `provider_id` it was
        // registered under, into this SAME vec — never a second, separately
        // owned registry/aggregator. Uses `provider_health_pairs`
        // (unconditionally per-id), never `data_providers_for_teardown`
        // (identity-deduplicated), so an aliased provider registered under
        // two ids still yields two independent contributors.
        health_contributors.extend(self.provider_health_pairs.into_iter().map(
            |(provider_id, provider)| {
                Arc::new(ego_runtime::providers::ProviderHealthContributor::new(
                    provider_id,
                    provider,
                )) as Arc<dyn ego_domain::health::HealthContributor>
            },
        ));
        // The registered reservation store contributes its own readiness, from
        // the SAME `Arc` handed to `RuntimeInner` below — `Arc::clone` of the
        // field, never a second construction and never a second read of the
        // configuration. Two instances built from one config would be a store
        // that dispatch reserves through and a *different* store that readiness
        // reports on, and the report would be true about the wrong thing.
        //
        // Keyed on the store being present, not on the enforcement mode. A
        // `Compatibility` runtime with no store registered adds no contributor
        // at all and stays ready, which is correct: it never promised to
        // reserve anything, so there is no dependency to be down. A
        // `Compatibility` runtime that *did* register one is still dispatching
        // through it, so it is still a real dependency and is checked. The
        // remaining combination — `MandatoryKey` with no store — cannot reach
        // this line, because `validate_idempotency` already refused the build.
        if let Some(store) = &self.idempotency_reservation_store {
            health_contributors.push(Arc::new(
                crate::health::OperationReservationStoreHealthContributor::new(Arc::clone(store)),
            )
                as Arc<dyn ego_domain::health::HealthContributor>);
        }
        let health_aggregator = Arc::new(HealthAggregator::new(
            HealthRegistry::from_contributors(health_contributors),
            HealthAggregationConfig::default(),
        ));

        // AD-3i: the four pieces are assembled once, here, or not at all. A
        // deployment without a store has no reservation capability rather than a
        // half-configured one — which is why `RuntimeInner` holds
        // `Option<ReservationConfig>` and the config itself has no optional
        // fields.
        let reservation = match self.idempotency_reservation_store.clone() {
            None => None,
            Some(store) => {
                let clock = self
                    .reservation_clock
                    .clone()
                    .unwrap_or_else(|| Arc::new(ego_domain::time::SystemClock));
                // Minted once, here, so every operation this runtime reserves
                // carries the same identity and a restart carries a different
                // one.
                let owner = self.reservation_owner_id.clone().unwrap_or_else(|| {
                    ego_domain::operation::OwnerId::new(uuid::Uuid::new_v4().to_string())
                });
                Some(
                    crate::runtime::idempotency::ReservationConfig::new(
                        store,
                        clock,
                        owner,
                        self.reservation_lease_duration,
                    )
                    .expect(
                        "the reservation lease duration must be greater than zero: a zero \
                         lease expires the instant it is taken, so every attempt would see \
                         the previous one as expired and take it over — the reservation \
                         would exclude nobody while appearing to work",
                    ),
                )
            }
        };

        let runtime = Runtime {
            health_aggregator,
            inner: Arc::new(RuntimeInner::new_with_logger(
                self.registry,
                interceptor_chain,
                security_providers,
                DependencyTable::with_registrations(RegisteredDependencies {
                    adapters: self.adapters,
                    configs: self.configs,
                    projections: self.projections,
                    entities: self.entities,
                }),
                self.logger,
                Mutex::new(teardown),
                TenantResolver::new(self.tenant_enforcement_mode),
                // Exactly the value this build was validated against — see the
                // check above, which refuses MandatoryKey with no store. Passing
                // it on rather than dropping it is what lets a transport apply
                // the same policy instead of choosing its own.
                self.idempotency_enforcement_mode,
                reservation,
                self.observability,
                effect_acceptor_impl,
                self.effect_drain_deadline,
                data_provider_access,
            )),
        };

        // Single owning teardown path (spec: "Explicit, Single-Owner
        // Lifecycle") — every registered provider's `shutdown()` runs
        // exactly once, through the same `register_async_teardown`/
        // `shutdown_async` mechanism the effects subsystem already uses.
        // A no-op registration when no provider was ever registered — the
        // zero-cost path incurs no extra hook.
        //
        // Issue #242: each provider's `shutdown()` is isolated with
        // `catch_unwind`, so a panic in one provider does not prevent the
        // others from shutting down — the loop continues and any panic is
        // surfaced afterwards as `RuntimeInfraError::Teardown`. The panic
        // payload is dropped (`.is_err()` only), never logged or embedded; the
        // `reason` is a fixed string. The exactly-once drain (`shutdown_async`
        // takes the hook list via `mem::take`) and the `Arc::ptr_eq` alias
        // dedup at registration are both preserved — this only wraps the call
        // inside the loop.
        if !data_providers_for_teardown.is_empty() {
            runtime.register_async_teardown(async move {
                let mut any_panicked = false;
                for provider in data_providers_for_teardown {
                    // The call is deferred to poll-time INSIDE the guard (an
                    // inner `async` block, not eager `provider.shutdown()` as an
                    // argument), so a manual `ExternalDataProvider` impl that
                    // panics *synchronously* during future construction — before
                    // returning its pinned future — is caught here too, and the
                    // loop still continues to the remaining providers.
                    if AssertUnwindSafe(async { provider.shutdown().await })
                        .catch_unwind()
                        .await
                        .is_err()
                    {
                        any_panicked = true;
                    }
                }
                if any_panicked {
                    Err(RuntimeInfraError::Teardown {
                        reason: "a data provider panicked during shutdown".to_string(),
                    })
                } else {
                    Ok(())
                }
            });
        }

        // PROD-003 Phase 4 (TASK-014): register the tracer's lifecycle
        // shutdown as EXACTLY ONE async teardown hook — never twice, even if
        // `Runtime::shutdown_async` is triggered more than once, because
        // `shutdown_async` drains `async_teardown` via `mem::take`: a hook
        // runs at most once no matter how many times `shutdown_async` itself
        // is called (a second call finds an already-emptied hook list). No
        // hook is registered at all when no `tracer_lifecycle` was set — the
        // zero-cost path incurs no extra hook, mirroring the data-provider
        // teardown gate above.
        if let Some(lifecycle) = tracer_lifecycle {
            runtime.register_async_teardown(async move {
                lifecycle.shutdown();
                Ok(())
            });
        }

        runtime
    }

    /// Consumes the builder and produces a [`Runtime`], first running every
    /// `with_injectable`-recorded validator against the freshly built
    /// runtime's resolved tables. Fails fast on the first missing
    /// dependency, naming both the missing type and the requesting service
    /// (AD-3/AD-4). Calls the existing infallible [`Self::build`] unchanged
    /// — `Injectable::build` is never invoked here, only `Injectable::validate`.
    pub fn try_build(mut self) -> Result<Runtime, RuntimeError> {
        // Before delegating, not after. `build` panics on this condition, so checking
        // afterwards would mean this method could never return the error it exists to
        // return — the panic would already have unwound.
        self.validate_idempotency()?;
        let validators = std::mem::take(&mut self.validators);
        let rt = self.build();
        for (service_name, validate) in validators {
            if let Err(err) = validate(rt.inner()) {
                let err = match err {
                    RuntimeError::DependencyNotFound {
                        kind, type_name, ..
                    } => RuntimeError::DependencyNotFound {
                        kind,
                        type_name,
                        service_name: Some(service_name),
                    },
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
///
/// `Clone` (CORE-028 Stage 1 PR2): cheap — clones only the inner `Arc`, the
/// same shared state. This is the full infra-level handle — every direct
/// consumer of [`RuntimeBuilder`] (tests, low-level hosts) is expected to use
/// it, lifecycle methods included (AD-1/G2: `RuntimeBuilder`/`Runtime` is the
/// infrastructure API). A caller that only needs per-request resolution
/// (e.g. a transport layer's `AppState`) should hold a [`RuntimeResolver`]
/// (via [`Runtime::resolver`]) instead, not this type — see `RuntimeResolver`
/// for why.
#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
    /// The runtime-owned health aggregator (PROD-005 PR2 TASK-018/019),
    /// built once by [`RuntimeBuilder::build`] from every registered
    /// lifecycle component's `health_contributors()`. Cheap to clone (an
    /// `Arc`), consistent with the rest of this handle.
    health_aggregator: Arc<HealthAggregator>,
}

impl Runtime {
    /// Returns a reference to the inner [`RuntimeInner`].
    pub fn inner(&self) -> &Arc<RuntimeInner> {
        &self.inner
    }

    /// Process-internal liveness check (PROD-005 PR2 TASK-014/015).
    ///
    /// Takes NO registry/aggregator argument and consults NO contributor —
    /// liveness answers only "is the process alive and able to make
    /// progress", never "are my dependencies healthy" (that's
    /// [`Runtime::readiness`]/[`Runtime::startup`]). Always
    /// [`ego_domain::health::HealthStatus::Healthy`] with an empty
    /// contributor list, unaffected by anything registered on this runtime's
    /// [`HealthAggregator`].
    pub fn liveness(&self) -> ego_domain::health::HealthReport {
        ego_domain::health::HealthReport {
            probe: ego_domain::health::ProbeKind::Liveness,
            status: ego_domain::health::HealthStatus::Healthy,
            contributors: Vec::new(),
        }
    }

    /// Evaluates every registered lifecycle component's health contributors
    /// and folds them into a readiness [`ego_domain::health::HealthReport`]
    /// (PROD-005 PR2 TASK-018/019). Delegates entirely to this runtime's
    /// [`HealthAggregator`] — see [`HealthAggregator::readiness`].
    pub async fn readiness(&self) -> ego_domain::health::HealthReport {
        self.health_aggregator.readiness().await
    }

    /// Evaluates every registered lifecycle component's health contributors
    /// and folds them into a startup [`ego_domain::health::HealthReport`]
    /// (PROD-005 PR2 TASK-018/019). Uses the IDENTICAL fold as
    /// [`Runtime::readiness`] — see [`HealthAggregator::startup`].
    pub async fn startup(&self) -> ego_domain::health::HealthReport {
        self.health_aggregator.startup().await
    }

    /// Returns the registered security providers, if any.
    pub fn security_providers(&self) -> Option<&SecurityProviders> {
        self.inner.security_providers.as_ref()
    }

    /// The idempotency policy this runtime was built and validated under.
    ///
    /// Exposed so a transport can apply the same policy the build was checked
    /// against, rather than carrying a second copy of the configuration that
    /// could drift from it. Read it to *pass it on* — the policy table has one
    /// owner, `resolve_operation_key`.
    pub fn idempotency_enforcement_mode(&self) -> IdempotencyEnforcementMode {
        self.inner.idempotency_enforcement_mode()
    }

    /// Returns the registered logger, if any.
    pub fn logger(&self) -> Option<&Arc<KITLogger>> {
        self.inner.logger()
    }

    /// Spawns the external-effects `Deferred`-mode drain loop (if any
    /// executor was registered) and registers its drain-on-shutdown teardown
    /// hook.
    ///
    /// **CORE-019 PR4 review F-01 fix:** [`RuntimeBuilder::build`] only ever
    /// *constructs* the effects subsystem (`RuntimeEffectAcceptor::new`, safe
    /// outside Tokio) — it deliberately never calls `.start()`, which
    /// performs a real `tokio::spawn` and panics with no active Tokio
    /// runtime context. A host that registered at least one executor MUST
    /// call this method exactly once, from inside an active Tokio runtime,
    /// after `build()` and before relying on [`Runtime::effect_acceptor`] to
    /// return `Some` — e.g. from an async `main`, right after constructing
    /// the `Runtime` and before serving traffic.
    ///
    /// A no-op returning `Ok(())` in the zero-cost path (no executor
    /// registered) and on every call after the first — idempotent, calling
    /// this twice never double-spawns the runner task.
    ///
    /// Until this is called, [`Runtime::effect_acceptor`] returns `None`
    /// even though an executor was registered. This is deliberate, not a
    /// regression: a caller who never calls `start_effects` can never
    /// obtain (and therefore never silently use) an acceptor whose
    /// `Deferred`-mode runner task was never spawned — closing the "effects
    /// accepted into a queue nobody ever drains" gap a synchronous,
    /// auto-starting `build()` used to leave open.
    pub async fn start_effects(&self) -> Result<(), RuntimeInfraError> {
        let Some(acceptor) = self.inner.effect_acceptor_impl.clone() else {
            return Ok(());
        };
        if self
            .inner
            .effect_started
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_err()
        {
            // Already started by a previous call — idempotent no-op rather
            // than double-spawning the runner task.
            return Ok(());
        }

        let handle = acceptor.start();
        let deadline = self.inner.effect_drain_deadline;
        // Same "a failing hook surfaces through shutdown_async" contract
        // Finding 6/F-02 already established: `EffectRuntimeHandle::
        // shutdown_and_wait`'s `Result` is propagated, never swallowed.
        self.register_async_teardown(async move {
            handle
                .shutdown_and_wait(deadline)
                .await
                .map_err(|err| RuntimeInfraError::Teardown {
                    reason: format!(
                        "external-effects drain deadline reached before shutdown completed \
                     cleanly (drain_incomplete): {err}"
                    ),
                })
        });
        Ok(())
    }

    /// Returns the external-effects [`EffectAcceptor`] started via
    /// [`Runtime::start_effects`], if at least one executor was registered
    /// AND `start_effects` has actually run (CORE-019 Phase 9; PR4 review
    /// F-01). `None` both in the zero-cost path (design.md §8/§20 — no
    /// store, no queue, no acceptor was ever constructed) and in the
    /// constructed-but-not-yet-started path — a caller cannot obtain (and so
    /// cannot silently use) an acceptor whose `Deferred`-mode runner was
    /// never spawned.
    ///
    /// This is the seam a host wires into its entity-level runtime (e.g.
    /// `persistent_entity::builder::EntityRuntimeBuilder`) so spawned actors
    /// stop silently discarding described effects — that host-side plumbing
    /// is out of `ego-service-sdk`'s scope (it lives wherever the host
    /// constructs its `EntityRuntimeBuilder`/`EntityRuntime`).
    pub fn effect_acceptor(&self) -> Option<Arc<dyn EffectAcceptor>> {
        if !self
            .inner
            .effect_started
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return None;
        }
        self.inner
            .effect_acceptor_impl
            .as_ref()
            .map(|acceptor| acceptor.clone() as Arc<dyn EffectAcceptor>)
    }

    /// Returns the external-data-provider [`DataProviderAccess`] facade
    /// built via [`RuntimeBuilder::register_data_provider`] (CORE-019A Phase
    /// 4), if at least one provider was registered. `None` in the zero-cost
    /// path (AD-006) — no registry, no facade was ever constructed. Unlike
    /// [`Runtime::effect_acceptor`], this is available immediately after
    /// `build()` — there is no separate `start_effects`-style step, since
    /// `RuntimeDataProviderAccess` never spawns a task.
    pub fn data_provider_access(&self) -> Option<Arc<dyn DataProviderAccess>> {
        self.inner.data_provider_access.clone()
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
            .resolve_raw::<Tag>(&VersionConstraint::Exact(
                <Tag as ServiceContract>::version(),
            ))
            .map_err(|_| RuntimeError::ServiceNotFound {
                type_name: std::any::type_name::<Tag>(),
                required_by: None,
            })?;
        Tag::create_proxy(
            raw,
            self.inner.interceptor_chain.clone(),
            Arc::downgrade(&self.inner),
        )
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

    /// Returns a [`RuntimeResolver`] — a resolution-only view onto this
    /// `Runtime` (CORE-028 Stage 1 PR2 review). Deliberately narrower than
    /// handing out this `Runtime` itself: `start_effects`/`shutdown_async`/
    /// `register_async_teardown` are the lifecycle surface `App`/`RunningApp`
    /// own, and a caller that only resolves services per request (e.g. a
    /// transport layer) has no legitimate reason to reach them through a
    /// side channel.
    pub fn resolver(&self) -> RuntimeResolver {
        RuntimeResolver {
            runtime: self.clone(),
        }
    }
}

/// A resolution-only handle into a [`Runtime`] (CORE-028 Stage 1 PR2 review
/// finding, HIGH): `App::runtime()` used to hand out a full `Runtime`, which
/// let a transport-layer caller call `start_effects`/`shutdown_async`/
/// `register_async_teardown` directly — bypassing the `App`/`RunningApp`
/// typestate Stage 1 introduced specifically to make those lifecycle
/// transitions type-checked. `RuntimeResolver` exposes only what
/// `ego_transport::AppState`'s per-request dispatch actually needs
/// (`resolve`, `logger`) and nothing from the lifecycle surface. Cheap to
/// clone — wraps a `Runtime`, which itself only clones an `Arc`.
#[derive(Clone)]
pub struct RuntimeResolver {
    runtime: Runtime,
}

impl RuntimeResolver {
    /// Resolves `Tag` to its concrete macro-generated proxy — identical to
    /// [`Runtime::resolve`], the only difference is what else is (not)
    /// reachable from this handle.
    pub fn resolve<Tag>(&self) -> Result<Tag::Proxy, RuntimeError>
    where
        Tag: Resolvable + 'static,
    {
        self.runtime.resolve::<Tag>()
    }

    /// The idempotency policy this runtime was built under — identical to
    /// [`Runtime::idempotency_enforcement_mode`]. This is the accessor the HTTP
    /// operation-key extractor reads.
    pub fn idempotency_enforcement_mode(&self) -> IdempotencyEnforcementMode {
        self.runtime.idempotency_enforcement_mode()
    }

    /// Returns the registered logger, if any — identical to [`Runtime::logger`].
    pub fn logger(&self) -> Option<&Arc<KITLogger>> {
        self.runtime.logger()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ego_security_sdk::authentication::AuthenticationProvider;
    use ego_security_sdk::authorization::{AuthorizationDecision, AuthorizationProvider};
    use ego_security_sdk::context::SecurityContext;
    use ego_security_sdk::credential::Credential;
    use ego_security_sdk::error::SecurityError;
    use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};
    use ego_security_sdk::AuthenticationError;
    use kitlogger::KITLogger;

    use super::{IdempotencyEnforcementMode, Runtime, RuntimeBuilder};
    use crate::runtime::{RuntimeError, RuntimeInfraError};

    /// A builder that has explicitly not adopted idempotency enforcement.
    ///
    /// Every test below whose subject is *not* idempotency uses this, so the
    /// fail-closed default does not turn each of them into a statement about a topic
    /// it is not testing. It is `#[cfg(test)]` and local: nothing production-facing
    /// can reach it, and no example or host bootstrap can quietly inherit a relaxed
    /// mode from it.
    ///
    /// The tests that *are* about the default deliberately do not use it — they call
    /// `RuntimeBuilder::new()` directly, because a helper that pre-answers the
    /// question is no way to test the answer.
    fn compat() -> RuntimeBuilder {
        RuntimeBuilder::new()
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
    }

    /// A reservation store that refuses everything.
    ///
    /// Registration is the only thing under test here, so the store never needs to
    /// work — and one that errors is safer than one that pretends: if a future change
    /// made a build-time check actually call it, the failure is loud rather than a
    /// silently accepted reservation.
    struct UnusableReservationStore;

    #[async_trait]
    impl ego_domain::operation::OperationReservationStore for UnusableReservationStore {
        async fn reserve(
            &self,
            _req: ego_domain::operation::ReserveRequest,
        ) -> Result<
            ego_domain::operation::ReservationOutcome,
            ego_domain::operation::ReservationError,
        > {
            Err(ego_domain::operation::ReservationError::Backend(
                "registration-only stub".to_string(),
            ))
        }

        async fn renew(
            &self,
            _fence: &ego_domain::operation::OwnerFence,
            _until: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), ego_domain::operation::ReservationError> {
            Err(ego_domain::operation::ReservationError::Backend(
                "registration-only stub".to_string(),
            ))
        }

        async fn complete(
            &self,
            _fence: &ego_domain::operation::OwnerFence,
            _response: ego_domain::operation::StoredServiceResponse,
        ) -> Result<(), ego_domain::operation::ReservationError> {
            Err(ego_domain::operation::ReservationError::Backend(
                "registration-only stub".to_string(),
            ))
        }

        async fn abandon(
            &self,
            _fence: &ego_domain::operation::OwnerFence,
        ) -> Result<(), ego_domain::operation::ReservationError> {
            Err(ego_domain::operation::ReservationError::Backend(
                "registration-only stub".to_string(),
            ))
        }

        async fn purge_completed_before(
            &self,
            _cutoff: chrono::DateTime<chrono::Utc>,
            _batch: usize,
        ) -> Result<u64, ego_domain::operation::ReservationError> {
            Err(ego_domain::operation::ReservationError::Backend(
                "registration-only stub".to_string(),
            ))
        }

        async fn probe(&self) -> Result<(), ego_domain::operation::ReservationError> {
            Err(ego_domain::operation::ReservationError::Backend(
                "registration-only stub".to_string(),
            ))
        }
    }

    /// A reservation store whose `probe` answers from a script and counts its calls.
    ///
    /// The count is what makes instance identity provable: a contributor wired to a
    /// second store built from the same configuration would leave this one at zero
    /// while still reporting perfectly healthy. Its five real methods panic, so a
    /// readiness check that reserved, renewed or purged anything fails loudly rather
    /// than passing while mutating the table it is supposed to be observing.
    struct ProbeCountingStore {
        outcome: std::sync::Mutex<Result<(), ego_domain::operation::ReservationError>>,
        probes: std::sync::atomic::AtomicUsize,
    }

    impl ProbeCountingStore {
        fn new(outcome: Result<(), ego_domain::operation::ReservationError>) -> Arc<Self> {
            Arc::new(Self {
                outcome: std::sync::Mutex::new(outcome),
                probes: std::sync::atomic::AtomicUsize::new(0),
            })
        }

        fn probes(&self) -> usize {
            self.probes.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// Changes what the next probe answers — the store going down, or coming
        /// back, without the runtime being rebuilt.
        fn set(&self, outcome: Result<(), ego_domain::operation::ReservationError>) {
            *self.outcome.lock().expect("probe outcome mutex poisoned") = outcome;
        }
    }

    #[async_trait]
    impl ego_domain::operation::OperationReservationStore for ProbeCountingStore {
        async fn reserve(
            &self,
            _req: ego_domain::operation::ReserveRequest,
        ) -> Result<
            ego_domain::operation::ReservationOutcome,
            ego_domain::operation::ReservationError,
        > {
            panic!("a readiness probe must never reserve an operation");
        }

        async fn renew(
            &self,
            _fence: &ego_domain::operation::OwnerFence,
            _until: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), ego_domain::operation::ReservationError> {
            panic!("a readiness probe must never renew a lease");
        }

        async fn complete(
            &self,
            _fence: &ego_domain::operation::OwnerFence,
            _response: ego_domain::operation::StoredServiceResponse,
        ) -> Result<(), ego_domain::operation::ReservationError> {
            panic!("a readiness probe must never complete a reservation");
        }

        async fn abandon(
            &self,
            _fence: &ego_domain::operation::OwnerFence,
        ) -> Result<(), ego_domain::operation::ReservationError> {
            panic!("a readiness probe must never abandon a reservation");
        }

        async fn purge_completed_before(
            &self,
            _cutoff: chrono::DateTime<chrono::Utc>,
            _batch: usize,
        ) -> Result<u64, ego_domain::operation::ReservationError> {
            panic!("a readiness probe must never purge reservations");
        }

        async fn probe(&self) -> Result<(), ego_domain::operation::ReservationError> {
            self.probes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.outcome
                .lock()
                .expect("probe outcome mutex poisoned")
                .clone()
        }
    }

    /// The contributor's report within a readiness fold, by name.
    fn reservation_store_report(
        report: &ego_domain::health::HealthReport,
    ) -> Option<&ego_domain::health::ContributorReport> {
        report
            .contributors
            .iter()
            .find(|c| c.name == crate::health::OPERATION_RESERVATION_STORE_CONTRIBUTOR)
    }

    // ---- The fail-closed default (B3.6) ---------------------------------------

    /// The default mode with no store registered refuses to build.
    ///
    /// `RuntimeBuilder::new()` directly, not the `compat` helper: the subject is what
    /// an unconfigured builder does, so pre-configuring it would test nothing.
    ///
    /// A runtime built here would declare that every mutating operation carries a
    /// client-supplied key and have nowhere to reserve one. It would accept traffic it
    /// cannot make idempotent, and the symptom would be duplicated business
    /// operations under retry rather than a startup error.
    #[test]
    #[should_panic(expected = "no OperationReservationStore is registered")]
    fn the_default_mode_without_a_reservation_store_refuses_to_build() {
        let _ = RuntimeBuilder::new().build();
    }

    /// The same condition through `try_build`, as a structured error rather than a
    /// panic.
    ///
    /// Both paths must agree, and they do because there is one validation: `build`
    /// panics on its error and `try_build` returns it. This also pins the ordering —
    /// `try_build` delegates to `build`, so it has to validate *before* delegating or
    /// the panic would unwind before the error could be returned.
    #[test]
    fn try_build_reports_the_missing_reservation_store_as_an_error() {
        match RuntimeBuilder::new().try_build() {
            Err(RuntimeError::OperationReservationStoreNotRegistered) => {}
            Ok(_) => panic!("try_build must not produce a runtime that cannot enforce its mode"),
            Err(other) => panic!("expected OperationReservationStoreNotRegistered, got {other:?}"),
        }
    }

    /// The error names both ways out, because either can be the right one.
    #[test]
    fn the_refusal_names_the_registration_and_the_opt_out() {
        let message = RuntimeBuilder::new()
            .try_build()
            .err()
            .expect("the default without a store must fail")
            .to_string();

        assert!(
            message.contains(".with_operation_reservation_store("),
            "the error must name the registration that fixes it: {message}"
        );
        assert!(
            message.contains("Compatibility"),
            "the error must also name the explicit opt-out, since a deployment that has \
             genuinely not adopted enforcement needs a way to say so: {message}"
        );
    }

    /// Compatibility is the explicit way to build without a store.
    #[test]
    fn compatibility_mode_without_a_store_builds() {
        let rt = RuntimeBuilder::new()
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .build();
        assert!(rt.security_providers().is_none());
    }

    /// The runtime retains the very store that was registered.
    ///
    /// `Arc::ptr_eq`, not "a store is present": registration that type-checks and then
    /// drops the value on the floor would satisfy any weaker assertion, and the whole
    /// point of the setter is that idempotent dispatch later reaches *this* instance.
    /// A store that vanishes at build time makes the method nominal and leaves the
    /// dispatch it exists for with nothing to call.
    #[test]
    fn the_registered_reservation_store_is_the_one_the_runtime_keeps() {
        let store: Arc<dyn ego_domain::operation::OperationReservationStore> =
            Arc::new(UnusableReservationStore);
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(Arc::clone(&store))
            .build();

        let retained = rt
            .inner()
            .reservation()
            .map(|r| r.store())
            .expect("an enforcing runtime must retain the store it was given");
        assert!(
            Arc::ptr_eq(retained, &store),
            "the runtime must hold the registered instance, not merely some store"
        );
    }

    /// Compatibility without a store retains nothing.
    ///
    /// So a caller finding `None` knows enforcement is off, rather than that a
    /// registration was missed — the builder refuses to produce an enforcing runtime
    /// without one, which is what makes that reading sound.
    #[test]
    fn compatibility_without_a_store_retains_none() {
        let rt = RuntimeBuilder::new()
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .build();
        assert!(rt.inner().reservation().is_none());
    }

    /// Registering twice keeps the second, observed through the retained instance.
    ///
    /// Two stores would mean two places a key could be reserved and no answer to which
    /// one decides. Asserting only that the build succeeds would leave that unpinned:
    /// keeping the first, keeping the second, and keeping neither all build fine.
    #[test]
    fn registering_a_second_reservation_store_replaces_the_first() {
        let first: Arc<dyn ego_domain::operation::OperationReservationStore> =
            Arc::new(UnusableReservationStore);
        let second: Arc<dyn ego_domain::operation::OperationReservationStore> =
            Arc::new(UnusableReservationStore);
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(Arc::clone(&first))
            .with_operation_reservation_store(Arc::clone(&second))
            .build();

        let retained = rt
            .inner()
            .reservation()
            .map(|r| r.store())
            .expect("a store was registered");
        assert!(
            Arc::ptr_eq(retained, &second),
            "the second registration must replace the first"
        );
        assert!(
            !Arc::ptr_eq(retained, &first),
            "the first registration must not survive alongside it"
        );
    }

    // ---- Readiness for the registered store (B3.7) -----------------------------
    //
    // B3.6 above covers "no store registered at all": the build is refused and no
    // runtime exists. These cover the other failure, which cannot be decided at
    // startup — the store is registered, the process started, and the backing store
    // has since become unreachable.

    /// Registering a store registers its readiness contributor.
    ///
    /// The wiring is one `push` and is easy to omit while everything else keeps
    /// working: dispatch would reserve through the store, readiness would report on
    /// nothing, and the instance would keep taking traffic straight through an
    /// outage. Nothing else in the suite would notice, which is why this asserts the
    /// contributor is present by name rather than only that the report is healthy.
    #[tokio::test]
    async fn registering_a_reservation_store_registers_its_readiness_contributor() {
        let store = ProbeCountingStore::new(Ok(()));
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(
                store.clone() as Arc<dyn ego_domain::operation::OperationReservationStore>
            )
            .build();

        let report = rt.readiness().await;

        let contributor = reservation_store_report(&report).unwrap_or_else(|| {
            panic!(
                "a registered store must contribute to readiness; present: {:?}",
                report
                    .contributors
                    .iter()
                    .map(|c| &c.name)
                    .collect::<Vec<_>>()
            )
        });
        assert_eq!(
            contributor.status,
            ego_domain::health::HealthStatus::Healthy
        );
        assert_eq!(
            contributor.requirement,
            ego_domain::health::DependencyRequirement::Required
        );
        assert_eq!(report.status, ego_domain::health::HealthStatus::Healthy);
    }

    /// Readiness probes the exact instance the runtime dispatches through.
    ///
    /// Two assertions, because either alone is satisfiable by the wrong wiring. The
    /// probe count proves the contributor reached *this* object — a contributor built
    /// from a second store over the same configuration would leave it at zero while
    /// reporting a perfectly healthy, entirely unrelated connection. `Arc::ptr_eq`
    /// proves the runtime kept the same one, so the thing that was probed is the
    /// thing a reservation will go through.
    #[tokio::test]
    async fn readiness_probes_the_same_store_instance_the_runtime_retained() {
        let store = ProbeCountingStore::new(Ok(()));
        let registered = store.clone() as Arc<dyn ego_domain::operation::OperationReservationStore>;
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(Arc::clone(&registered))
            .build();

        assert_eq!(store.probes(), 0, "building must not probe the store");

        rt.readiness().await;

        assert_eq!(
            store.probes(),
            1,
            "readiness must probe the registered instance itself, not a second store \
             built from the same configuration"
        );
        let retained = rt
            .inner()
            .reservation()
            .map(|r| r.store())
            .expect("an enforcing runtime retains its store");
        assert!(
            Arc::ptr_eq(retained, &registered),
            "the probed instance and the dispatched instance must be one object"
        );
    }

    /// An unreachable store makes the runtime not ready.
    ///
    /// The error is reported as `Unavailable` and, because the store is a `Required`
    /// dependency, it clamps the whole report to `Unhealthy` rather than degrading
    /// it. Serving while it is down means a retried request cannot be recognised as
    /// a retry and gets executed a second time.
    #[tokio::test]
    async fn a_store_that_cannot_be_reached_makes_the_runtime_not_ready() {
        let store = ProbeCountingStore::new(Err(ego_domain::operation::ReservationError::Backend(
            "connection refused".to_string(),
        )));
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(
                store as Arc<dyn ego_domain::operation::OperationReservationStore>,
            )
            .build();

        let report = rt.readiness().await;

        assert_eq!(
            report.status,
            ego_domain::health::HealthStatus::Unhealthy,
            "a store error must never fold in as ready"
        );
        let contributor =
            reservation_store_report(&report).expect("the contributor must still be reported");
        assert_eq!(
            contributor.code,
            Some(ego_domain::health::HealthCode::Unavailable)
        );
    }

    /// The store's error text never reaches the readiness report.
    ///
    /// A driver's connection error routinely quotes the DSN it failed on, and a DSN
    /// routinely carries a password. Readiness payloads are commonly served
    /// unauthenticated, so this asserts on the whole rendered report rather than on
    /// the contributor's code alone.
    #[tokio::test]
    async fn a_store_failure_never_puts_credentials_in_the_readiness_report() {
        let store = ProbeCountingStore::new(Err(ego_domain::operation::ReservationError::Backend(
            "error connecting to postgres://ego:sup3r-s3cret@db.internal:5432/ego".to_string(),
        )));
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(
                store as Arc<dyn ego_domain::operation::OperationReservationStore>,
            )
            .build();

        let rendered = format!("{:?}", rt.readiness().await);

        assert!(
            !rendered.contains("sup3r-s3cret"),
            "the readiness report must not carry the store's error text: {rendered}"
        );
        assert!(
            !rendered.contains("db.internal"),
            "the readiness report must not carry the store's connection detail: {rendered}"
        );
    }

    /// Compatibility with no store registered is ready, and contributes nothing.
    ///
    /// It never promised to reserve anything, so there is no dependency to be down.
    /// A contributor that reported the *absence* of a store as a failure would make
    /// every not-yet-adopted deployment permanently un-ready — turning an explicit,
    /// supported opt-out into an outage.
    #[tokio::test]
    async fn compatibility_without_a_store_is_ready_and_contributes_nothing() {
        let rt = RuntimeBuilder::new()
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .build();

        let report = rt.readiness().await;

        assert_eq!(
            report.status,
            ego_domain::health::HealthStatus::Healthy,
            "declining enforcement is a supported configuration, not a failure"
        );
        assert!(
            reservation_store_report(&report).is_none(),
            "with no store registered there is nothing to report on: {:?}",
            report.contributors
        );
    }

    /// Compatibility *with* a store still checks it.
    ///
    /// The wiring keys on the store being present, not on the mode, and this is why:
    /// a `Compatibility` runtime that registered one is still dispatching through it,
    /// so it is still a real dependency. Keying on the mode instead would leave that
    /// deployment reporting ready with its store down.
    #[tokio::test]
    async fn compatibility_with_a_registered_store_still_checks_it() {
        let store = ProbeCountingStore::new(Err(ego_domain::operation::ReservationError::Backend(
            "connection refused".to_string(),
        )));
        let rt = RuntimeBuilder::new()
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .with_operation_reservation_store(
                store as Arc<dyn ego_domain::operation::OperationReservationStore>,
            )
            .build();

        let report = rt.readiness().await;

        assert!(
            reservation_store_report(&report).is_some(),
            "a registered store is a dependency regardless of the enforcement mode"
        );
        assert_eq!(report.status, ego_domain::health::HealthStatus::Unhealthy);
    }

    /// Losing the store stops traffic, and never touches liveness.
    ///
    /// This is the separation stated as an executable claim. Readiness goes
    /// unhealthy, so the instance leaves rotation. Liveness stays healthy with no
    /// contributors at all, so nothing kills the process.
    ///
    /// Restarting on a lost database would be actively harmful: the replacement comes
    /// up against the same unreachable store, fails identically, and under a
    /// restart-on-failure supervisor loops — replacing a recoverable outage with a
    /// crash loop that clears no state. The recovery case below is what actually
    /// happens instead.
    #[tokio::test]
    async fn losing_the_store_fails_readiness_without_touching_liveness() {
        let store = ProbeCountingStore::new(Err(ego_domain::operation::ReservationError::Backend(
            "connection refused".to_string(),
        )));
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(
                store as Arc<dyn ego_domain::operation::OperationReservationStore>,
            )
            .build();

        assert_eq!(
            rt.readiness().await.status,
            ego_domain::health::HealthStatus::Unhealthy
        );

        let liveness = rt.liveness();
        assert_eq!(
            liveness.status,
            ego_domain::health::HealthStatus::Healthy,
            "an unreachable dependency is not a reason to kill the process"
        );
        assert!(
            liveness.contributors.is_empty(),
            "liveness consults no contributor, including this one: {:?}",
            liveness.contributors
        );
    }

    /// Readiness recovers on its own once the store is reachable again.
    ///
    /// Each probe answers from the store as it is *now*, so there is no latched
    /// failure to clear and no restart needed to clear it. Without this, an outage
    /// that ended would leave the instance permanently out of rotation — the
    /// asserted-once cases above are all compatible with a report that never
    /// recovers.
    #[tokio::test]
    async fn readiness_recovers_when_the_store_becomes_reachable_again() {
        let store = ProbeCountingStore::new(Ok(()));
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(
                store.clone() as Arc<dyn ego_domain::operation::OperationReservationStore>
            )
            .build();

        assert_eq!(
            rt.readiness().await.status,
            ego_domain::health::HealthStatus::Healthy
        );

        store.set(Err(ego_domain::operation::ReservationError::Backend(
            "connection refused".to_string(),
        )));
        assert_eq!(
            rt.readiness().await.status,
            ego_domain::health::HealthStatus::Unhealthy
        );

        store.set(Ok(()));
        let recovered = rt.readiness().await;
        assert_eq!(
            recovered.status,
            ego_domain::health::HealthStatus::Healthy,
            "readiness must reflect the store's current reachability, not the worst it \
             has ever been"
        );
        assert_eq!(
            reservation_store_report(&recovered)
                .expect("the contributor is still registered")
                .code,
            None,
            "a recovered contributor must carry no failure code"
        );
    }

    struct StubAuthn;

    impl AuthenticationProvider for StubAuthn {
        fn authenticate(
            &self,
            _credential: &Credential,
        ) -> Result<SecurityContext, AuthenticationError> {
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
        let rt = compat().build();
        assert!(rt.security_providers().is_none());
    }

    #[test]
    fn build_with_security_succeeds() {
        let rt = compat()
            .with_security(Arc::new(StubAuthn), Arc::new(StubAuthz))
            .build();
        assert!(rt.security_providers().is_some());
    }

    #[test]
    fn runtime_inner_is_accessible() {
        let rt = compat().build();
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
        let rt = compat().build();
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
        let rt = compat().with_logger(Arc::new(KITLogger::default())).build();
        assert!(rt.logger().is_some());
    }

    #[test]
    fn shutdown_with_logger_succeeds_and_is_idempotent() {
        let rt = compat().with_logger(initialized_logger()).build();
        assert!(rt.shutdown().is_ok());
        assert!(rt.shutdown().is_ok());
    }

    #[test]
    fn shutdown_without_logger_succeeds() {
        let rt = compat().build();
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
        let rt = compat().with_logger(logger.clone()).build();

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
        let rt = compat().with_adapter(Arc::new(StubAdapter(7))).build();

        let resolved = rt.inner().resolve_adapter::<StubAdapter>();
        assert!(resolved.is_ok());
        assert_eq!(*resolved.unwrap(), StubAdapter(7));
    }

    #[test]
    fn with_config_registers_and_resolves() {
        let rt = compat()
            .with_config(Arc::new(StubConfig("hello".to_string())))
            .build();

        let resolved = rt.inner().resolve_config::<StubConfig>();
        assert!(resolved.is_ok());
        assert_eq!(*resolved.unwrap(), StubConfig("hello".to_string()));
    }

    #[test]
    fn with_adapter_last_write_wins() {
        let rt = compat()
            .with_adapter(Arc::new(StubAdapter(1)))
            .with_adapter(Arc::new(StubAdapter(2)))
            .build();

        let resolved = rt.inner().resolve_adapter::<StubAdapter>().unwrap();
        assert_eq!(*resolved, StubAdapter(2));
    }

    #[test]
    fn with_config_last_write_wins() {
        let rt = compat()
            .with_config(Arc::new(StubConfig("first".to_string())))
            .with_config(Arc::new(StubConfig("second".to_string())))
            .build();

        let resolved = rt.inner().resolve_config::<StubConfig>().unwrap();
        assert_eq!(*resolved, StubConfig("second".to_string()));
    }

    // -- CORE-028 Stage 2: RuntimeBuilder::with_projection -------------------

    #[derive(Debug, PartialEq)]
    struct StubProjection(u32);

    #[test]
    fn with_projection_registers_and_resolves() {
        let rt = compat()
            .with_projection(Arc::new(StubProjection(7)))
            .unwrap()
            .build();

        let resolved = rt.inner().resolve_projection::<StubProjection>();
        assert!(resolved.is_ok());
        assert_eq!(*resolved.unwrap(), StubProjection(7));
    }

    #[test]
    fn with_projection_rejects_duplicate_and_retains_first() {
        let builder = compat()
            .with_projection(Arc::new(StubProjection(1)))
            .unwrap();

        let err = match builder.clone().with_projection(Arc::new(StubProjection(2))) {
            Err(e) => e,
            Ok(_) => panic!("a second registration for the same projection type must fail closed"),
        };
        assert_eq!(err.type_name, std::any::type_name::<StubProjection>());

        // The runtime built from the ORIGINAL (first-registration) builder
        // must still resolve the first value — no replacement occurred.
        let rt = builder.build();
        let resolved = rt.inner().resolve_projection::<StubProjection>().unwrap();
        assert_eq!(*resolved, StubProjection(1));
    }

    #[test]
    fn resolve_projection_unregistered_returns_dependency_not_found() {
        let rt = compat().build();
        let err = rt
            .inner()
            .resolve_projection::<StubProjection>()
            .err()
            .expect("unregistered projection must fail to resolve");
        match err {
            RuntimeError::DependencyNotFound { type_name, .. } => {
                assert_eq!(type_name, std::any::type_name::<StubProjection>());
            }
            other => panic!("expected DependencyNotFound naming StubProjection, got {other:?}"),
        }
    }

    // -- CORE-028 Stage 2C: RuntimeBuilder::with_entity ----------------------

    use persistent_entity::builder::EntityRuntimeBuilder;
    use persistent_entity::test_entity::TestEntity;
    use persistent_entity::testing::{TestCommand, TestEvent, TestState};

    #[test]
    fn with_entity_registers_and_resolves() {
        let entity_runtime = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());
        let rt = compat()
            .with_entity::<TestEntity>(entity_runtime)
            .unwrap()
            .build();

        let resolved = rt.inner().resolve_entity::<TestEntity>();
        assert!(resolved.is_ok());
    }

    #[test]
    fn with_entity_rejects_duplicate_and_retains_first() {
        let first = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());
        let second = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());

        let builder = compat().with_entity::<TestEntity>(first.clone()).unwrap();

        let err = match builder.clone().with_entity::<TestEntity>(second.clone()) {
            Err(e) => e,
            Ok(_) => panic!("a second registration for the same aggregate type must fail closed"),
        };
        assert_eq!(err.type_name, std::any::type_name::<TestEntity>());

        // The runtime built from the ORIGINAL (first-registration) builder
        // must still resolve — and `second` must never have been stored
        // anywhere (AD-4, no replace_entity escape hatch).
        let rt = builder.build();
        assert!(rt.inner().resolve_entity::<TestEntity>().is_ok());
        assert_eq!(
            Arc::strong_count(&second),
            1,
            "the rejected second registration must never be stored"
        );
        assert!(
            Arc::strong_count(&first) > 1,
            "the first registration must still be held by the built runtime"
        );
    }

    #[test]
    fn resolve_entity_unregistered_returns_dependency_not_found_naming_aggregate() {
        let rt = compat().build();
        let err = rt
            .inner()
            .resolve_entity::<TestEntity>()
            .err()
            .expect("unregistered entity must fail to resolve");
        match err {
            RuntimeError::DependencyNotFound { type_name, .. } => {
                assert_eq!(type_name, std::any::type_name::<TestEntity>());
            }
            other => panic!("expected DependencyNotFound naming TestEntity, got {other:?}"),
        }
    }

    /// A second test-only aggregate whose `Event` is the SAME
    /// `persistent_entity::testing::TestEvent` `TestEntity` already uses —
    /// proves AD-1's actual claim (identity keyed on the aggregate `E`, not
    /// `E::Event`), not just that the error message names the right type.
    #[derive(Debug, Clone)]
    struct TestEntity2;

    #[async_trait::async_trait]
    impl persistent_entity::persistent_entity::PersistentEntity for TestEntity2 {
        type Command = TestCommand;
        type Event = TestEvent;
        type State = TestState;

        fn initial_state(&self) -> Self::State {
            TestState {
                value: 0,
                version: 0,
            }
        }

        async fn handle_command(
            &self,
            _command: &Self::Command,
            _state: &Self::State,
            _context: &persistent_entity::command_context::CommandContext,
        ) -> Result<Vec<Self::Event>, persistent_entity::error::EntityError> {
            Ok(vec![])
        }

        async fn apply_event(
            &self,
            state: &Self::State,
            _event: &Self::Event,
        ) -> Result<Self::State, persistent_entity::error::EntityError> {
            Ok(state.clone())
        }

        async fn apply_events(
            &self,
            state: &Self::State,
            _events: &[Self::Event],
        ) -> Result<Self::State, persistent_entity::error::EntityError> {
            Ok(state.clone())
        }
    }

    #[test]
    fn two_aggregates_sharing_an_event_type_register_and_resolve_without_collision() {
        let runtime_a = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());
        let runtime_b = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());

        let expected_a = EntityRuntimeRef::<TestEntity>::new(runtime_a.clone());
        let expected_b = EntityRuntimeRef::<TestEntity2>::new(runtime_b.clone());

        let rt = compat()
            .with_entity::<TestEntity>(runtime_a)
            .unwrap()
            .with_entity::<TestEntity2>(runtime_b)
            .unwrap()
            .build();

        let resolved_a = rt
            .inner()
            .resolve_entity::<TestEntity>()
            .expect("TestEntity must resolve its own registered runtime");
        let resolved_b = rt.inner().resolve_entity::<TestEntity2>().expect(
            "TestEntity2 must resolve its own registered runtime, unaffected by \
                 sharing an event type with TestEntity",
        );

        // `.is_ok()` alone would still pass if a keying bug stored `runtime_a`
        // under both TypeIds — both share the same erased `Arc<EntityRuntime<TestEvent>>`
        // shape, so only comparing the resolved Arc's identity against the
        // Arc each aggregate was actually registered with falsifies that bug.
        assert!(
            resolved_a.ptr_eq(&expected_a),
            "TestEntity must resolve the exact runtime it was registered with, not TestEntity2's"
        );
        assert!(
            resolved_b.ptr_eq(&expected_b),
            "TestEntity2 must resolve the exact runtime it was registered with, not TestEntity's"
        );
    }

    // -- CORE-120: chained registration --------------------------------------

    #[derive(Debug, PartialEq)]
    struct StubAdapterB(u32);

    #[derive(Debug, PartialEq)]
    struct StubConfigD(String);

    #[test]
    fn chained_registration_multiple_types() {
        let rt = compat()
            .with_adapter(Arc::new(StubAdapter(1)))
            .with_config(Arc::new(StubConfig("c".to_string())))
            .with_adapter(Arc::new(StubAdapterB(2)))
            .with_config(Arc::new(StubConfigD("d".to_string())))
            .build();

        assert_eq!(
            *rt.inner().resolve_adapter::<StubAdapter>().unwrap(),
            StubAdapter(1)
        );
        assert_eq!(
            *rt.inner().resolve_adapter::<StubAdapterB>().unwrap(),
            StubAdapterB(2)
        );
        assert_eq!(
            *rt.inner().resolve_config::<StubConfig>().unwrap(),
            StubConfig("c".to_string())
        );
        assert_eq!(
            *rt.inner().resolve_config::<StubConfigD>().unwrap(),
            StubConfigD("d".to_string())
        );
    }

    // -- PROD-005 PR2 TASK-014/015: Runtime::liveness ------------------------

    use ego_domain::health::{
        DependencyRequirement, HealthCheck, HealthContributor, HealthStatus, ProbeKind,
    };

    struct AlwaysUnhealthyRequired;

    #[async_trait]
    impl HealthContributor for AlwaysUnhealthyRequired {
        fn name(&self) -> &str {
            "always-unhealthy-required"
        }

        fn requirement(&self) -> DependencyRequirement {
            DependencyRequirement::Required
        }

        async fn check(&self) -> HealthCheck {
            HealthCheck {
                status: HealthStatus::Unhealthy,
                code: None,
            }
        }
    }

    /// A `LifecycleManaged` component whose sole contributor is Required +
    /// Unhealthy — used to prove `Runtime::liveness()` is completely
    /// unaffected by it (liveness consults no contributor/registry at all).
    struct UnhealthyLifecycleComponent;

    #[async_trait]
    impl crate::implementation::LifecycleManaged for UnhealthyLifecycleComponent {
        fn health_contributors(&self) -> Vec<Arc<dyn HealthContributor>> {
            vec![Arc::new(AlwaysUnhealthyRequired)]
        }
    }

    #[test]
    fn liveness_is_healthy_and_tagged_liveness_probe() {
        let rt = compat().build();
        let report = rt.liveness();
        assert_eq!(report.probe, ProbeKind::Liveness);
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.contributors.is_empty());
    }

    #[test]
    fn liveness_takes_no_registry_and_is_unaffected_by_a_required_unhealthy_contributor() {
        // `liveness()` is a zero-argument call on `Runtime` — it cannot be
        // handed a registry even if one exists on this runtime.
        let rt = compat()
            .with_lifecycle_component(Arc::new(UnhealthyLifecycleComponent))
            .build();

        let report = rt.liveness();

        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.contributors.is_empty());
    }

    // -- PROD-005 PR2 TASK-018/019: builder collects lifecycle contributors --

    #[tokio::test]
    async fn build_collects_health_contributors_from_registered_lifecycle_components() {
        let rt = compat()
            .with_lifecycle_component(Arc::new(UnhealthyLifecycleComponent))
            .build();

        let report = rt.readiness().await;

        assert_eq!(report.probe, ProbeKind::Readiness);
        assert_eq!(report.status, HealthStatus::Unhealthy);
        assert_eq!(report.contributors.len(), 1);
        assert_eq!(report.contributors[0].name, "always-unhealthy-required");
    }

    #[tokio::test]
    async fn a_lifecycle_component_contributing_none_leaves_aggregation_unaffected() {
        struct NoContributors;
        #[async_trait]
        impl crate::implementation::LifecycleManaged for NoContributors {
            // Default `health_contributors()` -> empty.
        }

        let rt = compat()
            .with_lifecycle_component(Arc::new(NoContributors))
            .build();

        let report = rt.readiness().await;

        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.contributors.is_empty());
    }

    #[tokio::test]
    async fn build_without_any_lifecycle_component_yields_healthy_empty_readiness() {
        let rt = compat().build();
        let report = rt.readiness().await;
        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.contributors.is_empty());
    }

    // -- PROD-005 PR3 TASK-022: ProviderHealthContributor + real HealthAggregator
    // concurrency/timeout behavior --------------------------------------------

    use crate::health::{HealthAggregationConfig, HealthAggregator, HealthRegistry};
    use ego_runtime::providers::{ProviderHealth, ProviderHealthContributor};

    /// A provider whose `health()` sleeps for a configurable duration before
    /// reporting healthy — used to prove `ProviderHealthContributor`s are
    /// fanned out concurrently through the real `HealthAggregator`, not
    /// evaluated sequentially.
    struct SlowHealthProvider {
        delay: Duration,
    }

    #[async_trait]
    impl ExternalDataProvider for SlowHealthProvider {
        async fn fetch(&self, _request: DataRequest) -> Result<DataResponse, DataProviderError> {
            unreachable!("these tests only exercise health(), never fetch()")
        }

        async fn health(&self) -> ProviderHealth {
            tokio::time::sleep(self.delay).await;
            ProviderHealth::Healthy
        }
    }

    /// A provider whose `health()` never resolves — used to prove the
    /// per-contributor timeout fires for a `ProviderHealthContributor` exactly
    /// as it would for any other `HealthContributor`.
    struct HangingHealthProvider;

    #[async_trait]
    impl ExternalDataProvider for HangingHealthProvider {
        async fn fetch(&self, _request: DataRequest) -> Result<DataResponse, DataProviderError> {
            unreachable!("these tests only exercise health(), never fetch()")
        }

        async fn health(&self) -> ProviderHealth {
            std::future::pending().await
        }
    }

    #[tokio::test(start_paused = true)]
    async fn provider_health_contributors_are_evaluated_concurrently_not_sequentially() {
        let registry = HealthRegistry::from_contributors(vec![
            Arc::new(ProviderHealthContributor::new(
                "slow-provider",
                Arc::new(SlowHealthProvider {
                    delay: Duration::from_millis(500),
                }),
            )),
            Arc::new(ProviderHealthContributor::new(
                "fast-provider-a",
                Arc::new(SlowHealthProvider {
                    delay: Duration::from_millis(1),
                }),
            )),
            Arc::new(ProviderHealthContributor::new(
                "fast-provider-b",
                Arc::new(SlowHealthProvider {
                    delay: Duration::from_millis(1),
                }),
            )),
        ]);
        let aggregator = HealthAggregator::new(registry, HealthAggregationConfig::default());

        let start = tokio::time::Instant::now();
        let report = aggregator.readiness().await;
        let elapsed = start.elapsed();

        // A slow provider must never serialize the others — the whole batch
        // completes in ~the slowest single contributor's time, not the sum.
        assert_eq!(elapsed, Duration::from_millis(500));
        assert_eq!(report.contributors.len(), 3);
        assert_eq!(report.status, HealthStatus::Healthy);
    }

    #[tokio::test(start_paused = true)]
    async fn a_provider_health_contributor_exceeding_the_per_contributor_timeout_is_unhealthy_with_timeout_code(
    ) {
        let registry = HealthRegistry::from_contributors(vec![
            Arc::new(ProviderHealthContributor::new(
                "wedged-provider",
                Arc::new(HangingHealthProvider),
            )),
            Arc::new(ProviderHealthContributor::new(
                "ok-provider",
                Arc::new(SlowHealthProvider {
                    delay: Duration::from_millis(1),
                }),
            )),
        ]);
        let config = HealthAggregationConfig {
            per_contributor: Duration::from_millis(50),
            global_budget: None,
        };
        let aggregator = HealthAggregator::new(registry, config);

        let report = aggregator.readiness().await;

        let wedged = report
            .contributors
            .iter()
            .find(|c| c.name == "wedged-provider")
            .expect("the timed-out provider must still produce a report");
        assert_eq!(wedged.status, HealthStatus::Unhealthy);
        assert_eq!(wedged.code, Some(ego_domain::health::HealthCode::Timeout));
        assert_eq!(wedged.requirement, DependencyRequirement::Required);

        // The other provider's real report survives the sibling's timeout.
        let ok = report
            .contributors
            .iter()
            .find(|c| c.name == "ok-provider")
            .expect("other providers are unaffected by one timing out");
        assert_eq!(ok.status, HealthStatus::Healthy);
        assert_eq!(ok.code, None);

        assert_eq!(report.status, HealthStatus::Unhealthy);
    }

    // -- CORE-120: unregistered type unchanged behavior ----------------------

    #[test]
    fn resolve_adapter_unregistered_returns_dependency_not_found() {
        let rt = compat().build();
        let result = rt.inner().resolve_adapter::<StubAdapter>();
        assert!(matches!(
            result,
            Err(RuntimeError::DependencyNotFound { .. })
        ));
    }

    #[test]
    fn resolve_config_unregistered_returns_dependency_not_found() {
        let rt = compat().build();
        let result = rt.inner().resolve_config::<StubConfig>();
        assert!(matches!(
            result,
            Err(RuntimeError::DependencyNotFound { .. })
        ));
    }

    // -- CORE-120: identity preservation (no clone-on-resolve) ---------------

    #[test]
    fn with_adapter_preserves_arc_identity() {
        let original = Arc::new(StubAdapter(7));
        let rt = compat().with_adapter(original.clone()).build();

        let resolved = rt.inner().resolve_adapter::<StubAdapter>().unwrap();
        assert!(
            std::ptr::eq(&*original, &*resolved),
            "resolve_adapter must return the exact registered instance, not a clone"
        );
    }

    #[test]
    fn with_config_preserves_arc_identity() {
        let original = Arc::new(StubConfig("hello".to_string()));
        let rt = compat().with_config(original.clone()).build();

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
        let rt = compat()
            .with_adapter(Arc::new(SharedType(1)))
            .with_config(Arc::new(SharedType(2)))
            .build();

        assert_eq!(
            *rt.inner().resolve_adapter::<SharedType>().unwrap(),
            SharedType(1)
        );
        assert_eq!(
            *rt.inner().resolve_config::<SharedType>().unwrap(),
            SharedType(2)
        );
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

    use crate::di::{AdapterRef, ConfigValue, DepKey, Injectable, ProjectionRef};
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
        let err = match compat().with_injectable::<NeedsAdapter>().try_build() {
            Err(e) => e,
            Ok(_) => panic!("try_build must fail fast when a recorded dependency is missing"),
        };

        match err {
            RuntimeError::DependencyNotFound {
                type_name,
                service_name,
                ..
            } => {
                assert_eq!(type_name, std::any::type_name::<StubAdapter>());
                assert_eq!(service_name, Some(std::any::type_name::<NeedsAdapter>()));
            }
            other => {
                panic!("expected DependencyNotFound naming both type and service, got {other:?}")
            }
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
            vec![DepKey::Config(
                TypeId::of::<u32>(),
                std::any::type_name::<u32>(),
            )]
        }

        fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(Self {
                limit: rt.resolve_config::<u32>()?,
            })
        }
    }

    // -- CORE-028 Stage 2 Phase 3: Injectable integration proof for
    // projections (mirrors NeedsAdapter/NeedsConfig above) --------------------

    struct NeedsProjection {
        #[allow(dead_code)]
        projection: ProjectionRef<StubProjection>,
    }

    impl Injectable for NeedsProjection {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Projection(
                TypeId::of::<StubProjection>(),
                std::any::type_name::<StubProjection>(),
            )]
        }

        fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(Self {
                projection: rt.resolve_projection::<StubProjection>()?,
            })
        }
    }

    #[test]
    fn try_build_succeeds_when_declared_projection_dependency_is_registered() {
        let rt = compat()
            .with_projection(Arc::new(StubProjection(9)))
            .unwrap()
            .with_injectable::<NeedsProjection>()
            .try_build()
            .expect("declared projection dependency is registered, try_build must succeed");

        let svc = NeedsProjection::build(rt.inner())
            .expect("build() succeeds using the same registered projection try_build validated");
        assert_eq!(*svc.projection, StubProjection(9));
    }

    #[test]
    fn try_build_fails_before_startup_when_declared_projection_dependency_is_missing() {
        let err = match compat().with_injectable::<NeedsProjection>().try_build() {
            Err(e) => e,
            Ok(_) => panic!(
                "try_build must fail fast when the declared projection dependency is missing"
            ),
        };

        match err {
            RuntimeError::DependencyNotFound {
                type_name,
                service_name,
                ..
            } => {
                assert_eq!(type_name, std::any::type_name::<StubProjection>());
                assert_eq!(service_name, Some(std::any::type_name::<NeedsProjection>()));
            }
            other => {
                panic!("expected DependencyNotFound naming both type and service, got {other:?}")
            }
        }
    }

    // -- CORE-028 Stage 2C Phase 4: Injectable integration proof for
    // entities (AD-7 item 1 — mirrors NeedsProjection above) -----------------

    use crate::di::EntityRuntimeRef;

    struct NeedsEntity {
        #[allow(dead_code)]
        entity: EntityRuntimeRef<TestEntity>,
    }

    impl Injectable for NeedsEntity {
        fn dependencies() -> Vec<DepKey> {
            vec![DepKey::Entity(
                TypeId::of::<TestEntity>(),
                std::any::type_name::<TestEntity>(),
            )]
        }

        fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> {
            Ok(Self {
                entity: rt.resolve_entity::<TestEntity>()?,
            })
        }
    }

    #[test]
    fn try_build_succeeds_when_declared_entity_dependency_is_registered() {
        let entity_runtime = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());
        let rt = compat()
            .with_entity::<TestEntity>(entity_runtime)
            .unwrap()
            .with_injectable::<NeedsEntity>()
            .try_build()
            .expect("declared entity dependency is registered, try_build must succeed");

        // Proves the built service's field holds the registered runtime
        // (mirrors `try_build_succeeds_when_declared_projection_dependency_is_registered`).
        // Does not call `entity.entity_ref(...)` here — that spawns a real
        // Tokio actor (`TokioEntityRef::new`), out of scope for a plain
        // `#[test]`; resolution success alone is what this proof needs.
        let _svc = NeedsEntity::build(rt.inner()).expect(
            "build() succeeds using the same registered entity runtime try_build validated",
        );
    }

    #[test]
    fn try_build_fails_before_startup_when_declared_entity_dependency_is_missing() {
        let err = match compat().with_injectable::<NeedsEntity>().try_build() {
            Err(e) => e,
            Ok(_) => {
                panic!("try_build must fail fast when the declared entity dependency is missing")
            }
        };

        match err {
            RuntimeError::DependencyNotFound {
                type_name,
                service_name,
                ..
            } => {
                assert_eq!(type_name, std::any::type_name::<TestEntity>());
                assert_eq!(service_name, Some(std::any::type_name::<NeedsEntity>()));
            }
            other => {
                panic!("expected DependencyNotFound naming both type and service, got {other:?}")
            }
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
    fn try_build_reports_only_the_first_registered_service_when_multiple_are_missing_dependencies()
    {
        let err = match compat()
            .with_injectable::<NeedsAdapter>()
            .with_injectable::<NeedsConfig>()
            .try_build()
        {
            Err(e) => e,
            Ok(_) => panic!("try_build must fail when multiple recorded dependencies are missing"),
        };
        match err {
            RuntimeError::DependencyNotFound { type_name, service_name, .. } => {
                assert_eq!(type_name, std::any::type_name::<StubAdapter>());
                assert_eq!(service_name, Some(std::any::type_name::<NeedsAdapter>()));
            }
            other => panic!(
                "expected DependencyNotFound naming the first-registered NeedsAdapter, got {other:?}"
            ),
        }

        let err = match compat()
            .with_injectable::<NeedsConfig>()
            .with_injectable::<NeedsAdapter>()
            .try_build()
        {
            Err(e) => e,
            Ok(_) => panic!("try_build must fail when multiple recorded dependencies are missing"),
        };
        match err {
            RuntimeError::DependencyNotFound {
                type_name,
                service_name,
                ..
            } => {
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
        let rt = compat().with_observability(obs.clone()).build();

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
        let rt = compat().build();
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
        let rt = compat().with_logger(logger.clone()).build();

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
        let rt = compat().with_logger(logger.clone()).build();

        rt.shutdown_async().await.expect("shutdown_async succeeds");
        assert!(logger.log(Severity::Info, "after-shutdown").is_err());
    }

    #[tokio::test]
    async fn shutdown_async_is_idempotent() {
        let rt = compat().build();
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
        let rt = compat().with_logger(logger.clone()).build();

        rt.register_async_teardown(async move {
            Err(RuntimeInfraError::Teardown {
                reason: "simulated read-side scheduler task failure".to_string(),
            })
        });

        let result = rt.shutdown_async().await;
        assert!(
            result.is_err(),
            "a failing hook must surface as an Err, not a silent Ok(())"
        );

        // The sync stack drained regardless of the hook's failure — no
        // resource leak just because one subsystem's teardown failed.
        assert!(logger.log(Severity::Info, "after-shutdown").is_err());
    }

    /// Only the FIRST hook's error is surfaced, but every hook still runs —
    /// a second, later hook's own teardown must not be skipped just because
    /// an earlier one failed.
    #[tokio::test]
    async fn shutdown_async_runs_every_hook_even_after_an_earlier_one_fails() {
        let rt = compat().build();
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
        let rt = compat()
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
        let rt = compat().with_injectable::<NeedsAdapter>().build();
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
        let rt = compat().build();
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
        let rt = compat().build();
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
        let err = match compat()
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

    // -- CORE-019 PR4 review F-01: build() must never panic outside Tokio ---

    /// **RED/GREEN proof for F-01 (BLOCKER).** Deliberately a plain `#[test]`,
    /// NOT `#[tokio::test]` — the absence of an active Tokio runtime is the
    /// whole point. Before the fix, `build()` called `RuntimeEffectAcceptor::
    /// start()` synchronously whenever an executor was registered, which
    /// performs a real `tokio::spawn` and panics with "there is no reactor
    /// running" outside Tokio. `build()` must remain safely callable from a
    /// plain sync bootstrap phase — it now only *constructs* the effects
    /// subsystem (`RuntimeEffectAcceptor::new`, itself already safe outside
    /// Tokio per PR3's own `new`/`start` split) and never starts it.
    #[test]
    fn build_with_registered_effect_executor_does_not_panic_outside_a_tokio_runtime() {
        let executor: Arc<dyn ExternalEffectExecutor> = Arc::new(AlwaysSucceedsExecutor::new());
        let rt = compat()
            .register_effect_executor(["invoice.created"], executor)
            .unwrap()
            .build();

        // Constructed, but deliberately NOT started — `effect_acceptor()`
        // must not expose an acceptor whose Deferred runner was never
        // spawned (see the companion test below proving it isn't
        // permanently inert once `start_effects` actually runs).
        assert!(
            rt.effect_acceptor().is_none(),
            "build() alone must never expose the acceptor — only start_effects() may"
        );
    }

    #[tokio::test]
    async fn start_effects_is_a_no_op_in_the_zero_cost_path() {
        let rt = compat().build();
        rt.start_effects()
            .await
            .expect("no executor registered — start_effects must be a harmless no-op");
        assert!(rt.effect_acceptor().is_none());
    }

    #[tokio::test]
    async fn start_effects_is_idempotent() {
        let executor: Arc<dyn ExternalEffectExecutor> = Arc::new(AlwaysSucceedsExecutor::new());
        let rt = compat()
            .register_effect_executor(["invoice.created"], executor)
            .unwrap()
            .build();

        rt.start_effects().await.expect("first call succeeds");
        rt.start_effects()
            .await
            .expect("a second call must be a safe no-op, not a double-spawn panic");
    }

    #[tokio::test]
    async fn accepted_effects_are_actually_delivered_through_the_wired_acceptor() {
        // Proves the split (construct in build(), start via start_effects())
        // does not leave the subsystem permanently inert — an accepted
        // effect really reaches the registered executor once started. This
        // is the exact gap PR3 flagged (effects silently lost when no
        // acceptor is configured), now proven end-to-end through the new
        // two-step lifecycle.
        let executor = Arc::new(AlwaysSucceedsExecutor::new());
        let rt = compat()
            .register_effect_executor(["invoice.created"], executor.clone())
            .unwrap()
            .build();

        rt.start_effects()
            .await
            .expect("an executor was registered — start_effects must succeed");

        let acceptor = rt
            .effect_acceptor()
            .expect("start_effects must make the acceptor available");
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
        let rt = compat()
            .register_effect_executor(["invoice.created"], executor.clone())
            .unwrap()
            .with_effect_drain_deadline(Duration::from_millis(200))
            .build();
        rt.start_effects().await.unwrap();

        let acceptor = rt.effect_acceptor().unwrap();
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
        let rt = compat()
            .register_effect_executor(["invoice.created"], executor)
            .unwrap()
            .with_delivery_config(DeliveryConfig::default())
            .with_effect_drain_deadline(Duration::from_millis(30))
            .build();
        rt.start_effects().await.unwrap();

        let acceptor = rt.effect_acceptor().unwrap();
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

    // -- CORE-019A Phase 4 (RED 4.1/4.2): register_data_provider, conditional
    // facade construction, single-owner teardown ---------------------------

    use ego_runtime::providers::{DuplicateProviderId, ExternalDataProvider};
    use persistent_entity::data_provider_access::{DataProviderError, DataRequest, DataResponse};

    struct RecordingShutdownProvider {
        shutdown_calls: AtomicUsize,
    }

    impl RecordingShutdownProvider {
        fn new() -> Self {
            Self {
                shutdown_calls: AtomicUsize::new(0),
            }
        }

        fn shutdown_call_count(&self) -> usize {
            self.shutdown_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ExternalDataProvider for RecordingShutdownProvider {
        async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError> {
            Ok(DataResponse {
                payload: request.payload,
                cache_hit: false,
            })
        }

        async fn shutdown(&self) {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn build_without_registering_any_data_provider_wires_no_facade() {
        // Zero-cost path (spec: "Zero Runtime Overhead When Unused";
        // design.md AD-006): no provider registered means `build()` never
        // constructs a registry or `RuntimeDataProviderAccess` at all.
        let rt = compat().build();
        assert!(
            rt.data_provider_access().is_none(),
            "no provider was registered, so no RuntimeDataProviderAccess may exist"
        );
    }

    #[tokio::test]
    async fn shutdown_async_with_no_registered_data_provider_completes_instantly() {
        let rt = compat().build();
        let started = Instant::now();
        rt.shutdown_async().await.expect("shutdown_async succeeds");
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "no data provider was registered — shutdown must not wait on anything provider-related"
        );
    }

    #[test]
    fn register_data_provider_duplicate_provider_id_fails_closed() {
        let provider: Arc<dyn ExternalDataProvider> = Arc::new(RecordingShutdownProvider::new());
        let err = match compat()
            .register_data_provider("pricing", provider.clone())
            .expect("first registration succeeds")
            .register_data_provider("pricing", provider)
        {
            Err(e) => e,
            Ok(_) => panic!("a second provider for the same provider_id must fail closed"),
        };

        assert!(matches!(
            err,
            DuplicateProviderId::AlreadyRegistered(id) if id == "pricing"
        ));
    }

    #[tokio::test]
    async fn build_with_registered_data_provider_wires_a_usable_facade() {
        let provider: Arc<dyn ExternalDataProvider> = Arc::new(RecordingShutdownProvider::new());
        let rt = compat()
            .register_data_provider("pricing", provider)
            .unwrap()
            .build();

        let access = rt
            .data_provider_access()
            .expect("a provider was registered — the facade must be constructed");
        let response = access
            .fetch("pricing", DataRequest::new("sku-1", vec![1, 2, 3]))
            .await
            .unwrap();
        assert_eq!(response.payload, vec![1, 2, 3]);
    }

    /// Spec scenario "Shutdown reaches every registered provider exactly
    /// once": with two providers registered, `shutdown_async` must invoke
    /// each one's `shutdown()` exactly once, through the single owning
    /// teardown path — never skipped, never double-invoked.
    #[tokio::test]
    async fn shutdown_async_invokes_every_registered_provider_shutdown_exactly_once() {
        let provider_a = Arc::new(RecordingShutdownProvider::new());
        let provider_b = Arc::new(RecordingShutdownProvider::new());

        let rt = compat()
            .register_data_provider(
                "pricing",
                provider_a.clone() as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .register_data_provider("jwks", provider_b.clone() as Arc<dyn ExternalDataProvider>)
            .unwrap()
            .build();

        rt.shutdown_async().await.expect("shutdown_async succeeds");

        assert_eq!(provider_a.shutdown_call_count(), 1);
        assert_eq!(provider_b.shutdown_call_count(), 1);
    }

    #[tokio::test]
    async fn shutdown_async_tears_down_an_aliased_provider_only_once() {
        // Registering the same Arc under two provider_ids is a valid
        // aliasing pattern (e.g. a migration exposing one client under both
        // an old and a new id) — it must still be torn down exactly once,
        // never once per registration (review finding on PR2).
        let provider = Arc::new(RecordingShutdownProvider::new());

        let rt = compat()
            .register_data_provider("jwks", provider.clone() as Arc<dyn ExternalDataProvider>)
            .unwrap()
            .register_data_provider(
                "jwks-legacy",
                provider.clone() as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .build();

        rt.shutdown_async().await.expect("shutdown_async succeeds");

        assert_eq!(provider.shutdown_call_count(), 1);
    }

    // -- #242: per-provider panic isolation in the teardown loop --------
    //
    // A provider `shutdown()` that panics must be isolated per-provider: the
    // panic is caught, the remaining providers still shut down, and the
    // failure is surfaced as `RuntimeInfraError::Teardown` without leaking the
    // panic payload. The exactly-once + alias dedup guarantees are unchanged.

    /// A provider whose `shutdown()` panics with a fixed message.
    struct PanickingShutdownProvider {
        message: &'static str,
    }

    impl PanickingShutdownProvider {
        fn new(message: &'static str) -> Self {
            Self { message }
        }
    }

    #[async_trait]
    impl ExternalDataProvider for PanickingShutdownProvider {
        async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError> {
            Ok(DataResponse {
                payload: request.payload,
                cache_hit: false,
            })
        }

        async fn shutdown(&self) {
            panic!("{}", self.message);
        }
    }

    #[tokio::test]
    async fn shutdown_isolates_a_panicking_provider_and_continues_others() {
        let before = Arc::new(RecordingShutdownProvider::new());
        let after = Arc::new(RecordingShutdownProvider::new());

        let rt = compat()
            .register_data_provider("before", before.clone() as Arc<dyn ExternalDataProvider>)
            .unwrap()
            .register_data_provider(
                "boom",
                Arc::new(PanickingShutdownProvider::new("kaboom")) as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .register_data_provider("after", after.clone() as Arc<dyn ExternalDataProvider>)
            .unwrap()
            .build();

        let result = rt.shutdown_async().await;

        assert!(
            matches!(result, Err(RuntimeInfraError::Teardown { .. })),
            "a panicking provider shutdown must surface as Teardown, got {result:?}"
        );
        assert_eq!(
            before.shutdown_call_count(),
            1,
            "the provider registered before the panicking one still shut down"
        );
        assert_eq!(
            after.shutdown_call_count(),
            1,
            "the provider registered after the panicking one still shut down"
        );
    }

    #[tokio::test]
    async fn panicking_shutdown_hook_does_not_escape_as_unwind() {
        let rt = compat()
            .register_data_provider(
                "boom",
                Arc::new(PanickingShutdownProvider::new("kaboom")) as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .build();

        // The panic must be caught and returned as a `Result`, never unwind
        // the caller.
        let result = rt.shutdown_async().await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn each_provider_shuts_down_at_most_once() {
        let provider = Arc::new(RecordingShutdownProvider::new());

        let rt = compat()
            .register_data_provider("pricing", provider.clone() as Arc<dyn ExternalDataProvider>)
            .unwrap()
            .build();

        rt.shutdown_async().await.expect("shutdown_async succeeds");

        assert_eq!(provider.shutdown_call_count(), 1);
    }

    #[tokio::test]
    async fn aliased_provider_is_not_shut_down_twice() {
        // The same Arc registered under two provider_ids must still be torn
        // down exactly once — the per-provider panic isolation must not break
        // the `Arc::ptr_eq` dedup at registration.
        let provider = Arc::new(RecordingShutdownProvider::new());

        let rt = compat()
            .register_data_provider("jwks", provider.clone() as Arc<dyn ExternalDataProvider>)
            .unwrap()
            .register_data_provider(
                "jwks-legacy",
                provider.clone() as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .build();

        rt.shutdown_async().await.expect("shutdown_async succeeds");

        assert_eq!(provider.shutdown_call_count(), 1);
    }

    #[tokio::test]
    async fn shutdown_panic_reason_does_not_leak_payload() {
        let rt = compat()
            .register_data_provider(
                "boom",
                Arc::new(PanickingShutdownProvider::new("SECRET_SHUTDOWN_abc123"))
                    as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .build();

        let err = rt
            .shutdown_async()
            .await
            .expect_err("a panicking shutdown must surface an error");

        let display = format!("{err}");
        let debug = format!("{err:?}");
        assert!(
            !display.contains("SECRET_SHUTDOWN_abc123"),
            "panic payload leaked into Display: {display}"
        );
        assert!(
            !debug.contains("SECRET_SHUTDOWN_abc123"),
            "panic payload leaked into Debug: {debug}"
        );
    }

    /// A provider implemented BY HAND (not via `#[async_trait]`) whose
    /// `shutdown` panics *synchronously* — during future construction, before
    /// returning the pinned future. Issue #242: the per-provider guard must
    /// cover construction too, so this panic must be caught (the loop
    /// continues) rather than escaping out of the eager argument evaluation and
    /// unwinding `shutdown_async`. Signatures mirror exactly what
    /// `#[async_trait]` desugars the methods into.
    struct SyncConstructionPanicShutdown;

    #[allow(clippy::manual_async_fn)]
    impl ExternalDataProvider for SyncConstructionPanicShutdown {
        fn fetch<'life0, 'async_trait>(
            &'life0 self,
            request: DataRequest,
        ) -> core::pin::Pin<
            Box<
                dyn core::future::Future<Output = Result<DataResponse, DataProviderError>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                Ok(DataResponse {
                    payload: request.payload,
                    cache_hit: false,
                })
            })
        }

        fn shutdown<'life0, 'async_trait>(
            &'life0 self,
        ) -> core::pin::Pin<Box<dyn core::future::Future<Output = ()> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            // Panics BEFORE returning the boxed future — outside the future's
            // own poll. With eager `AssertUnwindSafe(provider.shutdown())` this
            // escapes the guard and unwinds `shutdown_async`; with poll-time
            // deferral it is caught and the loop continues.
            panic!("SECRET_SYNC_SHUTDOWN_xyz789");
        }
    }

    #[tokio::test]
    async fn shutdown_isolates_a_synchronous_construction_panic_and_continues() {
        let after = Arc::new(RecordingShutdownProvider::new());

        let rt = compat()
            .register_data_provider(
                "boom",
                Arc::new(SyncConstructionPanicShutdown) as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .register_data_provider("after", after.clone() as Arc<dyn ExternalDataProvider>)
            .unwrap()
            .build();

        let result = rt.shutdown_async().await;

        assert!(
            matches!(result, Err(RuntimeInfraError::Teardown { .. })),
            "a synchronous-construction shutdown panic must surface as Teardown, got {result:?}"
        );
        assert_eq!(
            after.shutdown_call_count(),
            1,
            "a sync-construction panic in one provider must not skip the next"
        );
        let err = result.unwrap_err();
        let rendered = format!("{err} {err:?}");
        assert!(
            !rendered.contains("SECRET_SYNC_SHUTDOWN_xyz789"),
            "sync-construction panic payload leaked: {rendered}"
        );
    }

    // -- PROD-005 PR3 TASK-023: single registration authority — every
    // `register_data_provider` participates in the SAME runtime-owned
    // `HealthAggregator` `LifecycleManaged::health_contributors()` folds
    // into, via `ProviderHealthContributor` ---------------------------------

    struct StaticHealthDataProvider {
        health: ProviderHealth,
    }

    #[async_trait]
    impl ExternalDataProvider for StaticHealthDataProvider {
        async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError> {
            Ok(DataResponse {
                payload: request.payload,
                cache_hit: false,
            })
        }

        async fn health(&self) -> ProviderHealth {
            self.health
        }
    }

    #[tokio::test]
    async fn a_provider_registered_via_the_builder_participates_in_the_same_health_aggregator() {
        let rt = compat()
            .register_data_provider(
                "pricing",
                Arc::new(StaticHealthDataProvider {
                    health: ProviderHealth::Healthy,
                }) as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .build();

        let report = rt.readiness().await;

        assert_eq!(report.status, HealthStatus::Healthy);
        assert_eq!(report.contributors.len(), 1);
        assert_eq!(report.contributors[0].name, "pricing");
        assert_eq!(
            report.contributors[0].requirement,
            DependencyRequirement::Required
        );
        assert_eq!(report.contributors[0].code, None);
    }

    #[tokio::test]
    async fn an_unhealthy_registered_provider_drives_global_readiness_and_startup_unhealthy() {
        let rt = compat()
            .register_data_provider(
                "pricing",
                Arc::new(StaticHealthDataProvider {
                    health: ProviderHealth::Unhealthy,
                }) as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .build();

        for (probe, report) in [
            (ProbeKind::Readiness, rt.readiness().await),
            (ProbeKind::Startup, rt.startup().await),
        ] {
            assert_eq!(report.probe, probe);
            assert_eq!(report.status, HealthStatus::Unhealthy);
            assert_eq!(report.contributors.len(), 1);
            assert_eq!(report.contributors[0].name, "pricing");
            assert_eq!(
                report.contributors[0].requirement,
                DependencyRequirement::Required
            );
            assert_eq!(
                report.contributors[0].code,
                Some(ego_domain::health::HealthCode::DependencyFailure)
            );
        }
    }

    #[tokio::test]
    async fn liveness_still_consults_no_provider_even_when_one_is_registered_and_unhealthy() {
        let rt = compat()
            .register_data_provider(
                "pricing",
                Arc::new(StaticHealthDataProvider {
                    health: ProviderHealth::Unhealthy,
                }) as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .build();

        let report = rt.liveness();

        assert_eq!(report.status, HealthStatus::Healthy);
        assert!(report.contributors.is_empty());
    }

    /// CRITICAL ALIAS RULE: `data_providers_for_teardown` dedupes by
    /// `Arc::ptr_eq` (single-owner teardown), but health is a per-registered-id
    /// contract — the SAME provider `Arc` registered under two distinct
    /// `provider_id`s must still produce TWO `ProviderHealthContributor`s, one
    /// per id, never collapsed down to teardown's deduplicated single entry.
    #[tokio::test]
    async fn a_provider_aliased_under_two_ids_yields_two_distinct_health_contributors() {
        let provider = Arc::new(StaticHealthDataProvider {
            health: ProviderHealth::Healthy,
        }) as Arc<dyn ExternalDataProvider>;

        let rt = compat()
            .register_data_provider("pricing-v1", provider.clone())
            .unwrap()
            .register_data_provider("pricing-v2", provider)
            .unwrap()
            .build();

        let report = rt.readiness().await;

        assert_eq!(
            report.contributors.len(),
            2,
            "both aliased provider_ids must each contribute their own health report"
        );
        let mut names: Vec<&str> = report
            .contributors
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["pricing-v1", "pricing-v2"]);
    }

    #[tokio::test]
    async fn register_data_provider_health_composes_with_lifecycle_component_health() {
        let rt = compat()
            .with_lifecycle_component(Arc::new(UnhealthyLifecycleComponent))
            .register_data_provider(
                "pricing",
                Arc::new(StaticHealthDataProvider {
                    health: ProviderHealth::Healthy,
                }) as Arc<dyn ExternalDataProvider>,
            )
            .unwrap()
            .build();

        let report = rt.readiness().await;

        assert_eq!(report.contributors.len(), 2);
        assert_eq!(
            report.status,
            HealthStatus::Unhealthy,
            "the unhealthy lifecycle contributor still drives the SAME aggregator unhealthy"
        );
    }

    // -- PROD-003 Phase 4 (TASK-013/014): RuntimeBuilder::with_tracer /
    // with_tracer_lifecycle / with_traced wiring ----------------------------
    //
    // Ownership split (design.md ADR-9): `tracer` (used to build the
    // `TracingInterceptor`) and `tracer_lifecycle` (owned ONLY for a single
    // `shutdown()` on teardown) are two separate optional fields — `shutdown`
    // is exporter lifecycle, not a domain tracing call, so `NoopTracer`, test
    // spies, and any lifecycle-less `Tracer` never need to know it.

    use ego_domain::{SpanAttributes, SpanId, SpanOutcome, TraceContext, Tracer, TracerLifecycle};

    use crate::context::ServiceContext;
    use crate::interceptor::InterceptorChain;

    /// Spy `Tracer`: records every `start_span` call so tests can assert the
    /// `TracingInterceptor` actually fired through the runtime's interceptor
    /// chain.
    struct SpyTracer {
        start_calls: AtomicUsize,
    }

    impl SpyTracer {
        fn new() -> Self {
            Self {
                start_calls: AtomicUsize::new(0),
            }
        }

        fn start_call_count(&self) -> usize {
            self.start_calls.load(Ordering::SeqCst)
        }
    }

    impl Tracer for SpyTracer {
        fn start_span(&self, _ctx: &TraceContext, _name: &str, _attrs: SpanAttributes) {
            self.start_calls.fetch_add(1, Ordering::SeqCst);
        }

        fn end_span(&self, _span: SpanId, _outcome: SpanOutcome) {}
    }

    /// Spy `TracerLifecycle`: an `AtomicUsize` shutdown counter proving
    /// `Runtime::shutdown_async` invokes `shutdown()` EXACTLY once — even if
    /// `shutdown_async` itself is called more than once (single-shutdown
    /// guarantee).
    struct SpyTracerLifecycle {
        shutdown_calls: AtomicUsize,
    }

    impl SpyTracerLifecycle {
        fn new() -> Self {
            Self {
                shutdown_calls: AtomicUsize::new(0),
            }
        }

        fn shutdown_call_count(&self) -> usize {
            self.shutdown_calls.load(Ordering::SeqCst)
        }
    }

    impl TracerLifecycle for SpyTracerLifecycle {
        fn shutdown(&self) {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// A double implementing BOTH `Tracer` and `TracerLifecycle` from the
    /// same concrete type — stands in for the OTLP adapter (PR5) that
    /// `with_traced` is the wiring seam for.
    struct TracedDouble {
        start_calls: AtomicUsize,
        shutdown_calls: AtomicUsize,
    }

    impl TracedDouble {
        fn new() -> Self {
            Self {
                start_calls: AtomicUsize::new(0),
                shutdown_calls: AtomicUsize::new(0),
            }
        }
    }

    impl Tracer for TracedDouble {
        fn start_span(&self, _ctx: &TraceContext, _name: &str, _attrs: SpanAttributes) {
            self.start_calls.fetch_add(1, Ordering::SeqCst);
        }

        fn end_span(&self, _span: SpanId, _outcome: SpanOutcome) {}
    }

    impl TracerLifecycle for TracedDouble {
        fn shutdown(&self) {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn ctx_with_root_trace() -> ServiceContext {
        ServiceContext::new().with_trace_context(TraceContext::root())
    }

    #[tokio::test]
    async fn with_tracer_wires_a_tracing_interceptor_into_the_chain() {
        let tracer = Arc::new(SpyTracer::new());
        let rt = compat()
            .with_tracer(tracer.clone() as Arc<dyn Tracer>)
            .build();

        let ctx = ctx_with_root_trace();
        rt.inner()
            .interceptor_chain
            .on_request(&ctx)
            .await
            .expect("on_request succeeds");

        assert_eq!(
            tracer.start_call_count(),
            1,
            "with_tracer must wire a TracingInterceptor that drives the supplied Tracer"
        );
    }

    #[tokio::test]
    async fn without_with_tracer_no_interceptor_is_wired_and_behavior_is_unchanged() {
        // Omitted ⇒ byte-identical to today: no TracingInterceptor is added
        // at all (not even a NoopTracer-backed one running for nothing).
        let rt = compat().build();

        let ctx = ctx_with_root_trace();
        rt.inner()
            .interceptor_chain
            .on_request(&ctx)
            .await
            .expect("on_request succeeds with an empty chain");

        // No tracer was ever wired, so there is nothing to assert calls
        // against — the interceptor chain itself must report zero wired
        // interceptors (default `compat()` behavior,
        // pre-PROD-003, unchanged).
        assert_eq!(
            format!("{:?}", rt.inner().interceptor_chain),
            format!("{:?}", InterceptorChain::new()),
            "with no tracer registered, the interceptor chain must remain exactly as empty as \
             compat()'s default — no TracingInterceptor wired"
        );
    }

    #[tokio::test]
    async fn shutdown_async_invokes_tracer_lifecycle_shutdown_exactly_once() {
        let lifecycle = Arc::new(SpyTracerLifecycle::new());
        let rt = compat()
            .with_tracer_lifecycle(lifecycle.clone() as Arc<dyn TracerLifecycle>)
            .build();

        rt.shutdown_async().await.expect("shutdown_async succeeds");

        assert_eq!(lifecycle.shutdown_call_count(), 1);
    }

    #[tokio::test]
    async fn shutdown_async_called_twice_still_shuts_down_tracer_lifecycle_exactly_once() {
        let lifecycle = Arc::new(SpyTracerLifecycle::new());
        let rt = compat()
            .with_tracer_lifecycle(lifecycle.clone() as Arc<dyn TracerLifecycle>)
            .build();

        rt.shutdown_async()
            .await
            .expect("first shutdown_async succeeds");
        rt.shutdown_async()
            .await
            .expect("second shutdown_async succeeds");

        assert_eq!(
            lifecycle.shutdown_call_count(),
            1,
            "shutdown must never run twice, even when shutdown_async is triggered more than once"
        );
    }

    #[tokio::test]
    async fn shutdown_async_with_no_tracer_lifecycle_registers_no_hook() {
        let rt = compat().build();
        let started = Instant::now();
        rt.shutdown_async().await.expect("shutdown_async succeeds");
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "no tracer_lifecycle was registered — shutdown must not wait on anything tracer-related"
        );
    }

    #[tokio::test]
    async fn with_traced_sets_both_tracer_and_tracer_lifecycle_from_the_same_instance() {
        let traced = Arc::new(TracedDouble::new());
        let rt = compat().with_traced(traced.clone()).build();

        let ctx = ctx_with_root_trace();
        rt.inner()
            .interceptor_chain
            .on_request(&ctx)
            .await
            .expect("on_request succeeds");
        rt.shutdown_async().await.expect("shutdown_async succeeds");

        assert_eq!(
            traced.start_calls.load(Ordering::SeqCst),
            1,
            "with_traced must wire the same instance as the span-producing Tracer"
        );
        assert_eq!(
            traced.shutdown_calls.load(Ordering::SeqCst),
            1,
            "with_traced must wire the same instance as the TracerLifecycle shut down on teardown"
        );
    }
}

/// PROD-012 AD-3i — the reservation configuration `build()` assembles.
///
/// These go through `RuntimeBuilder::build()` rather than calling
/// `ReservationConfig::new` directly, on purpose. A test that constructed the
/// config itself would keep passing if `build()` stopped threading the setters
/// through — which is the failure that actually matters here, because the
/// setters are the only way a deployment configures any of this.
#[cfg(test)]
mod reservation_config_tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use ego_domain::operation::{
        OwnerFence, OwnerId, ReservationError, ReservationOutcome, ReserveRequest,
        StoredServiceResponse,
    };
    use ego_domain::time::Clock;
    use std::time::Duration;

    /// A clock that never moves, so lease arithmetic is checked rather than raced.
    struct FrozenClock(DateTime<Utc>);

    impl Clock for FrozenClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    /// Present only so a reservation can be configured; every method panics
    /// because none of these tests dispatch an operation.
    struct InertStore;

    #[async_trait::async_trait]
    impl OperationReservationStore for InertStore {
        async fn reserve(
            &self,
            _req: ReserveRequest,
        ) -> Result<ReservationOutcome, ReservationError> {
            panic!("these tests configure a reservation; they never take one");
        }
        async fn renew(
            &self,
            _fence: &OwnerFence,
            _until: DateTime<Utc>,
        ) -> Result<(), ReservationError> {
            panic!("these tests configure a reservation; they never renew one");
        }
        async fn complete(
            &self,
            _fence: &OwnerFence,
            _response: StoredServiceResponse,
        ) -> Result<(), ReservationError> {
            panic!("these tests configure a reservation; they never complete one");
        }
        async fn abandon(&self, _fence: &OwnerFence) -> Result<(), ReservationError> {
            panic!("these tests configure a reservation; they never abandon one");
        }
        async fn purge_completed_before(
            &self,
            _cutoff: DateTime<Utc>,
            _batch: usize,
        ) -> Result<u64, ReservationError> {
            panic!("these tests configure a reservation; they never purge");
        }
        async fn probe(&self) -> Result<(), ReservationError> {
            Ok(())
        }
    }

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0)
            .single()
            .expect("a valid instant")
    }

    fn runtime_with(lease: Duration, clock_at: i64, owner: Option<OwnerId>) -> Runtime {
        let mut b = RuntimeBuilder::new()
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .with_operation_reservation_store(Arc::new(InertStore))
            .with_reservation_clock(Arc::new(FrozenClock(at(clock_at))))
            .with_reservation_lease_duration(lease);
        if let Some(owner) = owner {
            b = b.with_reservation_owner_id(owner);
        }
        b.build()
    }

    /// One identity per runtime, stable for its whole life — which is what makes
    /// a retry inside this runtime observable as `OwnedInProgress`.
    #[test]
    fn one_runtime_reports_the_same_owner_every_time() {
        let rt = runtime_with(Duration::from_secs(30), 1_000, None);
        let first = rt
            .inner
            .reservation()
            .expect("configured")
            .owner_id()
            .clone();
        let second = rt
            .inner
            .reservation()
            .expect("configured")
            .owner_id()
            .clone();
        assert_eq!(
            first, second,
            "an owner that changed between reads would make self-contention \
             indistinguishable from external contention"
        );
    }

    /// Two runtimes are two owners. Sharing one would erase the difference
    /// between self-contention and external contention, and would break lease
    /// renewal, which must only renew a lease this instance holds.
    #[test]
    fn two_runtimes_get_different_owners() {
        let a = runtime_with(Duration::from_secs(30), 1_000, None);
        let b = runtime_with(Duration::from_secs(30), 1_000, None);
        assert_ne!(
            a.inner.reservation().expect("configured").owner_id(),
            b.inner.reservation().expect("configured").owner_id(),
            "each runtime instance must mint its own reservation identity"
        );
    }

    /// The injected owner survives `build()` intact — the property a test that
    /// needs to decide who owns what depends on.
    #[test]
    fn an_injected_owner_is_kept_exactly() {
        let owner = OwnerId::new("owner-under-test");
        let rt = runtime_with(Duration::from_secs(30), 1_000, Some(owner.clone()));
        assert_eq!(
            rt.inner.reservation().expect("configured").owner_id(),
            &owner
        );
    }

    /// `lease_until` is `now + lease` read from the configured clock, so expiry
    /// is checked by arithmetic rather than by waiting.
    #[test]
    fn the_lease_expiry_comes_from_the_injected_clock() {
        let rt = runtime_with(Duration::from_secs(45), 1_000, None);
        assert_eq!(
            rt.inner.reservation().expect("configured").lease_until(),
            at(1_045),
            "a lease computed from the system clock instead would make expiry \
             testable only by sleeping"
        );
    }

    /// Zero is refused at construction, so no partially-valid configuration can
    /// exist for a caller to hold.
    #[test]
    fn a_zero_lease_is_refused_rather_than_stored() {
        let refused = crate::runtime::idempotency::ReservationConfig::new(
            Arc::new(InertStore),
            Arc::new(FrozenClock(at(0))),
            OwnerId::new("owner"),
            Duration::ZERO,
        );
        assert_eq!(
            refused.err(),
            Some(crate::runtime::idempotency::ReservationConfigError::ZeroLease),
            "a zero lease expires the instant it is taken: every attempt would \
             see the previous one as expired and take it over"
        );
    }
}
