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
use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime::error::RuntimeInfraError;
use crate::runtime::idempotency::{
    IdempotencyEnforcementMode, ReservationConfig, ReservationDecision, ReservationRejection,
};
use ego_domain::context::TenantId;
use ego_domain::event::DomainEvent;
use ego_domain::operation::OperationFingerprint;
use ego_domain::operation::OperationKeyHash;
use ego_domain::operation::ReservationError;
use ego_domain::{
    MetricAttribute, Observability, SemanticEvent, SpanAttributes, SpanOutcome, Tracer,
};
use ego_runtime::effects::RuntimeEffectAcceptor;
use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::{
    authorize_in_context, Action, AuthorizationProvider, Resource,
};
use ego_security_sdk::error::SecurityError;
use kitlogger::KITLogger;
use persistent_entity::data_provider_access::DataProviderAccess;

use persistent_entity::persistent_entity::PersistentEntity;
use persistent_entity::runtime::EntityRuntime;

use super::logger::TeardownStack;
use super::permit::CrossTenantPermit;
#[cfg(test)]
use super::tenant::TenantEnforcementMode;
use super::tenant::{EstablishedTenantFacts, TenantResolver};
use crate::context::ServiceContext;
use crate::di::{AdapterRef, ConfigValue, DepKey, EntityRuntimeRef, ProjectionRef};
use crate::interceptor::InterceptorChain;
use crate::registry::ServiceRegistry;

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
    /// Entity runtimes registered via `RuntimeBuilder::with_entity`
    /// (CORE-028 Stage 2C), keyed by the aggregate type `E`, never
    /// `E::Event` (design.md AD-1).
    entities: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

/// The four host-registered dependency maps `RuntimeBuilder::build` hands to
/// [`DependencyTable::with_registrations`], bundled as named fields (code
/// review fix, CORE-028 Stage 2C): the previous four-positional-parameter
/// signature had all four maps at the identical type
/// `HashMap<TypeId, Arc<dyn Any + Send + Sync>>`, so a transposed call site
/// (e.g. `adapters` and `configs` swapped) compiled cleanly and would only
/// surface as a runtime `DependencyNotFound`. Named fields make that a
/// compile error instead — a swapped field name at a construction site is
/// caught by the type checker, not discovered by a test.
pub(super) struct RegisteredDependencies {
    pub(super) adapters: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    pub(super) configs: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    pub(super) projections: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    pub(super) entities: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl DependencyTable {
    #[cfg(test)]
    fn new() -> Self {
        Self {
            projections: HashMap::new(),
            adapters: HashMap::new(),
            configs: HashMap::new(),
            entities: HashMap::new(),
        }
    }

    /// Builds a table from host-registered adapters/configs/projections/entities
    /// (`RuntimeBuilder`). Takes a single [`RegisteredDependencies`] value
    /// with named fields so the four identically-typed maps can't be
    /// silently transposed at the call site — a mismatched field name is a
    /// compile error, unlike four positional parameters of the same type
    /// (CORE-028 Stage 2: was `with_registrations(adapters, configs)`,
    /// always hardcoding an empty `projections` map — now also threaded
    /// through from `RuntimeBuilder::with_entity`, CORE-028 Stage 2C).
    pub(super) fn with_registrations(registered: RegisteredDependencies) -> Self {
        let RegisteredDependencies {
            adapters,
            configs,
            projections,
            entities,
        } = registered;
        Self {
            projections,
            adapters,
            configs,
            entities,
        }
    }

    fn resolve_projection<T: 'static + Send + Sync>(
        &self,
    ) -> Result<ProjectionRef<T>, RuntimeError> {
        self.projections
            .get(&TypeId::of::<T>())
            .and_then(|arc| arc.clone().downcast::<T>().ok())
            .map(ProjectionRef::new)
            .ok_or_else(|| dependency_not_found::<T>(DependencyKind::Projection))
    }

    /// Resolves a registered entity runtime as `EntityRuntimeRef<E>`, keyed
    /// by the aggregate type `E` (design.md AD-1) — never `E::Event`.
    /// Mirrors `resolve_projection`'s downcast-and-wrap shape exactly.
    fn resolve_entity<E>(&self) -> Result<EntityRuntimeRef<E>, RuntimeError>
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
        self.entities
            .get(&TypeId::of::<E>())
            .and_then(|arc| arc.clone().downcast::<EntityRuntime<E::Event>>().ok())
            .map(EntityRuntimeRef::new)
            .ok_or_else(|| dependency_not_found::<E>(DependencyKind::Entity))
    }

    fn resolve_adapter<A: 'static + Send + Sync>(&self) -> Result<AdapterRef<A>, RuntimeError> {
        self.adapters
            .get(&TypeId::of::<A>())
            .and_then(|arc| arc.clone().downcast::<A>().ok())
            .map(AdapterRef::new)
            .ok_or_else(|| dependency_not_found::<A>(DependencyKind::Adapter))
    }

    fn resolve_config<C: 'static + Send + Sync>(&self) -> Result<ConfigValue<C>, RuntimeError> {
        self.configs
            .get(&TypeId::of::<C>())
            .and_then(|arc| arc.clone().downcast::<C>().ok())
            .map(ConfigValue::new)
            .ok_or_else(|| dependency_not_found::<C>(DependencyKind::Config))
    }
}

/// Builds a `DependencyNotFound` naming `T` and its `kind`, with no requesting
/// service attached yet (the `try_build()` validator path fills in
/// `service_name` on the way out).
fn dependency_not_found<T: 'static>(kind: DependencyKind) -> RuntimeError {
    RuntimeError::DependencyNotFound {
        kind,
        type_name: std::any::type_name::<T>(),
        service_name: None,
    }
}

// ---------------------------------------------------------------------------
/// What an operator should do about a lost completion.
///
/// Carried on every `idempotency.completion_lost` event because the three
/// causes are not equally actionable, and an operator should not have to infer
/// that from the reason tag.
///
/// Deliberately an **action** rather than a claim about recurrence. Whether the
/// same failure would happen again is not something this code can establish —
/// a `Serialize` implementation may fail on one value and succeed on the next,
/// and a stale fence says nothing about what the reservation looks like now.
/// What can be stated is what is worth doing, so that is what travels.
#[derive(Clone, Copy)]
enum OperatorAction {
    /// Nothing per occurrence; the rate is the signal.
    MonitorRate,
    /// Compare the configured lease against how long the work actually takes.
    ReviewLeaseDuration,
    /// Do not assume retry or waiting will clear it; investigate.
    Investigate,
}

impl OperatorAction {
    fn as_str(self) -> &'static str {
        match self {
            OperatorAction::MonitorRate => "monitor_rate",
            OperatorAction::ReviewLeaseDuration => "review_lease_duration",
            OperatorAction::Investigate => "investigate",
        }
    }
}

// Security denial observability (CORE-012A)
// ---------------------------------------------------------------------------

/// The three macro-guard denial outcomes reachable and instrumented by this
/// change (design.md AD-1). Fieldless by design (AD-3) — the guard remains
/// solely responsible for deciding *what* happened and constructing the
/// `SecurityError` it independently returns via `?`; `RuntimeInner` receives
/// only the tag it needs to emit an observability event, never a copy of
/// sensitive detail. `CrossTenantDenied` is deliberately absent — no
/// macro-reachable call path can produce it today (spec requirement 5).
#[derive(Debug, Clone, Copy)]
pub enum SecurityDenialKind {
    /// No `SecurityContext` was attached (`#[authorize]`'s `ctx.security()`
    /// check, or `enforce_tenant`'s unresolvable-context arm).
    MissingContext,
    /// `enforce_tenant` resolved a hard mismatch between the authenticated
    /// tenant and a caller-supplied hint, with no covering grant.
    TenantMismatch,
    /// The configured `AuthorizationProvider` denied the request.
    AuthorizationDenied,
}

impl SecurityDenialKind {
    /// Maps a `SecurityError` produced at a macro guard call site to the
    /// denial kind it represents, or `None` if `err` isn't one of the three
    /// reachable, in-scope denial outcomes (design.md AD-1) — e.g.
    /// `ProviderError`/`CapabilityNotEnabled` (infra failures) or any other
    /// `SecurityError` variant this change doesn't instrument. Centralizes
    /// the mapping as ordinary, unit-testable Rust so both macro call sites
    /// (`service-sdk-macros/src/lib.rs`) share one classification instead of
    /// duplicating `match` arms inside `quote!{}` token trees.
    pub fn from_security_error(err: &SecurityError) -> Option<Self> {
        match err {
            SecurityError::MissingContext => Some(Self::MissingContext),
            SecurityError::TenantMismatch { .. } => Some(Self::TenantMismatch),
            SecurityError::AuthorizationDenied { .. } => Some(Self::AuthorizationDenied),
            _ => None,
        }
    }
}

/// Redacted, `Display`-safe label (design.md AD-3) — there is no field to
/// leak because `SecurityDenialKind` itself carries none; full diagnostic
/// detail (`expected`/`actual`/`reason`) stays exclusively on the
/// `SecurityError` value the guard independently returns, whose own `Debug`
/// impl (AD-010) already retains it. Byte-identical to the derived `Debug`
/// output, spelled out explicitly so this contract doesn't silently change if
/// a future variant gets fields.
impl std::fmt::Display for SecurityDenialKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            SecurityDenialKind::MissingContext => "MissingContext",
            SecurityDenialKind::TenantMismatch => "TenantMismatch",
            SecurityDenialKind::AuthorizationDenied => "AuthorizationDenied",
        };
        write!(f, "{label}")
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
/// The `RuntimeBuilder` constructs this struct with registered instances;
/// `registry` and `interceptor_chain` are read by `Runtime::resolve` and
/// written by `RuntimeBuilder::with_service` (CORE-025).
/// A single registered async teardown hook (Finding 6/F-02) — a pinned,
/// boxed, fallible future. Named to keep `RuntimeInner`'s field declaration
/// under clippy's `type_complexity` threshold.
type AsyncTeardownHook = Pin<Box<dyn Future<Output = Result<(), RuntimeInfraError>> + Send>>;

pub struct RuntimeInner {
    pub(crate) registry: ServiceRegistry,
    pub(crate) interceptor_chain: Arc<InterceptorChain>,
    /// Optional security providers (authn + authz) installed via RuntimeBuilder.
    pub(crate) security_providers: Option<(
        Arc<dyn AuthenticationProvider>,
        Arc<dyn AuthorizationProvider>,
    )>,
    /// Resolved instances for projection, adapter, and config injection.
    resolved: DependencyTable,
    /// Resolves the canonical tenant for enforcement (CORE-008A AD-001/AD-009).
    /// Built from the [`TenantEnforcementMode`] configured via
    /// `RuntimeBuilder::with_tenant_enforcement_mode` (AD-012).
    tenant_resolver: TenantResolver,
    /// The idempotency policy this runtime was built under, retained exactly as
    /// configured.
    ///
    /// The builder already validates it — it refuses to build under
    /// [`IdempotencyEnforcementMode::MandatoryKey`] with no reservation store,
    /// because a runtime promising every mutating operation carries a key and
    /// having nowhere to reserve one cannot keep the promise. What it did not do
    /// was *keep* the value, so nothing downstream could apply the policy the
    /// build was validated against.
    ///
    /// A transport must read it from here. The alternative — configuring the
    /// same policy again at the HTTP layer — creates two places that can
    /// disagree about whether keys are mandatory, and the one that decides
    /// whether a request is rejected would not be the one the builder checked.
    idempotency_enforcement_mode: IdempotencyEnforcementMode,
    /// The logger constructed by the host and registered via `RuntimeBuilder::with_logger`.
    logger: Option<Arc<KITLogger>>,
    /// The reservation store registered via
    /// `RuntimeBuilder::with_operation_reservation_store`, retained so idempotent
    /// dispatch can reach the same instance the host supplied.
    ///
    /// `None` under `IdempotencyEnforcementMode::Compatibility`, which is the mode a
    /// deployment declares when it has not adopted enforcement. It cannot be `None`
    /// under the enforcing variant: the builder refuses to produce a runtime in that
    /// state.
    ///
    /// Read only through [`RuntimeInner::operation_reservation_store`], which is
    /// what keeps this field from being reported unused — the accessor carries the
    /// awaiting-its-consumer marker, not the field.
    reservation: Option<ReservationConfig>,
    /// Observability sink for macro-guard security denials (CORE-012A AD-2).
    /// `None` by default — behaviorally identical to `NoopObservability`
    /// discarding events, keeping `ego-service-sdk` free of an
    /// `ego-infrastructure` dependency edge. Set via
    /// `RuntimeBuilder::with_observability(..)`.
    observability: Option<Arc<dyn Observability>>,
    /// Infrastructure teardown stack, drained in reverse construction order on shutdown.
    ///
    /// `RuntimeInner` is always shared via `Arc` (generated proxies hold
    /// `Weak<RuntimeInner>`), so `Runtime::shutdown(&self)` needs interior
    /// mutability to drain it.
    pub(super) teardown: Mutex<TeardownStack>,
    /// Additive (Finding 6): async teardown hooks registered post-build via
    /// `Runtime::register_async_teardown`, run in registration order by
    /// `Runtime::shutdown_async` before the sync `teardown` stack above
    /// drains. Always empty for callers who never register a hook — the
    /// existing sync `shutdown()` path is completely unaffected. Fallible
    /// (post-review Finding F-02): a hook's failure to drain must be
    /// distinguishable from a clean drain, not silently treated as success.
    pub(super) async_teardown: Mutex<Vec<AsyncTeardownHook>>,
    /// **PR4 review F-01 fix:** the external-effects acceptor constructed by
    /// `RuntimeBuilder::build()` (CORE-019 Phase 9), if at least one executor
    /// was registered. `None` is the zero-cost path (design.md §8/§20): no
    /// store, no queue, no acceptor exists at all when this is `None`.
    ///
    /// Kept as the concrete `RuntimeEffectAcceptor` (not the `EffectAcceptor`
    /// trait object `Runtime::effect_acceptor()` exposes) so
    /// `Runtime::start_effects` can call `.start()` on it — construction
    /// (safe outside Tokio) and starting (spawns a real Tokio task, panics
    /// without an active runtime) are deliberately re-separated at this
    /// layer, mirroring `RuntimeEffectAcceptor`'s own already-shipped
    /// `new`/`start` split (PR3).
    pub(crate) effect_acceptor_impl: Option<Arc<RuntimeEffectAcceptor>>,
    /// Set to `true` exactly once, by `Runtime::start_effects` — guards
    /// against calling `RuntimeEffectAcceptor::start` (which spawns a Tokio
    /// task and must run at most once) more than once, and against
    /// `Runtime::effect_acceptor()` exposing an acceptor whose `Deferred`
    /// runner was never spawned. A caller can therefore never obtain (and so
    /// never silently use) an acceptor that would just accept effects into a
    /// queue nobody drains (PR4 review F-01).
    pub(crate) effect_started: AtomicBool,
    /// Guards `start_retention` so a second call starts no second worker.
    pub(crate) retention_started: AtomicBool,
    /// How long `Runtime::start_effects`'s registered async teardown hook
    /// waits for the `Deferred` drain loop before forcing remaining
    /// in-flight effects back to `Pending` (design.md §8). Meaningless when
    /// `effect_acceptor_impl` is `None`.
    pub(crate) effect_drain_deadline: Duration,
    /// The external-data-provider facade constructed by
    /// `RuntimeBuilder::build()` (CORE-019A Phase 4), if at least one
    /// provider was registered via `RuntimeBuilder::register_data_provider`.
    /// `None` is the zero-cost path (design.md AD-006): no registry, no
    /// `RuntimeDataProviderAccess` is ever constructed when this is `None`.
    /// Unlike `effect_acceptor_impl`, there is no separate `start` step —
    /// `RuntimeDataProviderAccess` never spawns a task, so it is fully usable
    /// the moment `build()` returns.
    pub(crate) data_provider_access: Option<Arc<dyn DataProviderAccess>>,
    /// The `Tracer` registered via `RuntimeBuilder::with_tracer`, **retained**
    /// rather than only consumed at `build()` time.
    ///
    /// It used to be consumed solely to construct a `TracingInterceptor` and then
    /// dropped, which meant nothing outside the interceptor chain could open a
    /// span — including this type's own idempotency instrumentation (AD-10),
    /// whose spans belong to the reservation path rather than to a request
    /// boundary. Keeping it here is what makes those emittable at all.
    ///
    /// `None` means no tracer was registered, and every span site is then a
    /// no-op. That mirrors `TracingInterceptor`'s own behaviour when a request
    /// carries no `TraceContext`: no silent `NoopTracer` standing in, and nothing
    /// running for nothing.
    tracer: Option<Arc<dyn Tracer>>,
}

/// A started span that is guaranteed to be closed, even if the work it wraps never
/// finishes.
///
/// # Why a guard and not two calls
///
/// The obvious shape — `start_span`, `.await`, `end_span` — leaks a span whenever
/// the future is dropped between them. That is not exotic: an `.await` is a
/// cancellation point, so a client disconnect, a timeout wrapper, a `select!` that
/// another branch wins, or a panic unwinding through the frame all drop the future
/// mid-flight and skip the `end_span` that follows.
///
/// The consequence is worse than one missing span. The OTLP adapter's table is
/// bounded by `max_in_flight_spans`, and **at capacity it drops new spans rather
/// than evicting live ones**. So leaked entries accumulate, and once they fill the
/// table the adapter silently stops recording anything — tracing degrades to
/// nothing under exactly the conditions (load, timeouts, cancellation) where it is
/// most needed, and reports no error while doing so.
///
/// `Drop` closes what a normal path did not, so the leak is not reachable rather
/// than merely unlikely.
///
/// # The cancelled outcome is an error, and a fixed one
///
/// A reserve attempt that was abandoned did not complete, so `Ok` would be a lie
/// about work whose result nobody ever learned. The message is a constant:
/// `SpanOutcome::Error` requires a redaction-safe one, and there is nothing about
/// the cancellation worth forwarding.
///
/// # Double-closing is harmless anyway
///
/// `Tracer::end_span` is contractually idempotent per `SpanId`, so a guard that
/// closed twice would be absorbed by the adapter. The flag is kept regardless, so
/// the *recorded* outcome is the classified one rather than whichever call landed
/// second — which is what makes the distinction assertable in a test.
pub(super) struct OpenSpan {
    tracer: Arc<dyn Tracer>,
    span_id: ego_domain::SpanId,
    closed: bool,
}

impl OpenSpan {
    pub(super) fn new(tracer: Arc<dyn Tracer>, span_id: ego_domain::SpanId) -> Self {
        Self {
            tracer,
            span_id,
            closed: false,
        }
    }

    /// Closes the span with the outcome the completed work earned.
    pub(super) fn close(mut self, outcome: SpanOutcome) {
        self.tracer.end_span(self.span_id, outcome);
        self.closed = true;
    }
}

impl Drop for OpenSpan {
    fn drop(&mut self) {
        if !self.closed {
            self.tracer.end_span(
                self.span_id,
                SpanOutcome::Error {
                    status_message: "the reservation attempt was abandoned before it completed"
                        .to_string(),
                },
            );
        }
    }
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
    // Sole runtime wiring constructor; a params struct is a larger refactor out of scope.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_with_logger(
        registry: ServiceRegistry,
        interceptor_chain: Arc<InterceptorChain>,
        security_providers: Option<(
            Arc<dyn AuthenticationProvider>,
            Arc<dyn AuthorizationProvider>,
        )>,
        resolved: DependencyTable,
        logger: Option<Arc<KITLogger>>,
        teardown: Mutex<TeardownStack>,
        tenant_resolver: TenantResolver,
        idempotency_enforcement_mode: IdempotencyEnforcementMode,
        reservation: Option<ReservationConfig>,
        observability: Option<Arc<dyn Observability>>,
        effect_acceptor_impl: Option<Arc<RuntimeEffectAcceptor>>,
        effect_drain_deadline: Duration,
        data_provider_access: Option<Arc<dyn DataProviderAccess>>,
        tracer: Option<Arc<dyn Tracer>>,
    ) -> Self {
        Self {
            registry,
            interceptor_chain,
            security_providers,
            resolved,
            logger,
            teardown,
            async_teardown: Mutex::new(Vec::new()),
            tenant_resolver,
            idempotency_enforcement_mode,
            reservation,
            observability,
            effect_acceptor_impl,
            effect_started: AtomicBool::new(false),
            retention_started: AtomicBool::new(false),
            effect_drain_deadline,
            data_provider_access,
            tracer,
        }
    }

    /// The idempotency policy this runtime was built and validated under.
    ///
    /// A transport reads this to decide what to do with an operation key it did
    /// or did not receive — but it must read it *only to pass it on*. The policy
    /// table itself has exactly one owner,
    /// [`resolve_operation_key`](crate::idempotency::resolve_operation_key);
    /// matching on this value to re-decide whether a missing key is admissible
    /// would create the second definition that module exists to prevent.
    pub fn idempotency_enforcement_mode(&self) -> IdempotencyEnforcementMode {
        self.idempotency_enforcement_mode
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

    /// Resolves a registered entity runtime as `EntityRuntimeRef<E>`, keyed
    /// by the aggregate type `E` (CORE-028 Stage 2C design.md AD-1/AD-8).
    ///
    /// Returns `DependencyNotFound` naming `E` (never `E::Event`) if no
    /// entity runtime was registered for `E`.
    pub fn resolve_entity<E>(&self) -> Result<EntityRuntimeRef<E>, RuntimeError>
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
        self.resolved.resolve_entity::<E>()
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

    /// Checks whether a single dependency's backing instance is present in
    /// this runtime's resolved tables — a pure presence check that
    /// constructs nothing (AD-3 / OQ-2). Used by `Injectable::validate()`'s
    /// generic default.
    ///
    /// `DepKey::Entity` is a real presence check against the `entities`
    /// table (CORE-028 Stage 2C design.md AD-1/AD-7) — mirroring the
    /// `Projection` arm exactly. Before this change, `Entity` unconditionally
    /// returned `Err` regardless of table state (no entity table existed);
    /// that fail-safe stub is retired now that entity registration exists.
    pub(crate) fn check_dependency(&self, dep: &DepKey) -> Result<(), RuntimeError> {
        let (present, type_name, kind) = match dep {
            DepKey::Entity(id, name) => (
                self.resolved.entities.contains_key(id),
                *name,
                DependencyKind::Entity,
            ),
            DepKey::Projection(id, name) => (
                self.resolved.projections.contains_key(id),
                *name,
                DependencyKind::Projection,
            ),
            DepKey::Adapter(id, name) => (
                self.resolved.adapters.contains_key(id),
                *name,
                DependencyKind::Adapter,
            ),
            DepKey::Config(id, name) => (
                self.resolved.configs.contains_key(id),
                *name,
                DependencyKind::Config,
            ),
        };
        if present {
            Ok(())
        } else {
            Err(RuntimeError::DependencyNotFound {
                kind,
                type_name,
                service_name: None,
            })
        }
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

    /// Records exactly one denial event for a macro-guard outcome (CORE-012A
    /// design.md AD-1, spec "Reachable Macro-Guard Denials Are Recorded" +
    /// "Minimum Recorded Event Contract"). A silent no-op when no
    /// `Observability` implementor is configured (AD-2 default `None`).
    ///
    /// Calls the configured implementor's `trace()` synchronously, with no
    /// blocking isolation — relies entirely on the `Observability` trait's
    /// own "Non-blocking" contract (`ego_domain::Observability`). A panicking
    /// implementor is isolated via `catch_unwind` below; a *blocking* one is
    /// not this method's concern to defend against (code-review 4R
    /// resilience finding, accepted: fixing that here would require making
    /// this method async and restructuring macro-generated control flow —
    /// out of proportion to a contract violation by the implementor).
    ///
    /// # Accessibility contract (macro-visibility)
    ///
    /// This method is `pub` solely to satisfy Rust's visibility rules for
    /// code generated by the `ego-service-sdk-macros` proc-macro crate —
    /// same contract as [`Self::authorization_provider`] above. Application
    /// code MUST NOT call this directly; `#[doc(hidden)]` hides it from
    /// rustdoc.
    #[doc(hidden)]
    pub fn record_security_denial(
        &self,
        service: &'static str,
        operation: &'static str,
        kind: SecurityDenialKind,
    ) {
        let Some(obs) = &self.observability else {
            return;
        };
        // ponytail: HashMap<String,String> allocation for 3 fixed keys is forced by
        // SemanticEvent::metadata's pre-existing type (crates/domain); a lower-allocation
        // shape would mean changing that shared domain-wide type, out of this change's
        // scope. Accepted technical debt — revisit only if it shows up in a real profile.
        let mut metadata = HashMap::new();
        metadata.insert("denial_kind".to_string(), kind.to_string());
        metadata.insert("service".to_string(), service.to_string());
        metadata.insert("operation".to_string(), operation.to_string());
        let event = SemanticEvent::new("security.denial", "", "", "Denied", "", metadata)
            .expect("event_name is a fixed non-empty literal");
        // A caller-supplied `Observability` implementor is untrusted, same as
        // `AuthorizationProvider` (security-sdk/authorization/mod.rs) — a panicking
        // sink must not turn a clean security-denial `Err` return into an unwind.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| obs.trace(event)));
    }

    /// Resolves and enforces the canonical tenant for this call (CORE-008A
    /// AD-009). On success, writes the resolved [`super::tenant::CanonicalTenant`]
    /// into `ctx` via `ctx.set_resolved_tenant` and returns `Ok(())`. On
    /// failure, returns `Err` without mutating `ctx`.
    ///
    /// Wired into `#[tenant_scoped]`-generated operations via a fallible `?`
    /// call (`service-sdk-macros`); unmarked operations never call this at
    /// all (AD-007).
    ///
    /// Gathers the closed set of Established Facts (AD-014) from `ctx` —
    /// the authenticated `SecurityContext`, the caller-supplied hint, and any
    /// already-established cross-tenant grant — and hands them to
    /// `TenantResolver::resolve` as a single value. This function only
    /// orchestrates the gathering; it never itself decides the tenant
    /// outcome.
    pub fn enforce_tenant(&self, ctx: &mut ServiceContext) -> Result<(), SecurityError> {
        let facts = EstablishedTenantFacts::new(
            ctx.security(),
            ctx.tenant_hint(),
            ctx.cross_tenant_grant(),
        );
        let canonical = self.tenant_resolver.resolve(facts)?;
        ctx.set_resolved_tenant(canonical);
        Ok(())
    }

    /// The registered `Tracer`, if any.
    ///
    /// `pub(super)`, which is the least that works: the only callers are span sites
    /// inside `crate::runtime` — this module's own reservation span and the retention
    /// worker's. Handing it out beyond that would let an adopter open spans that look
    /// like the runtime's own.
    pub(super) fn tracer(&self) -> Option<Arc<dyn Tracer>> {
        self.tracer.clone()
    }

    /// The registered `Observability`, if any.
    ///
    /// `pub(super)` for the same reason as [`RuntimeInner::tracer`]: the only callers
    /// are metric sites inside `crate::runtime`.
    pub(super) fn observability(&self) -> Option<Arc<dyn Observability>> {
        self.observability.clone()
    }

    /// The reservation capability, if this deployment configured one.
    ///
    /// `pub(crate)` deliberately. Idempotent dispatch is the only caller, and it
    /// lives inside this crate; a public accessor would invite reaching around
    /// the `#[idempotent]` seam to reserve operations by hand, which is the one
    /// path the enforcement mode cannot police.
    ///
    /// `None` under
    /// [`IdempotencyEnforcementMode::Compatibility`](crate::runtime::IdempotencyEnforcementMode::Compatibility),
    /// which is the mode a deployment declares when it has not adopted
    /// enforcement. It cannot be `None` under the enforcing variant: the builder
    /// refuses to produce a runtime in that state, so a caller that finds `None`
    /// here knows enforcement is off rather than that a registration was missed.
    ///
    /// The store is reached only through here, never handed out. AD-3g keeps the
    /// reservation and its outcome branching inside this crate so there is one
    /// implementation to test rather than one copy per generated operation.
    pub(crate) fn reservation(&self) -> Option<&ReservationConfig> {
        self.reservation.as_ref()
    }

    /// Reserves one `#[idempotent]` operation before it is dispatched.
    ///
    /// The single public entry point generated slot-3 code calls (AD-3g). The
    /// store access and the six-way outcome interpretation stay behind it, so
    /// changing how an outcome is translated is not a breaking change for every
    /// generated caller. `tenant`, `key` and `fingerprint` arrive already
    /// definitive — canonicalisation and fingerprinting belong to the generated
    /// code under AD-3f, and nothing is re-derived here.
    ///
    /// # Why it takes the context by `&mut`
    ///
    /// Mirroring [`RuntimeInner::enforce_tenant`], and for the same reason. Two
    /// of the three values a reservation needs are *already definitive on the
    /// context* — the `OperationKey` carried from ingress and the canonical
    /// tenant `enforce_tenant` resolved — so reading them here rather than
    /// re-passing them keeps one reading of each, instead of one copy expanded
    /// into every generated operation. Only `fingerprint` is passed, because
    /// only it is the generated code's to compute (AD-3f).
    ///
    /// The `&mut` also lets this method **stamp the fingerprint onto the
    /// context**, which is what makes the whole chain work: a service body
    /// downstream threads that exact value into each aggregate's
    /// `CommandContext`, so the per-aggregate receipt gate compares against the
    /// same request identity the reservation used. Stamping here rather than
    /// exposing a public setter means a fingerprint on a context is evidence
    /// that this method ran, not that somebody assigned a field.
    ///
    /// The namespace is the **canonical** tenant, never
    /// [`ServiceContext::tenant_hint`] — a caller-supplied hint must not choose
    /// which namespace its key lands in.
    ///
    /// # An unresolved scope is refused, never defaulted
    ///
    /// A context that resolved *no* scope is not a context that resolved to the
    /// systemwide one. The first has no namespace; the second's namespace is the
    /// absent partition. Treating them alike files every unresolved dispatch in
    /// the shared tenant-less partition, where two tenants presenting one
    /// operation key become one operation — the second replaying the first's
    /// stored response, or being refused as a conflict against it. So an
    /// unresolved scope returns
    /// [`ReservationRejection::TenantUnresolved`](crate::runtime::ReservationRejection::TenantUnresolved)
    /// before the store is reached.
    ///
    /// The contract is **"a scope was resolved before reserving"**, not "a
    /// particular attribute is present". Nothing here inspects markers: an
    /// operation reaches this method with a resolved scope or it does not, and how
    /// it got one is not this method's business.
    ///
    /// # The three answers, and why `Option` rather than a third decision
    ///
    /// `None` means **this runtime did not reserve** — either no
    /// [`ReservationConfig`] was registered, which the builder permits only
    /// under
    /// [`IdempotencyEnforcementMode::Compatibility`](crate::runtime::IdempotencyEnforcementMode::Compatibility),
    /// or the context carries no key. That is not the same statement as
    /// [`ReservationDecision::Proceed`](crate::runtime::ReservationDecision::Proceed),
    /// even though both continue: `Proceed` carries a permit, and therefore a
    /// fence that a later completion must present. A dispatch that never
    /// reserved has no fence to present and must not have one invented for it.
    /// Folding the two into a single "continue" would make a permit-less
    /// completion representable, which is exactly the shape
    /// [`ReservationDecision`](crate::runtime::ReservationDecision) is split to
    /// prevent. Nothing is stamped in that case either, so the receipt gate
    /// downstream stays inactive rather than gating on a request identity that
    /// reserved nothing.
    ///
    /// # What this method deliberately does not decide
    ///
    /// Whether a *keyless* request may proceed. It returns `Ok(None)` and
    /// dispatch continues. That is the missing-key policy, and it has one owner
    /// — [`resolve_operation_key`](crate::idempotency::resolve_operation_key) at
    /// the transport edge — so two adapters cannot disagree about it. Deciding
    /// it a second time here would create the second definition that module
    /// exists to prevent.
    pub async fn reserve_idempotent_operation(
        &self,
        ctx: &mut ServiceContext,
        fingerprint: OperationFingerprint,
    ) -> Result<Option<ReservationDecision>, ReservationRejection> {
        let (Some(config), Some(key)) = (self.reservation(), ctx.operation_key().cloned()) else {
            return Ok(None);
        };

        // An explicit `match` over three states, not `and_then` over two.
        //
        // `canonical_tenant()` is `None` when nothing on this path resolved a
        // scope, and `Some(_)` when something did — where that resolution may
        // legitimately be the tenant-less systemwide scope, whose namespace is the
        // absent one. Those are three answers, and only two of them name a
        // namespace.
        //
        // `and_then` flattened the first into the third. It compiled, it read
        // naturally, and it filed every unresolved dispatch in the shared
        // tenant-less partition — so two tenants presenting one operation key
        // became one operation there. Measured before this changed, through
        // generated dispatch with two authenticated principals and one key: with
        // an identical payload the handler ran once and the second tenant was
        // handed the first's stored response; with a differing payload the second
        // tenant was refused `FingerprintConflict` against a reservation belonging
        // to the other scope. The first of those is an information disclosure, not
        // a correctness slip.
        //
        // `crates/service-sdk/tests/cross_tenant_reservation_isolation.rs` holds
        // it. Restoring the `and_then` puts two of its tests red on the refusal
        // they assert, and a third in-crate test red on the reserve count.
        //
        // Refused before the store is reached, deliberately: reserving first and
        // refusing afterwards would leave the lease taken under a namespace this
        // dispatch was never entitled to.
        let tenant = match ctx.canonical_tenant() {
            None => return Err(ReservationRejection::TenantUnresolved),
            Some(resolved) => resolved.tenant_id().cloned(),
        };

        // AD-10's `idempotency.reserve` span, opened around the durable write and
        // nothing else.
        //
        // Opened here rather than at the top of the method, deliberately: the span
        // reports a reserve *attempt*, and the two early exits above — no store or
        // no key, and an unresolved scope — never attempt one. A span covering
        // those would report a durable write that did not happen, with a duration
        // that means nothing.
        //
        // A child `TraceContext`, so this span has its own `SpanId`. The adapter's
        // table is keyed on that id and ignores a duplicate start for a live one,
        // so reusing the request's id would silently drop either this span or the
        // interceptor's. This is the first production caller of
        // `TraceContext::child()`, which existed as a seam for exactly this.
        //
        // No tracer registered, or no `TraceContext` on the request, means no span
        // — the same rule `TracingInterceptor` follows for a request that
        // originated no trace. Minting a root here instead would attach orphans to
        // no trace, on the hot path of every keyed operation.
        //
        // Held as a guard rather than as a plain id, because the `.await` below is a
        // cancellation point: see [`OpenSpan`].
        let span =
            self.tracer
                .as_ref()
                .zip(ctx.trace_context().copied())
                .map(|(tracer, parent)| {
                    let child = parent.child();
                    tracer.start_span(
                        &child,
                        "idempotency.reserve",
                        // The key is about to be moved into `reserve`, so the token is
                        // derived first. `OperationKeyHash::of` is the only way to build
                        // one, so the raw key cannot be what lands here.
                        SpanAttributes::new().with_operation_key_hash(OperationKeyHash::of(&key)),
                    );
                    OpenSpan::new(tracer.clone(), child.span_id())
                });

        let outcome = config
            .reserve(
                tenant,
                key,
                fingerprint.clone(),
                // The registered instance, or `None`. A runtime with no observability
                // emits no counters and dispatches identically — the same posture the
                // tracer takes.
                self.observability.as_ref(),
            )
            .await;

        if let Some(span) = span {
            // A refusal on the merits is not a failed span. `Conflict`,
            // `OtherInProgress` and the rest are answers the store gave, and the
            // attempt they answer completed exactly as designed — recording them as
            // errors would make a correctly-refused duplicate look like an outage
            // on every dashboard that counts span errors. Only the case where the
            // store could not answer at all is a failure of this attempt.
            //
            // The message is a fixed string. `SpanOutcome::Error`'s own contract
            // requires a redaction-safe one, and the rejection carries nothing
            // worth forwarding anyway.
            let ended = match &outcome {
                Err(ReservationRejection::StoreUnavailable) => SpanOutcome::Error {
                    status_message: "the reservation store could not answer".to_string(),
                },
                _ => SpanOutcome::Ok,
            };
            span.close(ended);
        }

        let decision = outcome?;

        // Stamped only after the store accepted it. A fingerprint left on a
        // context whose reservation was refused would be carried into an
        // aggregate's `CommandContext` by a body that never ran — and would sit
        // in a receipt describing work no reservation ever authorised.
        ctx.set_operation_fingerprint(fingerprint);
        Ok(Some(decision))
    }

    /// Records the response of an operation that just completed, so the next
    /// identical arrival replays it instead of running anything.
    ///
    /// The writer half of the pair
    /// [`reserve_idempotent_operation`](RuntimeInner::reserve_idempotent_operation)
    /// opens. Called by the `#[idempotent]` epilogue after the handler returned
    /// `Ok`, under the fence the permit carries — so a lease taken over in the
    /// meantime cannot have its result overwritten by the owner it replaced.
    ///
    /// # Why this returns nothing
    ///
    /// **A failed completion must not fail an operation that succeeded.** By the
    /// time this runs the handler has returned `Ok` and every aggregate has
    /// committed its events and its receipt. The work happened. Reporting an
    /// error to the caller now would describe successful work as a failure, and
    /// invite a retry of something that must not run twice.
    ///
    /// # Exactly what is lost, and for how long
    ///
    /// Not "the retry just re-runs the body". The reservation stays open, so
    /// there is a window. In order:
    ///
    /// 1. **The durable work stays successful.** It was confirmed before this
    ///    ran and nothing here revisits it.
    /// 2. **The immediate replay is lost.** The reservation never reached
    ///    `Succeeded`, so it has no stored response to answer with.
    /// 3. **While the lease is still valid, retries are refused as in progress**
    ///    — `SelfInProgress` or `OtherInProgress` per AD-3h. They do *not* reach
    ///    the body. This is a real unavailability window, and it is as long as
    ///    the configured lease.
    /// 4. **Once the lease expires, a retry takes ownership** (`TakenOver`) and
    ///    does reach the body.
    /// 5. **The per-aggregate receipts then stop each durable step from
    ///    happening twice** — every step already confirmed replays instead of
    ///    re-running.
    ///
    /// So the guarantee holds, but the cost is not only "a slower retry": it is
    /// a lease-long window in which an operation that in fact succeeded answers
    /// its caller with contention.
    ///
    /// **Scope.** Step 5 protects what the receipt protocol covers — durable
    /// aggregate writes and effects made idempotent through it. It says nothing
    /// about an arbitrary external side effect a handler performs outside that
    /// protocol; nothing here makes such an effect safe to repeat.
    ///
    /// That window and that scope limit are why the failure is reported rather
    /// than swallowed: it is invisible to the caller by design, so it has to be
    /// visible to an operator.
    ///
    /// # What each answer means, and how urgent it is
    ///
    /// - `None`, or `Replay` — nothing to record, and nothing is reported. A
    ///   dispatch that never reserved has no fence to present, and a replay
    ///   produced no new response; it returned one that was already stored.
    /// - `StaleOwner` (`stale_owner`, `review_lease_duration`) — this response
    ///   was **discarded** because the caller no longer held a current fence.
    ///   That is all the contract guarantees: the fence triple no longer matched
    ///   or the lease had lapsed, and this call did not modify the reservation.
    ///
    ///   It does **not** say another owner completed the operation. Any of these
    ///   produces it: another owner took the lease over and is still running;
    ///   another owner already completed it; or the lease simply expired with no
    ///   takeover at all. **What the reservation looks like now is not knowable
    ///   from this error** — reading it back is the only way to find out. Worth
    ///   acting on either way, because every path here means the lease elapsed
    ///   before the work did.
    ///
    ///   The window described above also does not apply as written to this case:
    ///   the reservation is not necessarily still open under this owner. It may
    ///   already be completed, or held by someone else.
    /// - Store failure (`store_unavailable`, `monitor_rate`) — the ordinary
    ///   contingency. The shortcut is lost for this operation; the next one is
    ///   unaffected. One occurrence is noise, a rate is a problem.
    /// - Encoding failure (`not_encodable`, `investigate`) — this *value* failed
    ///   to serialise. Note what that does and does not say: `T: Serialize` is
    ///   satisfied at compile time, so this is not "the type cannot be
    ///   serialised" — a hand-written `Serialize` may fail on one value and
    ///   succeed on the next, or depend on state outside the value entirely.
    ///
    ///   So this is not a proof that the failure recurs. It is a judgement that
    ///   waiting is not a justified recovery strategy: the failure requires
    ///   investigation, and treating it as an infrastructure blip that will pass
    ///   is the one response the evidence does not support. Until someone looks,
    ///   this operation does not reach `Succeeded` and does not replay.
    pub async fn complete_idempotent_operation<T: serde::Serialize>(
        &self,
        reservation: Option<&ReservationDecision>,
        output: &T,
    ) {
        let Some(ReservationDecision::Proceed(permit)) = reservation else {
            return;
        };
        let Some(config) = self.reservation() else {
            return;
        };

        let stored = match crate::runtime::encode_stored_response(output) {
            Ok(stored) => stored,
            Err(rejection) => {
                self.record_completion_lost(
                    "not_encodable",
                    OperatorAction::Investigate,
                    &rejection.to_string(),
                );
                return;
            }
        };

        match config.store().complete(permit.fence(), stored).await {
            Ok(()) => {}
            Err(ReservationError::StaleOwner) => {
                // AD-10's `idempotency.lease.stale_owner`, whose attribute names the
                // operation that hit it. `complete` is the only value, and **AD-10d
                // withdrew the other two** rather than leaving them owed: nothing
                // invokes `renew` or `abandon`, so neither can return `StaleOwner`
                // and neither has anything to count.
                //
                // Neither is a wire left unconnected. `renew` needs a renewal policy;
                // `abandon` needs a safe-abandonment policy, and that one is delicate
                // — if a commit may have landed and only the response was lost,
                // releasing the key early admits the re-execution this whole design
                // exists to prevent. AD-10d carries the reasoning.
                //
                // A counter and not just the existing operator event: this is the
                // signal a rate alert fires on. `record_completion_lost` below emits a
                // semantic event carrying the operator action, which is what somebody
                // reads *after* being paged — the two are not substitutes.
                if let Some(obs) = &self.observability {
                    obs.counter(
                        "idempotency.lease.stale_owner",
                        1.0,
                        &[MetricAttribute::new("operation", "complete")],
                    );
                }
                self.record_completion_lost(
                    "stale_owner",
                    OperatorAction::ReviewLeaseDuration,
                    "this response was discarded because the caller no longer held a \
                     current fence — the lease had lapsed or been taken over. What the \
                     reservation looks like now is not knowable from this error",
                )
            }
            Err(e) => self.record_completion_lost(
                "store_unavailable",
                OperatorAction::MonitorRate,
                &e.to_string(),
            ),
        }
    }

    /// Reports a completion that was lost, through the same sink and with the
    /// same panic isolation as [`RuntimeInner::record_security_denial`].
    ///
    /// This exists because the failure is deliberately invisible to the caller —
    /// see [`complete_idempotent_operation`](RuntimeInner::complete_idempotent_operation)
    /// for why an operation that succeeded must not be reported as failed. A
    /// consequence nobody is told about is a consequence nobody can act on, so
    /// the one place it surfaces is here.
    ///
    /// `reason` is a fixed tag rather than prose so an operator can count these
    /// without matching on a message; `action` says what to do about it; `detail`
    /// carries the specifics.
    fn record_completion_lost(&self, reason: &'static str, action: OperatorAction, detail: &str) {
        let Some(obs) = &self.observability else {
            return;
        };
        let mut metadata = HashMap::new();
        metadata.insert("reason".to_string(), reason.to_string());
        metadata.insert("action".to_string(), action.as_str().to_string());
        metadata.insert("detail".to_string(), detail.to_string());
        let event = SemanticEvent::new(
            "idempotency.completion_lost",
            "",
            "",
            "Completed",
            "",
            metadata,
        )
        .expect("event_name is a fixed non-empty literal");
        // Same reasoning as `record_security_denial`: a caller-supplied sink is
        // untrusted, and a panicking one must not unwind through an operation
        // that already succeeded.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| obs.trace(event)));
    }

    /// Mints a cross-tenant permit authorizing access to `destination`
    /// (CORE-008A AD-008, FR-005/FR-006).
    ///
    /// Resolves the `Principal` from `ctx.security()`, then runs an
    /// `AuthorizationProvider` capability check for the explicit
    /// `"tenant:cross-tenant-access"` request (resource/action authorization
    /// alone is never sufficient — FR-005). A `Deny` maps to
    /// `SecurityError::CrossTenantDenied`; other provider failures propagate
    /// unchanged. On `Allow`, mints a permit scoped to `destination`.
    ///
    /// # Errors
    /// - [`SecurityError::CapabilityNotEnabled`] if no security context is
    ///   attached, or if no `AuthorizationProvider` is configured on this
    ///   runtime.
    /// - [`SecurityError::CrossTenantDenied`] if the provider denies the
    ///   cross-tenant capability check.
    /// - Any other [`SecurityError`] the provider itself returns (e.g.
    ///   `ProviderError` on backend failure/panic).
    // SAFETY: must remain pub(crate) — widening to pub would let external crates
    // mint CrossTenantPermit without authorization.
    // Used only in tests until a real production caller adopts cross-tenant
    // issuance (this framework-stage codebase has no application services yet).
    #[allow(dead_code)]
    pub(crate) async fn issue_cross_tenant_permit(
        &self,
        ctx: &ServiceContext,
        destination: TenantId,
    ) -> Result<CrossTenantPermit, SecurityError> {
        let provider = self
            .authorization_provider()
            .ok_or(SecurityError::CapabilityNotEnabled)?;
        let resource = Resource {
            kind: Cow::Borrowed("tenant"),
            id: Some(destination.as_str().to_string()),
        };
        let action = Action(Cow::Borrowed("cross-tenant-access"));

        match authorize_in_context(ctx.security(), resource, action, provider.as_ref()).await {
            Ok(()) => {
                let issued_to = ctx
                    .security()
                    .ok_or(SecurityError::CapabilityNotEnabled)?
                    .principal()
                    .subject_id
                    .clone();
                Ok(CrossTenantPermit::new(destination, issued_to))
            }
            Err(SecurityError::AuthorizationDenied { reason }) => {
                Err(SecurityError::CrossTenantDenied { reason })
            }
            Err(other) => Err(other),
        }
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
            DependencyTable::with_registrations(RegisteredDependencies {
                adapters: HashMap::new(),
                configs: HashMap::new(),
                projections: HashMap::new(),
                entities: HashMap::new(),
            }),
            None,
            Mutex::new(TeardownStack::new()),
            TenantResolver::new(mode),
            IdempotencyEnforcementMode::Compatibility,
            None,
            None,
            None,
            Duration::from_secs(5),
            None,
            None,
        )
    }

    /// Test fixture variant with an `Observability` implementor configured
    /// (CORE-012A TASK-003) — `for_test()`'s sibling helper for tests that
    /// need to assert on recorded denial events, mirroring
    /// `for_test_with_authz`'s pattern.
    #[cfg(test)]
    pub(crate) fn for_test_with_observability(obs: Arc<dyn Observability>) -> Self {
        Self::new_with_logger(
            ServiceRegistry::new(),
            Arc::new(InterceptorChain::new()),
            None,
            DependencyTable::with_registrations(RegisteredDependencies {
                adapters: HashMap::new(),
                configs: HashMap::new(),
                projections: HashMap::new(),
                entities: HashMap::new(),
            }),
            None,
            Mutex::new(TeardownStack::new()),
            TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly),
            IdempotencyEnforcementMode::Compatibility,
            None,
            Some(obs),
            None,
            Duration::from_secs(5),
            None,
            None,
        )
    }

    /// Test fixture variant with an `AuthorizationProvider` configured
    /// (CORE-008A TASK-018 — `for_test()`'s sibling helper for the
    /// `issue_cross_tenant_permit` call-site migration; production code
    /// configures both providers via `RuntimeBuilder::with_security`
    /// instead). The authentication side is a stub never invoked by
    /// `issue_cross_tenant_permit`, which only reads `authorization_provider()`.
    #[cfg(test)]
    pub(crate) fn for_test_with_authz(provider: Arc<dyn AuthorizationProvider>) -> Self {
        Self::new_with_logger(
            ServiceRegistry::new(),
            Arc::new(InterceptorChain::new()),
            Some((
                Arc::new(NoopTestAuthn) as Arc<dyn AuthenticationProvider>,
                provider,
            )),
            DependencyTable::with_registrations(RegisteredDependencies {
                adapters: HashMap::new(),
                configs: HashMap::new(),
                projections: HashMap::new(),
                entities: HashMap::new(),
            }),
            None,
            Mutex::new(TeardownStack::new()),
            TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly),
            IdempotencyEnforcementMode::Compatibility,
            None,
            None,
            None,
            Duration::from_secs(5),
            None,
            None,
        )
    }
}

/// Never-invoked authentication stub for [`RuntimeInner::for_test_with_authz`] —
/// `issue_cross_tenant_permit` never calls `authenticate()`, only
/// `authorization_provider()`.
#[cfg(test)]
struct NoopTestAuthn;

#[cfg(test)]
impl AuthenticationProvider for NoopTestAuthn {
    fn authenticate(
        &self,
        _: &ego_security_sdk::credential::Credential,
    ) -> Result<ego_security_sdk::context::SecurityContext, ego_security_sdk::AuthenticationError>
    {
        unimplemented!("NoopTestAuthn is never invoked by issue_cross_tenant_permit")
    }
}

// ---------------------------------------------------------------------------
// Runtime errors
// ---------------------------------------------------------------------------

/// The kind of a missing dependency (design: DX follow-up) — carried by
/// [`RuntimeError::DependencyNotFound`] so the error can name both what is
/// missing *and* the exact builder method that registers it. Threaded from
/// [`RuntimeInner::check_dependency`]'s `DepKey` match and from each typed
/// `resolve_*` path, so no diagnostic identity is discarded on the way out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyKind {
    /// A host-registered adapter — registered with `.adapter(...)`.
    Adapter,
    /// A host-registered config value — registered with `.config(...)`.
    Config,
    /// A registered projection — registered with `.projection(...)`.
    Projection,
    /// A registered entity runtime — registered with `.entity::<E>()`.
    Entity,
}

impl DependencyKind {
    /// The `AppBuilder`/`RuntimeBuilder` method that registers this kind,
    /// ready to splice into a fix hint. `Entity` is parameterized on the
    /// missing aggregate type so the hint reads `.entity::<MyAgg>()`.
    fn fix_hint(self, type_name: &str) -> String {
        match self {
            DependencyKind::Adapter => ".adapter(...)".to_string(),
            DependencyKind::Config => ".config(...)".to_string(),
            DependencyKind::Projection => ".projection(...)".to_string(),
            DependencyKind::Entity => format!(".entity::<{type_name}>()"),
        }
    }
}

impl std::fmt::Display for DependencyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            DependencyKind::Adapter => "adapter",
            DependencyKind::Config => "config",
            DependencyKind::Projection => "projection",
            DependencyKind::Entity => "entity",
        };
        write!(f, "{label}")
    }
}

/// Errors that can occur during proxy resolution or dependency injection.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RuntimeError {
    /// The requested service was not found in the registry.
    #[error(
        "service `{type_name}` not found{} — register it with .service::<{type_name}>() or .service_instance::<{type_name}>(instance) before resolving",
        required_by.map(|s| format!(" (required by `{s}`)")).unwrap_or_default()
    )]
    ServiceNotFound {
        /// The name of the missing service tag.
        type_name: &'static str,
        /// The name of the requesting service, when known.
        required_by: Option<&'static str>,
    },
    /// A dependency was not found during resolution.
    #[error(
        "{kind} dependency `{type_name}` not found{} — register it with {} on AppBuilder",
        service_name.map(|s| format!(" (required by `{s}`)")).unwrap_or_default(),
        kind.fix_hint(type_name)
    )]
    DependencyNotFound {
        /// The kind of the missing dependency (adapter/config/projection/entity).
        kind: DependencyKind,
        /// The name of the missing dependency's type.
        type_name: &'static str,
        /// The name of the requesting service, when known.
        service_name: Option<&'static str>,
    },
    /// Idempotency enforcement is on and no reservation store was registered.
    ///
    /// A variant of its own rather than a [`Self::DependencyNotFound`] with some
    /// existing [`DependencyKind`]: none of those kinds describes this, and each
    /// carries a fix hint naming the registration method for *its* kind. Reusing one
    /// would tell the reader to call `.adapter(...)`, which does not register a
    /// reservation store — a misdirecting error is worse than a terse one.
    #[error(
        "idempotency enforcement is on (IdempotencyEnforcementMode::MandatoryKey) but no \
         OperationReservationStore is registered — a runtime that requires a client-supplied \
         operation key has nowhere to reserve it. Register one with \
         .with_operation_reservation_store(store), or state that this deployment has not \
         adopted enforcement with \
         .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)"
    )]
    OperationReservationStoreNotRegistered,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use ego_domain::operation::{
        FencingToken, Lease, OperationId, OperationKey, OperationReservationStore, OwnerFence,
        OwnerId, ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse,
    };
    use ego_domain::time::Clock;

    use crate::runtime::{Runtime, RuntimeBuilder};

    // -- DependencyTable unit tests -----------------------------------------

    #[test]
    fn dependency_table_new_is_empty() {
        let t = DependencyTable::new();
        assert!(t.projections.is_empty());
        assert!(t.adapters.is_empty());
        assert!(t.configs.is_empty());
        assert!(t.entities.is_empty());
    }

    // -- Missing registration (TypeId not found) ----------------------------

    #[test]
    fn runtime_inner_default_creates_empty() {
        let rt = RuntimeInner::for_test();
        assert!(matches!(
            rt.resolve_projection::<()>(),
            Err(RuntimeError::DependencyNotFound { .. })
        ));
    }

    #[test]
    fn resolve_projection_returns_not_found_for_unregistered() {
        let rt = RuntimeInner::for_test();
        let result: Result<ProjectionRef<()>, RuntimeError> = rt.resolve_projection();
        assert!(matches!(
            result,
            Err(RuntimeError::DependencyNotFound { .. })
        ));
    }

    #[test]
    fn resolve_adapter_returns_not_found_for_unregistered() {
        let rt = RuntimeInner::for_test();
        let result: Result<AdapterRef<()>, RuntimeError> = rt.resolve_adapter();
        assert!(matches!(
            result,
            Err(RuntimeError::DependencyNotFound { .. })
        ));
    }

    #[test]
    fn resolve_config_returns_not_found_for_unregistered() {
        let rt = RuntimeInner::for_test();
        let result: Result<ConfigValue<()>, RuntimeError> = rt.resolve_config();
        assert!(matches!(
            result,
            Err(RuntimeError::DependencyNotFound { .. })
        ));
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

    /// Asserts `result` is `Err(DependencyNotFound { type_name: expected, .. })`,
    /// panicking with the actual value otherwise.
    fn assert_dependency_not_found_named<T>(result: Result<T, RuntimeError>, expected: &str) {
        match result {
            Err(RuntimeError::DependencyNotFound { type_name, .. }) => {
                assert_eq!(type_name, expected);
            }
            Err(other) => panic!("expected DependencyNotFound, got {other:?}"),
            Ok(_) => panic!("expected DependencyNotFound, got Ok"),
        }
    }

    #[test]
    fn resolve_projection_returns_not_found_for_wrong_type() {
        let mut rt = RuntimeInner::for_test();
        // Register as String, request as MyProjection.
        let instance = Arc::new(String::from("not-a-projection")) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .projections
            .insert(TypeId::of::<String>(), instance);

        let result = rt.resolve_projection::<MyProjection>();
        assert_dependency_not_found_named(result, std::any::type_name::<MyProjection>());
    }

    #[test]
    fn resolve_adapter_returns_not_found_for_wrong_type() {
        let mut rt = RuntimeInner::for_test();
        let instance = Arc::new(String::from("not-an-adapter")) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .adapters
            .insert(TypeId::of::<String>(), instance);

        let result = rt.resolve_adapter::<MyProjection>();
        assert_dependency_not_found_named(result, std::any::type_name::<MyProjection>());
    }

    #[test]
    fn resolve_config_returns_not_found_for_wrong_type() {
        let mut rt = RuntimeInner::for_test();
        let instance = Arc::new(MyProjection(7)) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .configs
            .insert(TypeId::of::<MyProjection>(), instance);

        let result = rt.resolve_config::<String>();
        assert_dependency_not_found_named(result, std::any::type_name::<String>());
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
            assert!(matches!(r1, Err(RuntimeError::DependencyNotFound { .. })));
            assert!(matches!(r2, Err(RuntimeError::DependencyNotFound { .. })));
            assert!(matches!(r3, Err(RuntimeError::DependencyNotFound { .. })));
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

    // -- CrossTenantPermit issuer (CORE-008A Phase 4, AD-008) ---------------
    // AllowCrossTenant/DenyCrossTenant/authenticated_ctx moved to
    // crate::test_support (code-review fix: was duplicated with context/mod.rs's
    // copy, which had already drifted missing the Deny variant).

    use crate::test_support::{authenticated_ctx, AllowCrossTenant, DenyCrossTenant};

    // CORE-008A Phase 6 (TASK-028, FR-005/NFR-002): this test and its sibling
    // below already prove permit denial without a cross-tenant capability,
    // and denial even with resource/action-only authorization — the exact
    // scenarios TASK-028 specifies. `issue_cross_tenant_permit` is
    // `pub(crate)` (AD-008), so these scenarios cannot be exercised from the
    // external `tests/tenant_enforcement_contract.rs` acceptance suite; see
    // that file's module doc for the full explanation. No new test was added
    // for TASK-028's denial half.
    #[tokio::test]
    async fn issue_cross_tenant_permit_denied_without_capability() {
        let rt = RuntimeInner::for_test_with_authz(Arc::new(DenyCrossTenant));
        let ctx = authenticated_ctx();
        let destination = TenantId::new("tenant-b").unwrap();

        let result = rt.issue_cross_tenant_permit(&ctx, destination).await;

        assert!(matches!(
            result,
            Err(SecurityError::CrossTenantDenied { .. })
        ));
    }

    #[tokio::test]
    async fn issue_cross_tenant_permit_denied_even_with_resource_action_alone() {
        // A provider that denies the specific "tenant:cross-tenant-access"
        // capability check is functionally equivalent to "authorized for the
        // resource/action but without cross-tenant capability" (FR-005): the
        // permit issuer only ever asks for the cross-tenant capability, so
        // any Deny on that request is exactly this scenario.
        let rt = RuntimeInner::for_test_with_authz(Arc::new(DenyCrossTenant));
        let ctx = authenticated_ctx();
        let destination = TenantId::new("tenant-b").unwrap();

        let result = rt.issue_cross_tenant_permit(&ctx, destination).await;

        assert!(matches!(
            result,
            Err(SecurityError::CrossTenantDenied { .. })
        ));
    }

    // CORE-008A Phase 6 (TASK-028, FR-006/NFR-002 positive path): proves a
    // permit is minted end to end on an `Allow` decision. This covers
    // issuance only — see `enforce_tenant_succeeds_for_authorized_cross_tenant_grant`
    // below for the full issued → attached → consumed → operation-succeeds
    // flow FR-006 actually requires (AD-014).
    #[tokio::test]
    async fn issue_cross_tenant_permit_allowed_yields_destination_scoped_permit() {
        let rt = RuntimeInner::for_test_with_authz(Arc::new(AllowCrossTenant));
        let ctx = authenticated_ctx();
        let destination = TenantId::new("tenant-b").unwrap();

        let result = rt
            .issue_cross_tenant_permit(&ctx, destination.clone())
            .await;

        let permit = result.expect("Allow decision must yield a permit");
        assert_eq!(permit.destination(), &destination);
    }

    // FR-006 end-to-end acceptance scenario (AD-014): a Principal authenticated
    // on tenant-a, holding a validly-issued CrossTenantPermit for tenant-b,
    // invokes enforce_tenant with a hint of tenant-b — and it succeeds, not
    // rejected as a tenant violation. This is the test the original CORE-008A
    // review claimed was "already covered verbatim" by the issuance-only and
    // getter-only tests above; it was not — neither of those ever calls
    // enforce_tenant, so issuance and consumption were never actually proven
    // connected until this test.
    #[tokio::test]
    async fn enforce_tenant_succeeds_for_authorized_cross_tenant_grant() {
        use crate::runtime::CanonicalTenant;

        let rt = RuntimeInner::for_test_with_authz(Arc::new(AllowCrossTenant));
        let ctx = ctx_with_tenant(Some("tenant-a"));
        let destination = TenantId::new("tenant-b").unwrap();

        let permit = rt
            .issue_cross_tenant_permit(&ctx, destination.clone())
            .await
            .expect("Allow decision must yield a permit");

        let mut ctx = ctx
            .with_cross_tenant_access(&permit)
            .with_tenant_id("tenant-b");

        rt.enforce_tenant(&mut ctx)
            .expect("a valid grant for the requested destination must succeed, not TenantMismatch");

        assert_eq!(
            ctx.canonical_tenant()
                .and_then(CanonicalTenant::tenant_id)
                .map(TenantId::as_str),
            Some("tenant-b")
        );
    }

    #[tokio::test]
    async fn issue_cross_tenant_permit_without_provider_is_capability_not_enabled() {
        let rt = RuntimeInner::for_test();
        let ctx = authenticated_ctx();
        let destination = TenantId::new("tenant-b").unwrap();

        let result = rt.issue_cross_tenant_permit(&ctx, destination).await;

        assert!(matches!(result, Err(SecurityError::CapabilityNotEnabled)));
    }

    // -- CORE-008A Phase 2 (TASK-008): fallible enforce_tenant --------------

    use ego_security_sdk::context::SecurityContext;
    use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};

    fn ctx_with_tenant(tenant: Option<&str>) -> ServiceContext {
        let mut principal = Principal::new(PrincipalKind::User, SubjectId::new("alice").unwrap());
        principal.tenant_id = tenant.map(|t| TenantId::new(t).unwrap());
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

    // -- CORE-025 TASK-010: RuntimeInner::check_dependency presence check --
    // Test-first (RED before check_dependency exists): 4 arms — adapter,
    // config, and projection present/missing, plus Entity always-Err.

    #[test]
    fn check_dependency_adapter_present_is_ok() {
        let mut rt = RuntimeInner::for_test();
        let instance = Arc::new(MyProjection(1)) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .adapters
            .insert(TypeId::of::<MyProjection>(), instance);

        let dep = DepKey::Adapter(TypeId::of::<MyProjection>(), "MyProjection");
        assert!(rt.check_dependency(&dep).is_ok());
    }

    #[test]
    fn check_dependency_adapter_missing_is_err_named() {
        let rt = RuntimeInner::for_test();
        let dep = DepKey::Adapter(TypeId::of::<MyProjection>(), "MyProjection");
        assert_dependency_not_found_named(rt.check_dependency(&dep), "MyProjection");
    }

    #[test]
    fn check_dependency_config_present_is_ok() {
        let mut rt = RuntimeInner::for_test();
        let instance = Arc::new(String::from("v")) as Arc<dyn Any + Send + Sync>;
        rt.resolved.configs.insert(TypeId::of::<String>(), instance);

        let dep = DepKey::Config(TypeId::of::<String>(), "String");
        assert!(rt.check_dependency(&dep).is_ok());
    }

    #[test]
    fn check_dependency_config_missing_is_err_named() {
        let rt = RuntimeInner::for_test();
        let dep = DepKey::Config(TypeId::of::<String>(), "String");
        assert_dependency_not_found_named(rt.check_dependency(&dep), "String");
    }

    #[test]
    fn check_dependency_projection_present_is_ok() {
        let mut rt = RuntimeInner::for_test();
        let instance = Arc::new(MyProjection(2)) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .projections
            .insert(TypeId::of::<MyProjection>(), instance);

        let dep = DepKey::Projection(TypeId::of::<MyProjection>(), "MyProjection");
        assert!(rt.check_dependency(&dep).is_ok());
    }

    #[test]
    fn check_dependency_projection_missing_is_err_named() {
        let rt = RuntimeInner::for_test();
        let dep = DepKey::Projection(TypeId::of::<MyProjection>(), "MyProjection");
        assert_dependency_not_found_named(rt.check_dependency(&dep), "MyProjection");
    }

    // CORE-028 Stage 2C (task 3.6/3.8): `DepKey::Entity` is now a real
    // presence check against the `entities` table — replaces the retired
    // `check_dependency_entity_is_always_err_regardless_of_table_state`
    // pinning test (there was no entity table before this change).

    #[test]
    fn check_dependency_entity_present_is_ok() {
        let mut rt = RuntimeInner::for_test();
        let instance = Arc::new(MyProjection(3)) as Arc<dyn Any + Send + Sync>;
        rt.resolved
            .entities
            .insert(TypeId::of::<MyProjection>(), instance);

        let dep = DepKey::Entity(TypeId::of::<MyProjection>(), "MyProjection");
        assert!(rt.check_dependency(&dep).is_ok());
    }

    #[test]
    fn check_dependency_entity_missing_is_err_named() {
        let rt = RuntimeInner::for_test();
        let dep = DepKey::Entity(TypeId::of::<MyProjection>(), "MyProjection");
        assert_dependency_not_found_named(rt.check_dependency(&dep), "MyProjection");
    }

    // -- CORE-025 TASK-001: RuntimeError::DependencyNotFound struct variant --

    #[test]
    fn dependency_not_found_display_names_type_and_service_when_both_known() {
        let err = RuntimeError::DependencyNotFound {
            kind: DependencyKind::Adapter,
            type_name: "X",
            service_name: Some("Y"),
        };
        let msg = err.to_string();
        assert!(
            msg.contains('X'),
            "message must name the missing type: {msg}"
        );
        assert!(
            msg.contains('Y'),
            "message must name the requesting service: {msg}"
        );
    }

    #[test]
    fn dependency_not_found_display_omits_service_gracefully_when_none() {
        let err = RuntimeError::DependencyNotFound {
            kind: DependencyKind::Adapter,
            type_name: "X",
            service_name: None,
        };
        let msg = err.to_string();
        assert!(
            msg.contains('X'),
            "message must name the missing type: {msg}"
        );
    }

    #[test]
    fn dependency_not_found_is_a_real_std_error() {
        fn boxed_error() -> Result<(), Box<dyn std::error::Error>> {
            Err(RuntimeError::DependencyNotFound {
                kind: DependencyKind::Adapter,
                type_name: "X",
                service_name: Some("Y"),
            })?
        }
        let err = boxed_error().unwrap_err();
        assert!(err.to_string().contains('X'));
    }

    // DX follow-up: the message names the dependency KIND and the exact
    // builder method that registers it, so the caller reads the fix off the
    // error itself.
    #[test]
    fn dependency_not_found_display_names_kind_and_builder_method() {
        let err = RuntimeError::DependencyNotFound {
            kind: DependencyKind::Adapter,
            type_name: "X",
            service_name: Some("Y"),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("adapter dependency"),
            "must name the kind: {msg}"
        );
        assert!(msg.contains(".adapter"), "must name the fix method: {msg}");
    }

    // The Entity fix hint is parameterized on the missing aggregate type so it
    // reads `.entity::<MyAgg>()`, not a bare placeholder.
    #[test]
    fn dependency_not_found_entity_names_the_typed_entity_method() {
        let err = RuntimeError::DependencyNotFound {
            kind: DependencyKind::Entity,
            type_name: "MyAgg",
            service_name: None,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("entity dependency"),
            "must name the kind: {msg}"
        );
        assert!(
            msg.contains(".entity::<MyAgg>()"),
            "must name the typed method: {msg}"
        );
    }

    // DX follow-up: `check_dependency` threads the `DepKey` variant into the
    // error's `kind` for every dependency kind, rather than discarding it.
    #[test]
    fn check_dependency_missing_names_the_dependency_kind() {
        let rt = RuntimeInner::for_test();
        let cases = [
            (
                DepKey::Adapter(TypeId::of::<MyProjection>(), "MyProjection"),
                DependencyKind::Adapter,
            ),
            (
                DepKey::Config(TypeId::of::<String>(), "String"),
                DependencyKind::Config,
            ),
            (
                DepKey::Projection(TypeId::of::<MyProjection>(), "MyProjection"),
                DependencyKind::Projection,
            ),
            (
                DepKey::Entity(TypeId::of::<MyProjection>(), "MyProjection"),
                DependencyKind::Entity,
            ),
        ];
        for (dep, expected) in cases {
            match rt.check_dependency(&dep) {
                Err(RuntimeError::DependencyNotFound { kind, .. }) => assert_eq!(kind, expected),
                other => panic!("expected DependencyNotFound, got {other:?}"),
            }
        }
    }

    // DX follow-up (Part A): `ServiceNotFound` is typed — its message names the
    // missing tag and the fix method, and carries an optional requester.
    #[test]
    fn service_not_found_display_names_the_missing_tag_and_the_fix() {
        let err = RuntimeError::ServiceNotFound {
            type_name: "MyTag",
            required_by: None,
        };
        let msg = err.to_string();
        assert!(msg.contains("MyTag"), "must name the missing tag: {msg}");
        assert!(
            msg.contains(".service::<"),
            "must name the fix method: {msg}"
        );
        // The service_instance hint must show its full, copy-pasteable signature
        // (`::<Tag>(instance)`), not a bare `.service_instance()` that won't compile.
        assert!(
            msg.contains(".service_instance::<"),
            "must name the full service_instance signature: {msg}"
        );
    }

    #[test]
    fn service_not_found_display_names_the_requester_when_known() {
        let err = RuntimeError::ServiceNotFound {
            type_name: "MyTag",
            required_by: Some("Requester"),
        };
        let msg = err.to_string();
        assert!(msg.contains("MyTag"), "must name the missing tag: {msg}");
        assert!(msg.contains("Requester"), "must name the requester: {msg}");
    }

    // -- CORE-012A Phase 1 (TASK-001/002): SecurityDenialKind Display --

    #[test]
    fn security_denial_kind_display_yields_only_the_kind_label() {
        assert_eq!(
            SecurityDenialKind::MissingContext.to_string(),
            "MissingContext"
        );
        assert_eq!(
            SecurityDenialKind::TenantMismatch.to_string(),
            "TenantMismatch"
        );
        assert_eq!(
            SecurityDenialKind::AuthorizationDenied.to_string(),
            "AuthorizationDenied"
        );
    }

    // -- CORE-012A Phase 2 (TASK-003/004/005): record_security_denial helper --

    use crate::test_support::RecordingObservability;

    #[test]
    fn record_security_denial_emits_one_event_with_required_fields() {
        let obs = Arc::new(RecordingObservability::new());
        let rt = RuntimeInner::for_test_with_observability(obs.clone());

        rt.record_security_denial("Svc", "op", SecurityDenialKind::AuthorizationDenied);

        let events = obs.events.lock().unwrap();
        assert_eq!(events.len(), 1, "expected exactly one recorded event");
        let event = &events[0];
        assert_eq!(
            event.metadata.get("denial_kind").map(String::as_str),
            Some("AuthorizationDenied")
        );
        assert_eq!(
            event.metadata.get("service").map(String::as_str),
            Some("Svc")
        );
        assert_eq!(
            event.metadata.get("operation").map(String::as_str),
            Some("op")
        );
    }

    #[test]
    fn record_security_denial_is_a_silent_no_op_without_observability() {
        // observability: None (AD-2 default) — for_test() already yields this.
        let rt = RuntimeInner::for_test();

        // Must not panic; there is no sink to assert on, which is the point.
        rt.record_security_denial("Svc", "op", SecurityDenialKind::MissingContext);
    }

    /// The three losses call for different responses, and an operator must not
    /// have to infer that from the reason tag. What travels is the **action**,
    /// not a claim about whether the failure would recur — that is not something
    /// this code can establish.
    #[test]
    fn a_lost_completion_reports_what_to_do_about_it() {
        use crate::test_support::RecordingObservability;

        let obs = Arc::new(RecordingObservability::new());
        let rt = RuntimeInner::for_test_with_observability(obs.clone());

        rt.record_completion_lost("store_unavailable", OperatorAction::MonitorRate, "down");
        rt.record_completion_lost(
            "not_encodable",
            OperatorAction::Investigate,
            "this value did not serialise",
        );

        let recorded: Vec<(String, String)> = obs
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e.metadata.get("reason").cloned().unwrap_or_default(),
                    e.metadata.get("action").cloned().unwrap_or_default(),
                )
            })
            .collect();

        assert_eq!(
            recorded,
            vec![
                ("store_unavailable".to_string(), "monitor_rate".to_string()),
                ("not_encodable".to_string(), "investigate".to_string()),
            ]
        );
    }

    /// A runtime with no sink has nowhere to report this, and must still not
    /// panic — the operation it belongs to already succeeded.
    #[test]
    fn a_lost_completion_is_a_silent_no_op_without_observability() {
        let rt = RuntimeInner::for_test();

        rt.record_completion_lost("store_unavailable", OperatorAction::MonitorRate, "down");
    }

    /// Same isolation as a security denial, and for a sharper reason: a panic
    /// here would unwind through an operation that already committed its work.
    #[test]
    fn a_lost_completion_isolates_a_panicking_observability_sink() {
        use crate::test_support::PanickingObservability;

        let rt = RuntimeInner::for_test_with_observability(Arc::new(PanickingObservability));

        rt.record_completion_lost(
            "stale_owner",
            OperatorAction::ReviewLeaseDuration,
            "stale fence",
        );
    }

    #[test]
    fn record_security_denial_isolates_a_panicking_observability_sink() {
        // RESIL-001 (CORE-012A 4R review): a caller-supplied Observability
        // implementor is untrusted, same as AuthorizationProvider. A panic
        // inside trace() must not unwind through the security-denial path.
        use crate::test_support::PanickingObservability;

        let rt = RuntimeInner::for_test_with_observability(Arc::new(PanickingObservability));

        rt.record_security_denial("Svc", "op", SecurityDenialKind::AuthorizationDenied);
    }

    #[test]
    fn authorization_provider_returns_arc_when_providers_set() {
        use async_trait::async_trait;
        use ego_security_sdk::authentication::AuthenticationProvider;
        use ego_security_sdk::authorization::AuthorizationProvider;
        use ego_security_sdk::context::SecurityContext;
        use ego_security_sdk::credential::Credential;
        use ego_security_sdk::principal::Principal;
        use ego_security_sdk::AuthenticationError;
        use ego_security_sdk::{AccessRequest, AuthorizationDecision, SecurityError};

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
            DependencyTable::with_registrations(RegisteredDependencies {
                adapters: HashMap::new(),
                configs: HashMap::new(),
                projections: HashMap::new(),
                entities: HashMap::new(),
            }),
            None,
            Mutex::new(TeardownStack::new()),
            TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly),
            IdempotencyEnforcementMode::Compatibility,
            None,
            None,
            None,
            Duration::from_secs(5),
            None,
            None,
        );

        let result = rt.authorization_provider();
        assert!(result.is_some(), "Expected Some when providers are set");
        assert_eq!(
            Arc::as_ptr(result.as_ref().unwrap()),
            authz_ptr,
            "Returned Arc must point to the same AuthorizationProvider"
        );
    }

    // -- The reservation namespace: three states, not two --------------------
    //
    // These live in-crate because the third state cannot be reached from
    // outside it. `CanonicalTenant::systemwide()` is `pub(super)` and
    // `ServiceContext::set_resolved_tenant` is `pub(crate)` — deliberately, since
    // `TenantResolver` is meant to be the only thing that resolves a scope. So a
    // test in `tests/` can build "no scope" and "scope with a tenant" but not
    // "scope resolved to systemwide", and that last one is exactly the state the
    // fix must not have broken.
    //
    // The behavioural pair over generated dispatch lives in
    // `crates/service-sdk/tests/cross_tenant_reservation_isolation.rs`.

    /// Records the scope each `reserve` was handed, and answers from a script.
    ///
    /// The answer is a full `Result`, so a test can script a store *failure* as
    /// well as an outcome — which is what the span's `Ok`/`Error` classification
    /// has to be judged against.
    struct ScopeRecordingStore {
        scopes: Mutex<Vec<Option<TenantId>>>,
        answer: Mutex<Option<Result<ReservationOutcome, ReservationError>>>,
    }

    impl ScopeRecordingStore {
        fn answering(answer: ReservationOutcome) -> Arc<Self> {
            Self::answering_with(Ok(answer))
        }

        fn answering_with(answer: Result<ReservationOutcome, ReservationError>) -> Arc<Self> {
            Arc::new(Self {
                scopes: Mutex::new(Vec::new()),
                answer: Mutex::new(Some(answer)),
            })
        }

        fn scopes(&self) -> Vec<Option<TenantId>> {
            self.scopes.lock().expect("not poisoned").clone()
        }
    }

    #[async_trait::async_trait]
    impl OperationReservationStore for ScopeRecordingStore {
        async fn reserve(
            &self,
            req: ReserveRequest,
        ) -> Result<ReservationOutcome, ReservationError> {
            self.scopes
                .lock()
                .expect("not poisoned")
                .push(req.tenant.clone());
            self.answer
                .lock()
                .expect("not poisoned")
                .take()
                .expect("each store answers exactly one reserve")
        }
        async fn renew(
            &self,
            _f: &OwnerFence,
            _u: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), ReservationError> {
            panic!("these tests only reserve");
        }
        async fn complete(
            &self,
            _f: &OwnerFence,
            _r: StoredServiceResponse,
        ) -> Result<(), ReservationError> {
            panic!("these tests only reserve");
        }
        async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
            panic!("these tests only reserve");
        }
        async fn purge_completed_before(
            &self,
            _c: chrono::DateTime<chrono::Utc>,
            _b: usize,
        ) -> Result<u64, ReservationError> {
            panic!("these tests only reserve");
        }
        async fn probe(&self) -> Result<(), ReservationError> {
            Ok(())
        }
    }

    struct FixedClock;
    impl Clock for FixedClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            use chrono::TimeZone;
            chrono::Utc
                .timestamp_opt(1_000, 0)
                .single()
                .expect("valid instant")
        }
    }

    fn reserving_runtime(store: Arc<ScopeRecordingStore>) -> Runtime {
        RuntimeBuilder::new()
            .with_operation_reservation_store(store)
            .with_reservation_clock(Arc::new(FixedClock))
            .with_reservation_owner_id(OwnerId::new("in-crate-owner"))
            .with_reservation_lease_duration(Duration::from_secs(30))
            .build()
    }

    /// One recorded `start_span` call: everything the runtime handed the port.
    ///
    /// The `TraceContext` is kept whole rather than reduced to a name, because the
    /// parent linkage is a property under test: an earlier version of this spy
    /// ignored the context, and with it a mutation reusing the request's context
    /// instead of deriving a child left the whole suite green.
    #[derive(Clone, Debug)]
    struct StartedSpan {
        name: String,
        ctx: ego_domain::TraceContext,
        attrs: SpanAttributes,
    }

    /// Records every span the runtime opens **and how it closed** — the context,
    /// the attributes, and the terminal outcome.
    ///
    /// Every field here exists because dropping it made a real mutation
    /// undetectable. Attributes: without them, a span carrying no token looks like
    /// one carrying the right token. Context: without it, reusing the request's
    /// `SpanId` instead of a child's is invisible. Outcome: without it, ending
    /// every span `Ok`, or every span `Error`, is invisible.
    struct SpanRecordingTracer {
        started: Mutex<Vec<StartedSpan>>,
        ended: Mutex<Vec<(ego_domain::SpanId, SpanOutcome)>>,
    }

    impl SpanRecordingTracer {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                started: Mutex::new(Vec::new()),
                ended: Mutex::new(Vec::new()),
            })
        }

        fn started(&self) -> Vec<StartedSpan> {
            self.started.lock().expect("not poisoned").clone()
        }

        fn ended(&self) -> Vec<(ego_domain::SpanId, SpanOutcome)> {
            self.ended.lock().expect("not poisoned").clone()
        }

        fn ended_count(&self) -> usize {
            self.ended.lock().expect("not poisoned").len()
        }

        /// The single span this runtime opened, or a failure naming what it saw.
        fn only_span(&self) -> StartedSpan {
            let started = self.started();
            assert_eq!(
                started.len(),
                1,
                "expected exactly one span, got {:?}",
                started.iter().map(|s| s.name.clone()).collect::<Vec<_>>()
            );
            started[0].clone()
        }

        /// How the single span closed.
        fn only_outcome(&self) -> SpanOutcome {
            let ended = self.ended();
            assert_eq!(ended.len(), 1, "expected exactly one closed span");
            ended[0].1.clone()
        }
    }

    impl Tracer for SpanRecordingTracer {
        fn start_span(&self, ctx: &ego_domain::TraceContext, name: &str, attrs: SpanAttributes) {
            self.started
                .lock()
                .expect("not poisoned")
                .push(StartedSpan {
                    name: name.to_string(),
                    ctx: *ctx,
                    attrs,
                });
        }
        fn end_span(&self, span: ego_domain::SpanId, outcome: SpanOutcome) {
            self.ended
                .lock()
                .expect("not poisoned")
                .push((span, outcome));
        }
    }

    fn reserving_runtime_with_tracer(
        store: Arc<ScopeRecordingStore>,
        tracer: Arc<SpanRecordingTracer>,
    ) -> Runtime {
        RuntimeBuilder::new()
            .with_operation_reservation_store(store)
            .with_reservation_clock(Arc::new(FixedClock))
            .with_reservation_owner_id(OwnerId::new("in-crate-owner"))
            .with_reservation_lease_duration(Duration::from_secs(30))
            .with_tracer(tracer as Arc<dyn Tracer>)
            .build()
    }

    /// A keyed reservation opens `idempotency.reserve`, carrying the token of the
    /// key that was actually presented.
    ///
    /// The token is compared against one this test derives from its own key, so
    /// the assertion is that the *presented* key was hashed — not merely that some
    /// 16-hex value appeared. An implementation hashing a constant, or the wrong
    /// value, produces a different token and fails here.
    #[tokio::test]
    async fn a_reservation_opens_a_span_carrying_the_presented_keys_token() {
        use crate::runtime::CanonicalTenant;

        let tenant = TenantId::new("tenant-a").expect("valid tenant");
        let store = ScopeRecordingStore::answering(ReservationOutcome::Fresh(lease_for(Some(
            tenant.clone(),
        ))));
        let tracer = SpanRecordingTracer::new();
        let rt = reserving_runtime_with_tracer(store, tracer.clone());

        let mut ctx = ServiceContext::new()
            .with_operation_key(scope_test_key())
            .with_trace_context(ego_domain::TraceContext::root());
        ctx.set_resolved_tenant(CanonicalTenant::scoped(tenant));

        rt.inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("the reservation proceeds");

        let span = tracer.only_span();
        assert_eq!(span.name, "idempotency.reserve");
        assert_eq!(
            span.attrs.operation_key_hash(),
            Some(OperationKeyHash::of(&scope_test_key()).as_str()),
            "the span must carry the token of the key this dispatch presented"
        );
        assert_eq!(
            tracer.ended_count(),
            1,
            "an opened span must be closed, or the adapter's table leaks it"
        );
    }

    /// A store whose `reserve` announces that it was entered and then never returns.
    ///
    /// The two halves are both needed: the notification is what lets the test know
    /// the future has reached its `.await` — rather than guessing with a sleep — and
    /// the park is what makes the future genuinely cancellable at that point.
    struct ParkingStore {
        entered: Arc<tokio::sync::Notify>,
    }

    impl ParkingStore {
        fn new() -> (Arc<Self>, Arc<tokio::sync::Notify>) {
            let entered = Arc::new(tokio::sync::Notify::new());
            (
                Arc::new(Self {
                    entered: entered.clone(),
                }),
                entered,
            )
        }
    }

    #[async_trait::async_trait]
    impl OperationReservationStore for ParkingStore {
        async fn reserve(
            &self,
            _req: ReserveRequest,
        ) -> Result<ReservationOutcome, ReservationError> {
            // `notify_one`, not `notify_waiters`. `Notified` registers on its first
            // poll, and `select!` polls its branches in an unspecified order — so with
            // `notify_waiters` a run that polled the reservation first would fire into
            // no waiter, lose the signal, and then park forever. A permit-leaving
            // notification makes the handshake independent of that order.
            self.entered.notify_one();
            // Parked forever. The test cancels instead of releasing.
            std::future::pending().await
        }
        async fn renew(
            &self,
            _f: &OwnerFence,
            _u: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), ReservationError> {
            unreachable!("this store only parks in reserve")
        }
        async fn complete(
            &self,
            _f: &OwnerFence,
            _r: StoredServiceResponse,
        ) -> Result<(), ReservationError> {
            unreachable!("this store only parks in reserve")
        }
        async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
            unreachable!("this store only parks in reserve")
        }
        async fn purge_completed_before(
            &self,
            _c: chrono::DateTime<chrono::Utc>,
            _b: usize,
        ) -> Result<u64, ReservationError> {
            unreachable!("this store only parks in reserve")
        }
        async fn probe(&self) -> Result<(), ReservationError> {
            Ok(())
        }
    }

    /// Reserves successfully, then refuses the completion as `StaleOwner`.
    ///
    /// The pair is the point: the runtime has to mint a real permit before there is
    /// anything to complete, and the refused completion is what the counter counts.
    struct StaleCompletingStore;

    impl StaleCompletingStore {
        fn new() -> Arc<Self> {
            Arc::new(Self)
        }
    }

    #[async_trait::async_trait]
    impl OperationReservationStore for StaleCompletingStore {
        async fn reserve(
            &self,
            req: ReserveRequest,
        ) -> Result<ReservationOutcome, ReservationError> {
            Ok(ReservationOutcome::Fresh(Lease {
                operation_id: OperationId::new(req.tenant.clone(), req.operation_key.clone()),
                owner_id: req.owner_id.clone(),
                fencing_token: FencingToken::initial(),
                lease_until: req.lease_until,
            }))
        }
        async fn renew(
            &self,
            _f: &OwnerFence,
            _u: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), ReservationError> {
            unreachable!("no runtime component renews — that is why there is no metric")
        }
        async fn complete(
            &self,
            _f: &OwnerFence,
            _r: StoredServiceResponse,
        ) -> Result<(), ReservationError> {
            Err(ReservationError::StaleOwner)
        }
        async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
            unreachable!("no runtime component abandons — that is why there is no metric")
        }
        async fn purge_completed_before(
            &self,
            _c: chrono::DateTime<chrono::Utc>,
            _b: usize,
        ) -> Result<u64, ReservationError> {
            unreachable!("this store exists for the completion path")
        }
        async fn probe(&self) -> Result<(), ReservationError> {
            Ok(())
        }
    }

    /// Reserves successfully and **accepts** the completion.
    ///
    /// The counterpart of `StaleCompletingStore`, and it exists because the negative
    /// control needs a completion that actually succeeds. Without it that test only
    /// reserved, so a mutation counting `stale_owner` on *every* completion — success
    /// included — went undetected: measured, and it is why this store is here.
    struct CompletingStore;

    impl CompletingStore {
        fn new() -> Arc<Self> {
            Arc::new(Self)
        }
    }

    #[async_trait::async_trait]
    impl OperationReservationStore for CompletingStore {
        async fn reserve(
            &self,
            req: ReserveRequest,
        ) -> Result<ReservationOutcome, ReservationError> {
            Ok(ReservationOutcome::Fresh(Lease {
                operation_id: OperationId::new(req.tenant.clone(), req.operation_key.clone()),
                owner_id: req.owner_id.clone(),
                fencing_token: FencingToken::initial(),
                lease_until: req.lease_until,
            }))
        }
        async fn renew(
            &self,
            _f: &OwnerFence,
            _u: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), ReservationError> {
            unreachable!("no runtime component renews")
        }
        async fn complete(
            &self,
            _f: &OwnerFence,
            _r: StoredServiceResponse,
        ) -> Result<(), ReservationError> {
            Ok(())
        }
        async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
            unreachable!("no runtime component abandons")
        }
        async fn purge_completed_before(
            &self,
            _c: chrono::DateTime<chrono::Utc>,
            _b: usize,
        ) -> Result<u64, ReservationError> {
            unreachable!("this store exists for the completion path")
        }
        async fn probe(&self) -> Result<(), ReservationError> {
            Ok(())
        }
    }

    /// Completes an operation whose fence the store rejects, and returns the metrics.
    async fn metrics_for_a_stale_completion(obs: Arc<crate::test_support::RecordingObservability>) {
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(StaleCompletingStore::new())
            .with_reservation_clock(Arc::new(FixedClock))
            .with_reservation_owner_id(OwnerId::new("in-crate-owner"))
            .with_reservation_lease_duration(Duration::from_secs(30))
            .with_observability(obs as Arc<dyn Observability>)
            .build();

        // A *real* permit, obtained by reserving through the runtime rather than
        // synthesised: `ReservationPermit` has no test constructor, and building one
        // would assert against a fence this path never minted.
        let mut ctx = ServiceContext::new().with_operation_key(scope_test_key());
        ctx.set_resolved_tenant(crate::runtime::CanonicalTenant::scoped(
            TenantId::new("tenant-a").expect("valid tenant"),
        ));
        let decision = rt
            .inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("the reservation proceeds")
            .expect("a decision was made");

        rt.inner()
            .complete_idempotent_operation(Some(&decision), &"answer")
            .await;
    }

    /// A completion the store refuses as `StaleOwner` counts
    /// `idempotency.lease.stale_owner` carrying `operation = complete`.
    ///
    /// This is the signal a rate alert fires on. Without it a lease configured too
    /// short is invisible in aggregate: each individual case emits an operator event,
    /// but nothing says "this is happening a hundred times an hour", which is the
    /// difference between a curiosity and a misconfiguration.
    ///
    /// The whole record is compared. `complete` is the only admissible value of
    /// `operation` (AD-10d), so a name-only assertion would pass with the dimension
    /// missing entirely — which is exactly what a call site left on the old folded
    /// name would produce.
    #[tokio::test]
    async fn a_stale_completion_counts_the_stale_owner_metric() {
        let obs = Arc::new(crate::test_support::RecordingObservability::new());
        metrics_for_a_stale_completion(obs.clone()).await;

        let stale: Vec<_> = obs
            .records()
            .into_iter()
            .filter(|m| m.name == "idempotency.lease.stale_owner")
            .collect();
        assert_eq!(
            stale.len(),
            1,
            "a discarded completion counts exactly once, or a rate is not a rate: {:?}",
            obs.metric_names()
        );
        assert_eq!(
            (stale[0].kind, stale[0].value, stale[0].attributes.clone()),
            (
                ego_domain::MetricKind::Counter,
                1.0,
                vec![("operation".to_string(), "complete".to_string())]
            ),
            "one counter increment carrying the operation that hit the stale owner"
        );
    }

    /// No folded `stale_owner` name survives the migration.
    ///
    /// The value used to live in the name, so the failure this guards is a call site
    /// left behind — which the assertion above cannot see, because it filters for
    /// the new name and a stale emitter simply would not appear.
    #[tokio::test]
    async fn a_stale_completion_emits_no_folded_name() {
        let obs = Arc::new(crate::test_support::RecordingObservability::new());
        metrics_for_a_stale_completion(obs.clone()).await;

        let names = obs.metric_names();
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with("idempotency.lease.stale_owner.")),
            "the operation belongs in a dimension; no name may carry it: {names:?}"
        );
    }

    /// A completion the store **accepts** counts no stale owner.
    ///
    /// The negative control, and the version of it that works. An earlier one only
    /// *reserved* — it never completed anything — so a mutation emitting the counter on
    /// every completion, success included, passed it untouched. Measured, not
    /// theorised. A counter that fires on success would make its own alert meaningless
    /// while looking correct in aggregate.
    #[tokio::test]
    async fn a_successful_completion_counts_no_stale_owner() {
        let obs = Arc::new(crate::test_support::RecordingObservability::new());
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(CompletingStore::new())
            .with_reservation_clock(Arc::new(FixedClock))
            .with_reservation_owner_id(OwnerId::new("in-crate-owner"))
            .with_reservation_lease_duration(Duration::from_secs(30))
            .with_observability(obs.clone() as Arc<dyn Observability>)
            .build();

        let mut ctx = ServiceContext::new().with_operation_key(scope_test_key());
        ctx.set_resolved_tenant(crate::runtime::CanonicalTenant::scoped(
            TenantId::new("tenant-a").expect("valid tenant"),
        ));
        let decision = rt
            .inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("the reservation proceeds")
            .expect("a decision was made");

        // The completion the store accepts. This is the call the earlier control was
        // missing entirely.
        rt.inner()
            .complete_idempotent_operation(Some(&decision), &"answer")
            .await;

        assert!(
            !obs.metric_names()
                .iter()
                .any(|n| n.starts_with("idempotency.lease.stale_owner")),
            "the completion was accepted, so nothing lost a fence: {:?}",
            obs.metric_names()
        );
    }

    /// An uninstrumented runtime handles the same refusal identically.
    #[tokio::test]
    async fn an_uninstrumented_stale_completion_is_handled_the_same() {
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(StaleCompletingStore::new())
            .with_reservation_clock(Arc::new(FixedClock))
            .with_reservation_owner_id(OwnerId::new("in-crate-owner"))
            .with_reservation_lease_duration(Duration::from_secs(30))
            .build();

        let mut ctx = ServiceContext::new().with_operation_key(scope_test_key());
        ctx.set_resolved_tenant(crate::runtime::CanonicalTenant::scoped(
            TenantId::new("tenant-a").expect("valid tenant"),
        ));
        let decision = rt
            .inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("the reservation proceeds")
            .expect("a decision was made");

        // Returns rather than panicking: a discarded completion is reported, never
        // fatal, and metrics must not change that.
        rt.inner()
            .complete_idempotent_operation(Some(&decision), &"answer")
            .await;
    }

    /// A cancelled reserve closes its span exactly once, as an error.
    ///
    /// The leak this prevents, stated as the mechanism rather than as a worry: the
    /// `.await` on `reserve` is a cancellation point, and `end_span` is only reached
    /// after it. A dropped future therefore left the adapter holding a live entry —
    /// and the adapter's table is bounded and **drops new spans at capacity instead
    /// of evicting**, so leaked entries accumulate until tracing silently stops. A
    /// client disconnect, a timeout wrapper, or a losing `select!` branch all reach
    /// it, so this is ordinary traffic rather than an edge.
    ///
    /// Construction: the store announces entry and then parks, so the future is
    /// pinned at exactly the cancellation point — no sleep, no race. The span is
    /// asserted open and *not yet closed* before the drop, so the single `end_span`
    /// afterwards can only have come from the guard.
    ///
    /// A panic unwinding through the same frame is covered by the same mechanism and
    /// is not separately tested: `Drop` is what runs in both cases, and a test that
    /// panicked inside the store would be asserting on `catch_unwind` rather than on
    /// this guard.
    #[tokio::test]
    async fn a_cancelled_reserve_still_closes_its_span_exactly_once_as_an_error() {
        use crate::runtime::CanonicalTenant;

        let (store, entered) = ParkingStore::new();
        let tracer = SpanRecordingTracer::new();
        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(store)
            .with_reservation_clock(Arc::new(FixedClock))
            .with_reservation_owner_id(OwnerId::new("in-crate-owner"))
            .with_reservation_lease_duration(Duration::from_secs(30))
            .with_tracer(tracer.clone() as Arc<dyn Tracer>)
            .build();

        let mut ctx = ServiceContext::new()
            .with_operation_key(scope_test_key())
            .with_trace_context(ego_domain::TraceContext::root());
        ctx.set_resolved_tenant(CanonicalTenant::scoped(
            TenantId::new("tenant-a").expect("valid tenant"),
        ));

        let notified = entered.notified();
        let inner = rt.inner();
        // `Box::pin`, not `tokio::pin!`: the latter shadows the binding with a
        // `Pin<&mut _>`, so `drop(reserving)` would drop a *reference* and leave the
        // future alive in a hidden local until the end of scope — which is how an
        // earlier version of this test observed no cancellation at all and briefly
        // looked like a missing guard. Owning the future is what makes the drop the
        // cancellation.
        let mut reserving =
            Box::pin(inner.reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp")));
        tokio::pin!(notified);

        // Drive the future only until the store reports it is parked. `&mut`, so
        // losing this `select!` does not drop it yet.
        tokio::select! {
            _ = &mut reserving => panic!("a parked store cannot let the reservation complete"),
            _ = &mut notified => {}
        }

        let span = tracer.only_span();
        assert_eq!(span.name, "idempotency.reserve");
        assert!(
            tracer.ended().is_empty(),
            "the span must still be open at the cancellation point, or this test \
             would be asserting about the normal path"
        );

        // The cancellation.
        drop(reserving);

        let ended = tracer.ended();
        assert_eq!(
            ended.len(),
            1,
            "a cancelled attempt must close its span exactly once, got {ended:?}"
        );
        assert_eq!(
            ended[0].0,
            span.ctx.span_id(),
            "the closed span must be the one that was opened"
        );
        match &ended[0].1 {
            SpanOutcome::Error { status_message } => assert!(
                !status_message.is_empty(),
                "an abandoned attempt needs a message naming what happened"
            ),
            SpanOutcome::Ok => panic!(
                "an abandoned attempt never learned its result, so Ok would claim \
                 something nobody observed"
            ),
        }
    }

    /// The span's context is a **child** of the request's: same trace, new span id,
    /// parent pointing back at the request's span.
    ///
    /// This is the assertion that was missing while the spy discarded the context,
    /// and its absence was not harmless — reusing `parent` instead of deriving a
    /// child left the whole suite green, while the adapter's `SpanId`-keyed table
    /// would have silently dropped either this span or the interceptor's as a
    /// duplicate start.
    ///
    /// All three parts are checked. Same `trace_id` or the span leaves the trace.
    /// A new `span_id` or it collides. And `parent_span_id` equal to the request's
    /// span, or the collector cannot stitch it under the request that caused it.
    #[tokio::test]
    async fn the_span_context_is_a_child_of_the_requests() {
        use crate::runtime::CanonicalTenant;

        let tenant = TenantId::new("tenant-a").expect("valid tenant");
        let store = ScopeRecordingStore::answering(ReservationOutcome::Fresh(lease_for(Some(
            tenant.clone(),
        ))));
        let tracer = SpanRecordingTracer::new();
        let rt = reserving_runtime_with_tracer(store, tracer.clone());

        let request_ctx = ego_domain::TraceContext::root();
        let mut ctx = ServiceContext::new()
            .with_operation_key(scope_test_key())
            .with_trace_context(request_ctx);
        ctx.set_resolved_tenant(CanonicalTenant::scoped(tenant));

        rt.inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("the reservation proceeds");

        let span = tracer.only_span();
        assert_eq!(
            span.ctx.trace_id(),
            request_ctx.trace_id(),
            "a child must stay in the request's trace"
        );
        assert_ne!(
            span.ctx.span_id(),
            request_ctx.span_id(),
            "the span needs its own id: the adapter's table is keyed on it, and a \
             duplicate start for a live id is dropped"
        );
        assert_eq!(
            span.ctx.parent_span_id(),
            Some(request_ctx.span_id()),
            "the parent must be the request's span, or nothing can stitch this \
             under the request that caused it"
        );
        // And the span that was closed is this one, not the request's.
        assert_eq!(
            tracer.ended().first().map(|(id, _)| *id),
            Some(span.ctx.span_id())
        );
    }

    /// A store that could not answer ends the span `Error`.
    ///
    /// Half of the classification, and the half a dashboard alerts on. While the
    /// spy discarded the outcome, ending every span `Error` — or every span `Ok` —
    /// was invisible.
    #[tokio::test]
    async fn a_store_that_could_not_answer_ends_the_span_as_an_error() {
        use crate::runtime::CanonicalTenant;

        let tenant = TenantId::new("tenant-a").expect("valid tenant");
        let store =
            ScopeRecordingStore::answering_with(Err(ReservationError::Backend("boom".into())));
        let tracer = SpanRecordingTracer::new();
        let rt = reserving_runtime_with_tracer(store, tracer.clone());

        let mut ctx = ServiceContext::new()
            .with_operation_key(scope_test_key())
            .with_trace_context(ego_domain::TraceContext::root());
        ctx.set_resolved_tenant(CanonicalTenant::scoped(tenant));

        let result = rt
            .inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await;
        match result {
            Err(rejection) => assert_eq!(rejection, ReservationRejection::StoreUnavailable),
            Ok(other) => panic!("a failed store must refuse, got {other:?}"),
        }

        match tracer.only_outcome() {
            SpanOutcome::Error { status_message } => assert!(
                !status_message.is_empty(),
                "an error outcome needs a message an operator can read"
            ),
            SpanOutcome::Ok => {
                panic!("a store that could not answer is a failed attempt, not a completed one")
            }
        }
    }

    /// A refusal on the merits ends the span `Ok`.
    ///
    /// `Conflict` is an *answer*: the attempt completed and the store said no. The
    /// span describes the attempt, so recording this as an error would make every
    /// correctly-refused duplicate look like an outage on any dashboard that counts
    /// span errors — and duplicates are the ordinary traffic this whole mechanism
    /// exists to absorb.
    #[tokio::test]
    async fn a_refusal_on_the_merits_ends_the_span_ok() {
        use crate::runtime::CanonicalTenant;

        for refused in [
            ReservationOutcome::Conflict,
            ReservationOutcome::OtherInProgress,
        ] {
            let tenant = TenantId::new("tenant-a").expect("valid tenant");
            let store = ScopeRecordingStore::answering(refused.clone());
            let tracer = SpanRecordingTracer::new();
            let rt = reserving_runtime_with_tracer(store, tracer.clone());

            let mut ctx = ServiceContext::new()
                .with_operation_key(scope_test_key())
                .with_trace_context(ego_domain::TraceContext::root());
            ctx.set_resolved_tenant(CanonicalTenant::scoped(tenant));

            let result = rt
                .inner()
                .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
                .await;
            assert!(result.is_err(), "{refused:?} refuses the dispatch");

            assert_eq!(
                tracer.only_outcome(),
                SpanOutcome::Ok,
                "{refused:?} is an answer the store gave, so the attempt completed"
            );
        }
    }

    /// And a successful reservation ends `Ok` too — the third classification, so
    /// the `Error` case above is shown to be the exception rather than the rule.
    #[tokio::test]
    async fn a_successful_reservation_ends_the_span_ok() {
        use crate::runtime::CanonicalTenant;

        let tenant = TenantId::new("tenant-a").expect("valid tenant");
        let store = ScopeRecordingStore::answering(ReservationOutcome::Fresh(lease_for(Some(
            tenant.clone(),
        ))));
        let tracer = SpanRecordingTracer::new();
        let rt = reserving_runtime_with_tracer(store, tracer.clone());

        let mut ctx = ServiceContext::new()
            .with_operation_key(scope_test_key())
            .with_trace_context(ego_domain::TraceContext::root());
        ctx.set_resolved_tenant(CanonicalTenant::scoped(tenant));

        rt.inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("the reservation proceeds");

        assert_eq!(tracer.only_outcome(), SpanOutcome::Ok);
    }

    /// A request that carries a `TraceContext` but reaches a runtime with **no
    /// tracer** still reserves.
    ///
    /// The other half of the no-op rule. The neighbouring test covers tracer
    /// present with no context; without this one, an implementation that unwrapped
    /// an absent tracer would pass everything and panic in the deployments that
    /// register none — which is most of them.
    #[tokio::test]
    async fn a_traced_request_against_an_untraced_runtime_still_reserves() {
        use crate::runtime::CanonicalTenant;

        let tenant = TenantId::new("tenant-a").expect("valid tenant");
        let store = ScopeRecordingStore::answering(ReservationOutcome::Fresh(lease_for(Some(
            tenant.clone(),
        ))));
        // No `with_tracer`, deliberately.
        let rt = reserving_runtime(store.clone());

        let mut ctx = ServiceContext::new()
            .with_operation_key(scope_test_key())
            .with_trace_context(ego_domain::TraceContext::root());
        ctx.set_resolved_tenant(CanonicalTenant::scoped(tenant.clone()));

        rt.inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("a runtime with no tracer still reserves");

        assert_eq!(
            store.scopes(),
            vec![Some(tenant)],
            "the reservation happened; only its span did not"
        );
    }

    /// The runtime retains the **same** `Arc` that was registered — not an
    /// equivalent tracer.
    ///
    /// `Arc::ptr_eq`, because nothing weaker can tell the two apart: a second
    /// instance of the same type would satisfy every behavioural assertion in this
    /// file while being a different object, and a build that constructed one tracer
    /// for the interceptor and another for this field would look correct in every
    /// test that only counts spans.
    #[test]
    fn the_runtime_retains_the_registered_tracer_itself() {
        let store = ScopeRecordingStore::answering(ReservationOutcome::Fresh(lease_for(None)));
        let registered = SpanRecordingTracer::new();
        let as_port: Arc<dyn Tracer> = registered.clone();

        let rt = RuntimeBuilder::new()
            .with_operation_reservation_store(store)
            .with_reservation_clock(Arc::new(FixedClock))
            .with_reservation_owner_id(OwnerId::new("in-crate-owner"))
            .with_reservation_lease_duration(Duration::from_secs(30))
            .with_tracer(as_port.clone())
            .build();

        let retained = rt
            .inner()
            .tracer
            .as_ref()
            .expect("a registered tracer must be retained");
        assert!(
            Arc::ptr_eq(retained, &as_port),
            "the retained tracer must be the registered object, not another one \
             built alongside it"
        );
    }

    /// No `TraceContext` on the request means no span, and dispatch is unaffected.
    ///
    /// Same rule `TracingInterceptor` already follows: a request that originated no
    /// trace makes every hook a no-op. Opening a root span here instead would mint
    /// orphans attached to no trace, and would do it on the hot path of every
    /// keyed operation.
    #[tokio::test]
    async fn a_request_with_no_trace_context_opens_no_span_and_still_reserves() {
        use crate::runtime::CanonicalTenant;

        let tenant = TenantId::new("tenant-a").expect("valid tenant");
        let store = ScopeRecordingStore::answering(ReservationOutcome::Fresh(lease_for(Some(
            tenant.clone(),
        ))));
        let tracer = SpanRecordingTracer::new();
        let rt = reserving_runtime_with_tracer(store, tracer.clone());

        let mut ctx = ServiceContext::new().with_operation_key(scope_test_key());
        ctx.set_resolved_tenant(CanonicalTenant::scoped(tenant));

        rt.inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("the reservation still proceeds without tracing");

        assert!(
            tracer.started().is_empty(),
            "no trace context, no span: got {:?}",
            tracer.started()
        );
    }

    /// A dispatch refused before the store is reached opens no span either.
    ///
    /// The span describes a reserve *attempt*. An unresolved scope never attempts
    /// one — it is refused before the store is asked — so a span here would report
    /// a durable write that never happened, and its duration would be noise.
    #[tokio::test]
    async fn a_dispatch_refused_before_the_store_opens_no_span() {
        let store = ScopeRecordingStore::answering(ReservationOutcome::Fresh(lease_for(None)));
        let tracer = SpanRecordingTracer::new();
        let rt = reserving_runtime_with_tracer(store, tracer.clone());

        let mut ctx = ServiceContext::new()
            .with_operation_key(scope_test_key())
            .with_trace_context(ego_domain::TraceContext::root());

        let result = rt
            .inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await;

        assert!(result.is_err(), "an unresolved scope is refused");
        assert!(
            tracer.started().is_empty(),
            "nothing was attempted, so nothing may be reported as attempted: {:?}",
            tracer.started()
        );
    }

    fn scope_test_key() -> OperationKey {
        OperationKey::parse("scope-under-test").expect("a non-empty key parses")
    }

    fn lease_for(tenant: Option<TenantId>) -> Lease {
        Lease {
            operation_id: OperationId::new(tenant, scope_test_key()),
            owner_id: OwnerId::new("in-crate-owner"),
            fencing_token: FencingToken::initial(),
            lease_until: FixedClock.now() + chrono::Duration::seconds(30),
        }
    }

    /// A context that resolved no scope is refused, and the store is never asked.
    ///
    /// The reserve count is the assertion that matters. An implementation that
    /// returned the right error *after* reserving would satisfy an
    /// error-only check while having already taken the lease under a namespace it
    /// was not entitled to — and the next caller in that namespace would then
    /// contend with it.
    ///
    /// Reverting the `match` to `and_then` makes this fail on the recorded scope:
    /// the store is reached, and it is handed `None`.
    #[tokio::test]
    async fn an_unresolved_scope_is_refused_rather_than_filed_as_systemwide() {
        let store = ScopeRecordingStore::answering(ReservationOutcome::Fresh(lease_for(None)));
        let rt = reserving_runtime(store.clone());
        let mut ctx = ServiceContext::new().with_operation_key(scope_test_key());
        assert!(
            ctx.canonical_tenant().is_none(),
            "the precondition: nothing resolved a scope on this path"
        );

        let result = rt
            .inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await;

        match result {
            Err(rejection) => assert_eq!(
                rejection,
                ReservationRejection::TenantUnresolved,
                "an unresolved scope names no namespace, so it cannot be reserved"
            ),
            Ok(other) => panic!("expected a refusal, got {other:?}"),
        }
        assert!(
            store.scopes().is_empty(),
            "the store must never be reached: reserving first and refusing after \
             would leave the lease taken, got {:?}",
            store.scopes()
        );
        assert!(
            ctx.operation_fingerprint().is_none(),
            "a refused dispatch must not carry a fingerprint into the receipt gate"
        );
    }

    /// A scope that resolved *to* systemwide still reserves, under the absent
    /// namespace.
    ///
    /// This is the case the refusal above must not have swallowed. Systemwide is a
    /// legitimate resolution whose namespace is `None`; what is refused is the
    /// absence of a resolution, which is a different thing wearing the same
    /// `Option` shape. Without this test the fix would be indistinguishable from
    /// "reject every tenant-less reservation", which would break the mode
    /// outright.
    #[tokio::test]
    async fn a_resolved_systemwide_scope_reserves_under_the_absent_namespace() {
        use crate::runtime::CanonicalTenant;

        let store = ScopeRecordingStore::answering(ReservationOutcome::Fresh(lease_for(None)));
        let rt = reserving_runtime(store.clone());
        let mut ctx = ServiceContext::new().with_operation_key(scope_test_key());
        ctx.set_resolved_tenant(CanonicalTenant::systemwide());

        let decision = rt
            .inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("a resolved systemwide scope is reservable");

        assert!(
            matches!(decision, Some(ReservationDecision::Proceed(_))),
            "got {decision:?}"
        );
        assert_eq!(
            store.scopes(),
            vec![None],
            "the systemwide scope's namespace is the absent one — reached, not refused"
        );
    }

    /// And a completed operation in the systemwide scope still replays, within
    /// that scope.
    ///
    /// Isolation that cost the systemwide mode its replay would be a regression
    /// dressed as a fix, so the read path is pinned alongside the write path.
    #[tokio::test]
    async fn a_resolved_systemwide_scope_replays_within_its_own_namespace() {
        use crate::runtime::CanonicalTenant;

        let stored = StoredServiceResponse::new(b"systemwide-answer".to_vec());
        let store = ScopeRecordingStore::answering(ReservationOutcome::Succeeded(stored.clone()));
        let rt = reserving_runtime(store.clone());
        let mut ctx = ServiceContext::new().with_operation_key(scope_test_key());
        ctx.set_resolved_tenant(CanonicalTenant::systemwide());

        let decision = rt
            .inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("a completed systemwide operation answers");

        match decision {
            Some(ReservationDecision::Replay(response)) => assert_eq!(
                response, stored,
                "the replay returns the systemwide scope's own stored response"
            ),
            other => panic!("expected a replay, got {other:?}"),
        }
        assert_eq!(
            store.scopes(),
            vec![None],
            "the lookup was made in the absent namespace, which is where it was stored"
        );
    }

    /// The scope carried to the store is the resolved tenant when there is one.
    ///
    /// The positive counterpart of the refusal: three states, and this is the one
    /// that names a concrete partition.
    #[tokio::test]
    async fn a_resolved_tenant_scope_reserves_under_that_tenant() {
        use crate::runtime::CanonicalTenant;

        let tenant = TenantId::new("tenant-a").expect("valid tenant");
        let store = ScopeRecordingStore::answering(ReservationOutcome::Fresh(lease_for(Some(
            tenant.clone(),
        ))));
        let rt = reserving_runtime(store.clone());
        let mut ctx = ServiceContext::new().with_operation_key(scope_test_key());
        ctx.set_resolved_tenant(CanonicalTenant::scoped(tenant.clone()));

        rt.inner()
            .reserve_idempotent_operation(&mut ctx, OperationFingerprint::new("fp"))
            .await
            .expect("a resolved tenant scope is reservable");

        assert_eq!(
            store.scopes(),
            vec![Some(tenant)],
            "the namespace is the resolved tenant, never the absent partition"
        );
    }
}
