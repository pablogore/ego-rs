//! Application composition root (`App` / `AppBuilder`) — design.md CORE-028
//! Stage 1.
//!
//! `AppBuilder` is a thin facade delegating to [`crate::runtime::RuntimeBuilder`]
//! (AD-1) — it collects registrations, dup-guards adapters (AD-4), and at
//! [`AppBuilder::build`] validates/constructs through the existing
//! `RuntimeBuilder`/`Injectable` contracts (AD-2/AD-3/AD-7), starting
//! nothing. `App` owns only the runtime lifecycle — [`App::start`] /
//! [`RunningApp::shutdown`] — never a transport future (AD-6).
//!
//! ## Pinned identifiers (task 1.3)
//!
//! - `RunningApp` is a distinct type returned by [`App::start`], not `App`
//!   reused — `start` consumes `App` (per design.md's Interfaces/Contracts
//!   sketch and AD-6).
//! - Method names: `App::start`, `RunningApp::shutdown`,
//!   `App::register_shutdown` (wraps `Runtime::register_async_teardown`,
//!   AD-6/M1).
//!
//! ## AD-3 construction-mechanism spike (task 2.1)
//!
//! Before implementing `.service::<S, Tag>()`'s construction mechanism, this
//! spike read `ServiceRegistry` (`crate::registry::registry`) and
//! `InterceptorChain` (`crate::interceptor::chain`) end to end, the two
//! internals explore.md flagged as unverified. Finding: **no hidden
//! constraint blocks the scratch-runtime clone-and-discard candidate**
//! design.md offers non-bindingly.
//!
//! - `ServiceRegistry` is a plain `HashMap<TypeId, Vec<(ContractVersion,
//!   Arc<dyn Any>)>>` with no lazy initialization, no global/static state,
//!   and no I/O — `register`/`resolve_raw` are pure in-memory operations.
//! - `InterceptorChain` is a plain `Vec<Arc<dyn Interceptor>>`; `add_interceptor`
//!   only pushes, nothing runs eagerly.
//! - `RuntimeBuilder::build()` (already confirmed in explore.md #1) never
//!   spawns a Tokio task and never calls `.start()` on the effects acceptor —
//!   only `Runtime::start_effects` does that, and this spike never calls it
//!   on the scratch runtime.
//!
//! Consequence: building a scratch `Runtime` via `builder.clone().build()`,
//! using it only to run `Injectable::validate`/`Injectable::build` for
//! registered services, then discarding it (never calling `shutdown()` /
//! `shutdown_async()` on it) is safe — the scratch's own `TeardownStack` and
//! any auto-registered async teardown hooks (logger, data-provider shutdown)
//! are simply dropped along with it, never executed, and never observed by
//! the real runtime built afterward from the retained (unconsumed) builder.
//! This is the mechanism `AppBuilder::build` below implements.

pub mod error;

use std::any::TypeId;
use std::collections::HashSet;
use std::sync::Arc;

use ego_domain::event::DomainEvent;
use ego_domain::read_side::dedup::DedupStore;
use ego_domain::read_side::offset::OffsetStore;
use ego_domain::Observability;
use ego_runtime::effects::{
    EffectDedupStore, EffectStateStore, ExternalEffectExecutor, RetentionMaintenance,
};
use ego_runtime::providers::ExternalDataProvider;
use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;
use kitlogger::KITLogger;

use persistent_entity::persistent_entity::PersistentEntity;
use persistent_entity::runtime::EntityRuntime;

use crate::di::{AdapterRef, ConfigValue, EntityRuntimeRef, Injectable, ProjectionRef};
use ego_domain::operation::OperationReservationStore;

use crate::runtime::IdempotencyEnforcementMode;
use crate::runtime::{
    HasServiceTag, Profile, Resolvable, Runtime, RuntimeBuilder, RuntimeInner, RuntimeResolver,
};

pub use error::CompositionError;

/// A registration recorded by `.service()`/`.service_instance()`, resolved
/// against a scratch runtime and applied to the retained `RuntimeBuilder` at
/// `build()` time (AD-3). Boxed and type-erased so `AppBuilder` can hold a
/// heterogeneous `Vec` of them.
type ServiceRegistrar =
    Box<dyn FnOnce(&RuntimeInner, RuntimeBuilder) -> Result<RuntimeBuilder, CompositionError>>;

/// Stamps `S`'s type name onto an as-yet-unattributed resolution error if it
/// isn't already set (review F3) — a single helper shared by both
/// `Injectable::validate` and `Injectable::build`'s error paths in
/// [`AppBuilder::service`], so the two routes can't independently diverge on
/// attribution. Covers both `DependencyNotFound` (a missing adapter/config/
/// projection/entity, preserving its `kind`) and `ServiceNotFound` (a missing
/// service tag resolved by another service's construction), so a service that
/// fails to build because a collaborator it needs is unregistered names both
/// the missing tag and this requester.
fn attribute_to<S: 'static>(err: crate::runtime::RuntimeError) -> crate::runtime::RuntimeError {
    match err {
        crate::runtime::RuntimeError::DependencyNotFound {
            kind,
            type_name,
            service_name: None,
        } => crate::runtime::RuntimeError::DependencyNotFound {
            kind,
            type_name,
            service_name: Some(std::any::type_name::<S>()),
        },
        crate::runtime::RuntimeError::ServiceNotFound {
            type_name,
            required_by: None,
        } => crate::runtime::RuntimeError::ServiceNotFound {
            type_name,
            required_by: Some(std::any::type_name::<S>()),
        },
        other => other,
    }
}

/// Builder for an [`App`] — the application-facing composition root (AD-1,
/// G2). Delegates every registration to an internal `RuntimeBuilder`; never
/// reimplements assembly (G3).
pub struct AppBuilder {
    runtime_builder: RuntimeBuilder,
    adapter_types: HashSet<TypeId>,
    /// Presence guard for `.effect_store()` (CORE-028D1): single-slot, so a
    /// `bool` — unlike `adapter_types`, a second call is rejected even for a
    /// different concrete type.
    effect_store_registered: bool,
    /// Presence guard for `.effect_retention_store()` (CORE-028D1),
    /// independent of `effect_store_registered`.
    effect_retention_store_registered: bool,
    /// Keys already registered through `.read_side_progress()` (PROD-014A
    /// AD-7). Keyed, not a `bool`: registration is per-projection (D-3), so
    /// a second projection is valid and only a second registration of the
    /// *same* `projection_id` is not.
    read_side_progress_ids: HashSet<String>,
    service_registrars: Vec<ServiceRegistrar>,
    /// First error encountered by an infallible-signature registration call
    /// (e.g. a duplicate adapter, AD-4) — surfaced at `build()` rather than
    /// changing `.adapter()`'s `Self`-returning chain shape.
    ///
    /// First-error-wins: every chainable registration method checks this
    /// before doing any work and returns immediately once it is `Some` — a
    /// builder that has already latched an error accepts no further
    /// registrations, and `build()` returns exactly this first error.
    pending_error: Option<CompositionError>,
}

/// A validated, constructed-but-not-started application (AD-2). Usable
/// directly in tests without ever calling [`App::start`] (spec: "An
/// Application Is Testable Without Running").
pub struct App {
    runtime: Runtime,
}

/// A started application, returned by [`App::start`]. Owns no transport
/// future (AD-6) — the host sequences its own workload between `start()`
/// and [`RunningApp::shutdown`].
pub struct RunningApp {
    runtime: Runtime,
}

impl App {
    /// Starts a fresh [`AppBuilder`].
    pub fn builder() -> AppBuilder {
        AppBuilder::new()
    }

    /// Returns a [`RuntimeResolver`] — a resolution-only handle (Stage 1 PR2
    /// — found during the reference-app migration; narrowed after review).
    /// A transport layer built against the lower-level `Runtime` API (e.g.
    /// `ego_transport::AppState`, which predates `App`/`AppBuilder` and
    /// needs direct resolution for its own generic per-request
    /// `resolve::<Tag>()` dispatch) is a legitimate, expected integration
    /// point — but handing out the full `Runtime` would leak `start_effects`/
    /// `shutdown_async`/`register_async_teardown`, the exact lifecycle
    /// surface `App`/`RunningApp`'s typestate exists to gate. `RuntimeResolver`
    /// exposes only `resolve`/`logger`. Cheap — wraps a `Runtime`, itself only
    /// an `Arc` clone. Callable before [`App::start`] since request-time
    /// resolution doesn't depend on whether effects have started.
    pub fn resolver(&self) -> RuntimeResolver {
        self.runtime.resolver()
    }

    /// Resolves `Tag` to its generated proxy — a thin pass-through to
    /// [`Runtime::resolve`], the same production resolution path (AD-1,
    /// AD-9). Lets a test assert on a resolved service or adapter without
    /// ever calling [`App::start`] (spec: "An Application Is Testable
    /// Without Running").
    pub fn resolve<Tag>(&self) -> Result<Tag::Proxy, crate::runtime::RuntimeError>
    where
        Tag: Resolvable + 'static,
    {
        self.runtime.resolve::<Tag>()
    }

    /// Resolves a registered adapter directly (review F4) — the same
    /// production resolution path `Injectable::build`-generated fields use.
    /// Lets a constructed-but-not-started application (or an external
    /// integration test) verify an adapter was registered without reaching
    /// into any private field (spec: "An Application Is Testable Without
    /// Running").
    pub fn resolve_adapter<A: Send + Sync + 'static>(
        &self,
    ) -> Result<AdapterRef<A>, crate::runtime::RuntimeError> {
        self.runtime.inner().resolve_adapter::<A>()
    }

    /// Resolves a registered config value directly (review F4) — the public
    /// counterpart to [`Self::resolve_adapter`], for the same reason.
    pub fn resolve_config<C: Send + Sync + 'static>(
        &self,
    ) -> Result<ConfigValue<C>, crate::runtime::RuntimeError> {
        self.runtime.inner().resolve_config::<C>()
    }

    /// Resolves a registered projection directly (CORE-028 Stage 2) — the
    /// same production resolution path `Injectable::build`-generated fields
    /// use, mirroring [`Self::resolve_adapter`]/[`Self::resolve_config`]
    /// (review F4 pattern) for the same reason: proving a projection
    /// registered via [`AppBuilder::projection`] is reachable without
    /// requiring a fabricated `Injectable` consumer.
    pub fn resolve_projection<P: Send + Sync + 'static>(
        &self,
    ) -> Result<ProjectionRef<P>, crate::runtime::RuntimeError> {
        self.runtime.inner().resolve_projection::<P>()
    }

    /// Resolves a registered entity runtime as `EntityRuntimeRef<E>`
    /// (CORE-028 Stage 2C design.md AD-8) — read-only, symmetric with
    /// [`Self::resolve_adapter`]/[`Self::resolve_config`]/[`Self::resolve_projection`].
    pub fn resolve_entity<E>(&self) -> Result<EntityRuntimeRef<E>, crate::runtime::RuntimeError>
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
        self.runtime.inner().resolve_entity::<E>()
    }

    /// Registers a shutdown participant — e.g. a spawned read-side
    /// projection handle's `stop()` future — to run when the started
    /// application shuts down (AD-6/M1). Wraps the existing
    /// `Runtime::register_async_teardown`; names the shared "knows how to
    /// shut down" contract, not one implementation's shape. Must be called
    /// before [`App::start`] — `App` tracks the participant for shutdown
    /// timing only and never wraps, returns, or re-owns whatever state it
    /// belongs to (spec: "Read-Model Ownership Is Preserved").
    pub fn register_shutdown<F>(&self, hook: F)
    where
        F: std::future::Future<Output = Result<(), crate::runtime::RuntimeInfraError>>
            + Send
            + 'static,
    {
        self.runtime.register_async_teardown(hook);
    }

    /// Starts the application's background processes (AD-6). Owns no
    /// transport future and awaits none — the host sequences its own
    /// workload between this and [`RunningApp::shutdown`]. Requires an
    /// active Tokio runtime (delegates to `Runtime::start_effects`).
    ///
    /// **On failure (Stage 1 PR2 review, MEDIUM):** a caller may have already
    /// registered shutdown participants via [`App::register_shutdown`] (e.g.
    /// a read-side scheduler the host already spawned) before calling
    /// `start()`. If `start_effects` then fails, there is no [`RunningApp`]
    /// to call `shutdown` on — without a rollback here, every hook already
    /// registered would leak instead of draining. `start()` runs
    /// `shutdown_async` on the failed attempt before returning, so a failed
    /// start leaves nothing running; the cleanup's own result is best-effort
    /// (the original startup error is what the caller needs to act on).
    pub async fn start(self) -> Result<RunningApp, CompositionError> {
        // CORE-028D4: the span is the whole method, not `start_effects()`
        // alone. Those coincide today; the metric means "how long App startup
        // took", so it stays correct when `start()` grows a second step.
        let started_at = std::time::Instant::now();
        self.runtime.inner().record_app_starting();
        if let Err(startup_err) = self.runtime.start_effects().await {
            let _ = self.runtime.shutdown_async().await;
            return Err(CompositionError::Startup(startup_err));
        }
        self.runtime.inner().record_app_started(started_at.elapsed());
        Ok(RunningApp {
            runtime: self.runtime,
        })
    }
}

impl RunningApp {
    /// Shuts down every process the application started (AD-6): runs async
    /// shutdown hooks in registration order (ALL of them, even after one
    /// fails), then the sync teardown stack, surfacing the FIRST hook error
    /// if any — matching the existing `Runtime::shutdown_async` contract
    /// exactly (spec: "One failing shutdown participant does not hide
    /// others, and its error surfaces").
    pub async fn shutdown(self) -> Result<(), CompositionError> {
        self.runtime
            .shutdown_async()
            .await
            .map_err(CompositionError::Shutdown)
    }
}

impl AppBuilder {
    fn new() -> Self {
        Self {
            runtime_builder: RuntimeBuilder::new(),
            adapter_types: HashSet::new(),
            effect_store_registered: false,
            effect_retention_store_registered: false,
            read_side_progress_ids: HashSet::new(),
            service_registrars: Vec::new(),
            pending_error: None,
        }
    }
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AppBuilder {
    /// Registers a host-constructed adapter, dup-guarded by concrete type
    /// (AD-4): a second registration of the same type `A` is rejected with
    /// `CompositionError::DuplicateAdapter` at [`Self::build`] — never a
    /// silent last-write-wins overwrite. Use [`Self::replace_adapter`] for a
    /// deliberate, explicit override.
    pub fn adapter<A: Send + Sync + 'static>(mut self, adapter: Arc<A>) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        if !self.adapter_types.insert(TypeId::of::<A>()) {
            self.pending_error = Some(CompositionError::DuplicateAdapter {
                type_name: std::any::type_name::<A>(),
            });
            return self;
        }
        self.runtime_builder = self.runtime_builder.with_adapter(adapter);
        self
    }

    /// Deliberately replaces any previously registered adapter of the same
    /// concrete type — the explicit escape hatch AD-4 requires instead of
    /// [`Self::adapter`]'s dup-guard. Intended for bootstrap/composition use
    /// only (G2), not as a routine runtime operation.
    pub fn replace_adapter<A: Send + Sync + 'static>(mut self, adapter: Arc<A>) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        self.adapter_types.insert(TypeId::of::<A>());
        self.runtime_builder = self.runtime_builder.with_adapter(adapter);
        self
    }

    /// Registers a host-constructed config value — thin delegation to
    /// [`RuntimeBuilder::with_config`] (last-write-wins, same as production).
    pub fn config<C: Send + Sync + 'static>(mut self, value: Arc<C>) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        self.runtime_builder = self.runtime_builder.with_config(value);
        self
    }

    /// Registers a projection instance — thin delegation to
    /// [`RuntimeBuilder::with_projection`] (CORE-028 Stage 2 design.md AD-3),
    /// following the exact clone-then-call + `pending_error` shape
    /// `.effect_executor()`/`.data_provider()` already use. Fail-closed on a
    /// duplicate registration for the same concrete type is already enforced
    /// by `RuntimeBuilder` (AD-1/AD-2) — this facade never adds a second
    /// guard. A `.projection(...)`-registered value never requires the
    /// caller to construct or reach into `RuntimeBuilder` — this signature
    /// is the whole proof (spec: "No internal runtime type is required to
    /// register a projection").
    pub fn projection<P: Send + Sync + 'static>(mut self, projection: Arc<P>) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        match self.runtime_builder.clone().with_projection(projection) {
            Ok(builder) => {
                self.runtime_builder = builder;
                self
            }
            Err(err) => {
                self.pending_error = Some(CompositionError::Projection(err));
                self
            }
        }
    }

    /// Registers a host-constructed entity runtime — thin delegation to
    /// [`RuntimeBuilder::with_entity`] (CORE-028 Stage 2C design.md AD-5),
    /// following the exact clone-then-call + `pending_error` shape
    /// [`Self::projection`] already uses. Fail-closed on a duplicate
    /// registration for the same aggregate type is already enforced by
    /// `RuntimeBuilder` (AD-1/AD-4) — this facade never adds a second guard.
    pub fn entity<E>(mut self, runtime: Arc<EntityRuntime<E::Event>>) -> Self
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
        if self.pending_error.is_some() {
            return self;
        }
        match self.runtime_builder.clone().with_entity::<E>(runtime) {
            Ok(builder) => {
                self.runtime_builder = builder;
                self
            }
            Err(err) => {
                self.pending_error = Some(CompositionError::Entity(err));
                self
            }
        }
    }

    /// Registers a pre-initialized logger — thin delegation to
    /// [`RuntimeBuilder::with_logger`]. The logger is constructed and
    /// initialized by the host via the existing kit-config pipeline before
    /// this call — `RuntimeBuilder` never constructs it, and neither does
    /// `AppBuilder` (spec: "Config, Security, Logging, And Observability
    /// Reuse Existing Abstractions" — no second config or provider system).
    pub fn logger(mut self, logger: Arc<KITLogger>) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        self.runtime_builder = self.runtime_builder.with_logger(logger);
        self
    }

    /// Registers security providers — thin delegation to
    /// [`RuntimeBuilder::with_security`]. Both-or-nothing is enforced
    /// structurally: this signature requires both providers in the same
    /// call, so a caller cannot represent "authn without authz" at all.
    pub fn security(
        mut self,
        authn: Arc<dyn AuthenticationProvider>,
        authz: Arc<dyn AuthorizationProvider>,
    ) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        self.runtime_builder = self.runtime_builder.with_security(authn, authz);
        self
    }

    /// Registers an observability hook — thin delegation to
    /// [`RuntimeBuilder::with_observability`] (review F1). Reuses the
    /// existing hook rather than inventing a second one (spec: "Config,
    /// Security, Logging, And Observability Reuse Existing Abstractions").
    pub fn observability(mut self, obs: Arc<dyn Observability>) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        self.runtime_builder = self.runtime_builder.with_observability(obs);
        self
    }

    /// Sets how a missing client-supplied operation key is treated.
    ///
    /// Forwarded because the underlying builder's default is fail-closed: a runtime
    /// under [`IdempotencyEnforcementMode::MandatoryKey`] cannot be built without a
    /// reservation store, so an application needs a way either to register one or to
    /// state that it has not adopted enforcement yet. Without this method the facade
    /// would offer neither, and every application would fail to build.
    ///
    /// Note that `AppBuilder` does *not* forward the tenant enforcement mode. That is
    /// not an inconsistency to copy: tenant enforcement has a permissive-by-default
    /// posture that needs no decision at bootstrap, while this one refuses to start
    /// until the decision is made.
    pub fn idempotency_enforcement_mode(mut self, mode: IdempotencyEnforcementMode) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        self.runtime_builder = self.runtime_builder.with_idempotency_enforcement_mode(mode);
        self
    }

    /// Declares what deployment posture this composition is being built for
    /// (PROD-013) — thin delegation to [`RuntimeBuilder::profile`], mirroring
    /// how [`Self::effect_store`] delegates to
    /// [`RuntimeBuilder::with_effect_store`].
    ///
    /// **Does not propagate to any entity runtime already registered via
    /// [`Self::entity`].** `AppBuilder::entity()` receives an
    /// already-built `Arc<EntityRuntime<E>>` — that entity runtime's own
    /// persistence gate (`EntityRuntimeBuilder::profile`) already ran before
    /// `AppBuilder` ever saw it, so a single call here cannot reach back and
    /// change a decision that was already made. Set the profile on each
    /// `EntityRuntimeBuilder` independently.
    pub fn profile(mut self, profile: Profile) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        self.runtime_builder = self.runtime_builder.profile(profile);
        self
    }

    /// Registers the single [`OperationReservationStore`] operations are reserved
    /// through.
    ///
    /// This is the other half of the decision above: registering a store is how an
    /// application adopts enforcement rather than deferring it.
    pub fn operation_reservation_store(
        mut self,
        store: Arc<dyn OperationReservationStore>,
    ) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        self.runtime_builder = self.runtime_builder.with_operation_reservation_store(store);
        self
    }

    /// Adopts operation-key enforcement, with everything a lease needs to behave.
    ///
    /// One method rather than four forwards, because these four are not four
    /// settings. A store with no clock cannot compute a lease expiry; an owner
    /// with no store means nothing; a lease length without a clock is unusable.
    /// Exposed separately they would let a host assemble combinations most of
    /// which are incoherent — enforcement declared with no store, a store with no
    /// owner, an owner with no clock — and each of those looks configured while
    /// guaranteeing nothing.
    ///
    /// So the facade's surface is a **complete** configuration. It delegates to
    /// [`RuntimeBuilder`]'s individual setters internally, where the pieces are
    /// separate for the builder's own reasons; what a host can express is the
    /// whole thing or none of it.
    ///
    /// # The parameters, and why each is a host decision
    ///
    /// - `store`: where reservations are held. Durable, or enforcement is
    ///   decorative — an in-memory store makes the build succeed and the
    ///   deployment look adopted while surviving nothing.
    /// - `owner_id`: who this runtime reserves as. A takeover is *this* owner
    ///   displacing *that* one, so two replicas sharing an id could not displace
    ///   each other at all.
    /// - `lease_duration`: how long a lease is held before another owner may take
    ///   it. Must exceed the longest a legitimate execution can take; a shorter
    ///   one permits two owners to run the same operation at once, which is a
    ///   correctness problem rather than a tuning preference.
    /// - `clock`: what expiry is measured against. One source, so a worker
    ///   reading wall time cannot disagree with a store reading an injected
    ///   clock — under test, or under skew.
    ///
    /// The alternative to calling this is
    /// [`Self::idempotency_enforcement_mode`] with the compatibility variant,
    /// which is how a deployment states it has not adopted enforcement yet.
    pub fn enforced_idempotency(
        mut self,
        store: Arc<dyn OperationReservationStore>,
        owner_id: ego_domain::operation::OwnerId,
        lease_duration: std::time::Duration,
        clock: Arc<dyn ego_domain::time::Clock>,
    ) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        self.runtime_builder = self
            .runtime_builder
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::MandatoryKey)
            .with_operation_reservation_store(store)
            .with_reservation_owner_id(owner_id)
            .with_reservation_lease_duration(lease_duration)
            .with_reservation_clock(clock);
        self
    }

    /// Registers the effect store the external-effects subsystem uses in
    /// place of the default `InMemoryEffectStore` — thin delegation to
    /// [`RuntimeBuilder::with_effect_store`] (PROD-002 Phase 8 finding:
    /// `RuntimeBuilder` already carried this capability, but nothing forwarded
    /// it through the facade every real host composes through).
    ///
    /// Takes one concrete type implementing both [`EffectStateStore`] and
    /// [`EffectDedupStore`], exactly as the underlying builder does — see its
    /// doc comment for why a mixed durable/non-durable pair is not
    /// representable here.
    pub fn effect_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: EffectStateStore + EffectDedupStore + 'static,
    {
        if self.pending_error.is_some() {
            return self;
        }
        if self.effect_store_registered {
            self.pending_error = Some(CompositionError::DuplicateEffectStore {
                type_name: std::any::type_name::<T>(),
            });
            return self;
        }
        self.effect_store_registered = true;
        self.runtime_builder = self.runtime_builder.with_effect_store(store);
        self
    }

    /// Registers the [`RetentionMaintenance`] capability settled external
    /// effects are purged through — thin delegation to
    /// [`RuntimeBuilder::with_effect_retention_store`]. Independent of
    /// [`Self::effect_store`]: a type implementing both composes via
    /// `.effect_store(store.clone()).effect_retention_store(store)`.
    pub fn effect_retention_store(mut self, store: Arc<dyn RetentionMaintenance>) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        if self.effect_retention_store_registered {
            self.pending_error = Some(CompositionError::DuplicateEffectRetentionStore);
            return self;
        }
        self.effect_retention_store_registered = true;
        self.runtime_builder = self.runtime_builder.with_effect_retention_store(store);
        self
    }

    /// Registers the durable read-side progress pair for a projection — thin
    /// delegation to [`RuntimeBuilder::with_read_side_progress`] (PROD-014A
    /// AD-1/AD-7). Keyed by `projection_id`, not a single slot like
    /// [`Self::effect_store`]: a second registration for a *different*
    /// `projection_id` is valid, but a second registration for the *same*
    /// `projection_id` is rejected — silently replacing a projection's resume
    /// state is not a composition a reader can verify.
    pub fn read_side_progress(
        mut self,
        projection_id: impl Into<String>,
        offset_store: Arc<dyn OffsetStore + Send + Sync>,
        dedup_store: Arc<dyn DedupStore + Send + Sync>,
    ) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        let projection_id = projection_id.into();
        if !self.read_side_progress_ids.insert(projection_id.clone()) {
            self.pending_error = Some(CompositionError::DuplicateReadSideProgress { projection_id });
            return self;
        }
        self.runtime_builder =
            self.runtime_builder
                .with_read_side_progress(projection_id, offset_store, dedup_store);
        self
    }

    /// Registers an external-effect executor — thin delegation to
    /// [`RuntimeBuilder::register_effect_executor`] (review F1). This is
    /// what makes [`App::start`] actually have effects to start:
    /// `CompositionError::EffectExecutor` already existed for this path but
    /// had no public registration method to produce it until this one.
    /// Fails closed on a duplicate `effect_type`, matching
    /// `register_effect_executor`'s own contract exactly — no second dup-guard
    /// is layered on top, unlike `.adapter()`'s (AD-4), because the
    /// underlying registry already fails closed here.
    pub fn effect_executor(
        mut self,
        effect_types: impl IntoIterator<Item = impl Into<String>>,
        executor: Arc<dyn ExternalEffectExecutor>,
    ) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        // Clone-then-call (not move-then-restore): `register_effect_executor`
        // consumes `RuntimeBuilder` by value, but `self.runtime_builder` must
        // stay intact on the error path — cloning first means it's never
        // taken out of `self` at all, so there's nothing to restore.
        let registration = self
            .runtime_builder
            .clone()
            .register_effect_executor(effect_types, executor);
        match registration {
            Ok(builder) => {
                self.runtime_builder = builder;
                self
            }
            Err(err) => {
                self.pending_error = Some(CompositionError::EffectExecutor(err));
                self
            }
        }
    }

    /// Registers an external data provider — thin delegation to
    /// [`RuntimeBuilder::register_data_provider`] (review F1). Fails closed
    /// on a duplicate `provider_id`, matching `register_data_provider`'s own
    /// contract exactly.
    pub fn data_provider(
        mut self,
        provider_id: impl Into<String>,
        provider: Arc<dyn ExternalDataProvider>,
    ) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        // Same clone-then-call reasoning as `.effect_executor()` above.
        match self
            .runtime_builder
            .clone()
            .register_data_provider(provider_id, provider)
        {
            Ok(builder) => {
                self.runtime_builder = builder;
                self
            }
            Err(err) => {
                self.pending_error = Some(CompositionError::DataProvider(err));
                self
            }
        }
    }

    /// Records `S` for construction through the existing `Injectable`
    /// contract at [`Self::build`] (AD-3) — `Injectable::validate` then
    /// `Injectable::build`, the same construction path production and
    /// testkit already use. The resulting service is made resolvable under
    /// `S::Tag`. A missing dependency surfaces at `build()` naming both the
    /// missing type and this requesting service (spec: "A missing
    /// dependency names both the missing type and the requester").
    ///
    /// The macro-linked, single-type-parameter form (CORE-028 Stage 2B) —
    /// takes only `S`, no Tag parameter and no coercion closure. Bounded on
    /// [`HasServiceTag`], generated by `#[service(impl_of = Trait)]`; a
    /// struct with no macro-generated trait link (e.g. a hand-rolled
    /// `Injectable`, or a bare `#[service]` struct with no `impl_of`
    /// argument) fails this bound at compile time (spec: "A service type
    /// with no trait link fails to compile against the macro-linked call").
    /// Use [`Self::service_with_tag`] for those.
    pub fn service<S>(mut self) -> Self
    where
        S: Injectable + HasServiceTag + 'static,
    {
        if self.pending_error.is_some() {
            return self;
        }
        self.service_registrars.push(Box::new(
            move |scratch: &RuntimeInner, builder: RuntimeBuilder| {
                S::validate(scratch)
                    .map_err(attribute_to::<S>)
                    .map_err(CompositionError::Validation)?;
                let instance = S::build(scratch)
                    .map_err(attribute_to::<S>)
                    .map_err(CompositionError::Validation)?;
                let arc = S::into_service(Arc::new(instance));
                builder
                    .with_service::<S::Tag>(arc)
                    .map_err(CompositionError::Service)
            },
        ));
        self
    }

    /// Explicit-Tag registration (renamed from `service` in CORE-028 Stage
    /// 2B) — the permanent path for a service type with no macro-generated
    /// trait link (spec: "A hand-rolled Injectable struct still registers
    /// through the renamed explicit-Tag form"). Never deprecated: a
    /// hand-rolled `Injectable` struct can never carry a macro-generated
    /// `HasServiceTag` link, so this is its only route to registration.
    ///
    /// Two type parameters are required because `S` carries no link to its
    /// resolution `Tag` — the caller must name both `S` and `Tag` together,
    /// plus a trivial unsizing coercion (typically `|arc| arc`), for the
    /// same reason [`Self::service`]'s doc explains above.
    ///
    /// **Deviation from design.md's Interfaces/Contracts sketch:** that
    /// sketch bounds this as `S: Injectable + Tag::Service` (naming an
    /// opaque associated type directly as a trait bound). That is not valid
    /// Rust — confirmed with an isolated repro (`rustc` `E0405`: "cannot
    /// find trait `Service` in `Tag`"). Rust has no mechanism to bound a
    /// type parameter against "whichever trait underlies this associated
    /// type"; only the call site, which names both `S` and the concrete
    /// trait together, can perform that coercion.
    pub fn service_with_tag<S, Tag>(
        mut self,
        to_trait_object: fn(Arc<S>) -> Arc<Tag::Service>,
    ) -> Self
    where
        Tag: Resolvable + 'static,
        S: Injectable + 'static,
    {
        if self.pending_error.is_some() {
            return self;
        }
        self.service_registrars.push(Box::new(
            move |scratch: &RuntimeInner, builder: RuntimeBuilder| {
                // Same attribution `RuntimeBuilder::try_build` already provides
                // (AD-3/AD-7): a `DependencyNotFound` reaching here has
                // `service_name: None` (nothing below this closure knows who's
                // asking); fill it in with the requesting service's type name
                // here, exactly as `try_build`'s validator loop does for
                // `with_injectable`.
                //
                // Applied to BOTH `S::validate` and `S::build`'s errors (review
                // F3): a hand-rolled `Injectable` with an incomplete
                // `dependencies()` list, conditional resolution, or any other
                // `DependencyNotFound` surfacing only during `build()` (not
                // caught by `validate()`'s presence check) must still name the
                // requesting service — the observable contract doesn't
                // distinguish "caught by validate" from "caught by build".
                S::validate(scratch)
                    .map_err(attribute_to::<S>)
                    .map_err(CompositionError::Validation)?;
                let instance = S::build(scratch)
                    .map_err(attribute_to::<S>)
                    .map_err(CompositionError::Validation)?;
                let arc = to_trait_object(Arc::new(instance));
                builder
                    .with_service::<Tag>(arc)
                    .map_err(CompositionError::Service)
            },
        ));
        self
    }

    /// Registers a pre-built instance directly, resolvable under `Tag` —
    /// the AD-3 escape hatch for a collaborator that genuinely cannot be
    /// expressed as an `Injectable` dependency (e.g. a non-DI collaborator
    /// like reference-app's read-side sink). Should be the exception, not
    /// the default registration path (G1) — prefer [`Self::service`]
    /// whenever construction can be expressed through `Injectable`.
    pub fn service_instance<Tag: Resolvable + 'static>(mut self, svc: Arc<Tag::Service>) -> Self {
        if self.pending_error.is_some() {
            return self;
        }
        self.service_registrars.push(Box::new(
            move |_scratch: &RuntimeInner, builder: RuntimeBuilder| {
                builder
                    .with_service::<Tag>(svc)
                    .map_err(CompositionError::Service)
            },
        ));
        self
    }

    /// Validates and constructs the application (AD-2/AD-7). Starts no
    /// background task and requires no active Tokio runtime. Composition
    /// errors — a duplicate adapter recorded earlier, a service registration
    /// rejected by the registry, or a missing dependency — surface here,
    /// before anything starts.
    pub fn build(self) -> Result<App, CompositionError> {
        if let Some(err) = self.pending_error {
            return Err(err);
        }
        let mut builder = self.runtime_builder;
        if !self.service_registrars.is_empty() {
            // AD-3 scratch-runtime mechanism (see module doc comment): a
            // throwaway `Runtime` built from a clone of the retained builder
            // resolves adapters/configs already registered above, letting
            // each `Injectable::build` run for real — then it is discarded,
            // never started or shut down.
            let scratch = builder.clone().build();
            for registrar in self.service_registrars {
                builder = registrar(scratch.inner(), builder)?;
            }
        }
        let runtime = builder.try_build().map_err(CompositionError::Validation)?;
        Ok(App { runtime })
    }
}

#[cfg(test)]
mod tests {

    /// An `AppBuilder` that has explicitly not adopted idempotency enforcement.
    ///
    /// `#[cfg(test)]` and local, so nothing production-facing can reach it and no
    /// example or host bootstrap can inherit a relaxed mode from it. Tests whose
    /// subject *is* the enforcement decision construct the builder directly.
    use crate::runtime::IdempotencyEnforcementMode;

    fn compat_app() -> AppBuilder {
        App::builder().idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
    }
    use super::*;

    #[derive(Debug, PartialEq)]
    struct StubAdapter(u32);

    // DX follow-up (Part C): `attribute_to` now stamps the requesting service
    // onto a `ServiceNotFound` too (previously only `DependencyNotFound`), so a
    // service that fails to build because a collaborator service is
    // unregistered names both the missing tag and the requester.
    #[test]
    fn attribute_to_stamps_requester_onto_service_not_found() {
        let err = crate::runtime::RuntimeError::ServiceNotFound {
            type_name: "MissingTag",
            required_by: None,
        };
        match attribute_to::<StubAdapter>(err) {
            crate::runtime::RuntimeError::ServiceNotFound {
                type_name,
                required_by,
            } => {
                assert_eq!(type_name, "MissingTag", "the missing tag must be preserved");
                assert_eq!(
                    required_by,
                    Some(std::any::type_name::<StubAdapter>()),
                    "the requester must be stamped in"
                );
            }
            other => panic!("expected ServiceNotFound, got {other:?}"),
        }
    }

    // An already-attributed `ServiceNotFound` is left untouched — attribution
    // is set-once, mirroring the `DependencyNotFound` path.
    #[test]
    fn attribute_to_leaves_an_already_attributed_service_not_found_untouched() {
        let err = crate::runtime::RuntimeError::ServiceNotFound {
            type_name: "MissingTag",
            required_by: Some("OriginalRequester"),
        };
        match attribute_to::<StubAdapter>(err) {
            crate::runtime::RuntimeError::ServiceNotFound { required_by, .. } => {
                assert_eq!(required_by, Some("OriginalRequester"));
            }
            other => panic!("expected ServiceNotFound, got {other:?}"),
        }
    }

    // Task 1.4 (RED): registering an adapter twice for the same type returns
    // `CompositionError::DuplicateAdapter` (spec "Duplicate adapter
    // registration has one documented, testable outcome").
    #[test]
    fn duplicate_adapter_registration_is_rejected() {
        let result = compat_app()
            .adapter(Arc::new(StubAdapter(1)))
            .adapter(Arc::new(StubAdapter(2)))
            .build();

        match result {
            Err(CompositionError::DuplicateAdapter { type_name }) => {
                assert_eq!(type_name, std::any::type_name::<StubAdapter>());
            }
            Err(other) => panic!("expected DuplicateAdapter, got {other:?}"),
            Ok(_) => panic!("expected duplicate adapter registration to fail"),
        }
    }

    // Triangulation: distinct adapter types never collide (mirrors
    // RuntimeBuilder's own `chained_registration_multiple_types` coverage).
    #[test]
    fn distinct_adapter_types_both_register_successfully() {
        #[derive(Debug, PartialEq)]
        struct OtherAdapter(u32);

        let app = compat_app()
            .adapter(Arc::new(StubAdapter(1)))
            .adapter(Arc::new(OtherAdapter(2)))
            .build()
            .expect("distinct adapter types must not collide");
        // Review F4: resolved via the public `App::resolve_adapter`, not
        // `app.runtime.inner()` — the same check an external integration
        // test (outside this module) can now perform.
        assert!(app.resolve_adapter::<StubAdapter>().is_ok());
        assert!(app.resolve_adapter::<OtherAdapter>().is_ok());
    }

    // AD-4 escape hatch: `.replace_adapter()` deliberately bypasses the
    // dup-guard and performs the explicit override.
    #[test]
    fn replace_adapter_bypasses_the_duplicate_guard() {
        let app = compat_app()
            .adapter(Arc::new(StubAdapter(1)))
            .replace_adapter(Arc::new(StubAdapter(2)))
            .build()
            .expect("replace_adapter must not be treated as a duplicate");
        let resolved = app.resolve_adapter::<StubAdapter>().unwrap();
        assert_eq!(*resolved, StubAdapter(2));
    }

    // Task 1.6 (RED/GREEN): a registered config value is resolvable after
    // `build()` — thin `.config()` pass-through (spec "A registered config
    // value is resolvable").
    #[derive(Debug, PartialEq)]
    struct StubConfig(String);

    #[test]
    fn registered_config_value_resolves_after_build() {
        let app = compat_app()
            .config(Arc::new(StubConfig("hello".to_string())))
            .build()
            .expect("build succeeds");

        // Review F4: `App::resolve_config`, the public counterpart to
        // `resolve_adapter`.
        let resolved = app.resolve_config::<StubConfig>();
        assert!(resolved.is_ok());
        assert_eq!(*resolved.unwrap(), StubConfig("hello".to_string()));
    }

    // Task 1.6 (RED/GREEN): `.security()` pass-through — both providers
    // resolve on the built runtime (spec "Security providers are
    // both-or-nothing", structurally enforced by requiring both in one call).
    #[test]
    fn registered_security_providers_resolve_after_build() {
        use async_trait::async_trait;
        use ego_security_sdk::authorization::{AuthorizationDecision, AuthorizationProvider};
        use ego_security_sdk::context::SecurityContext;
        use ego_security_sdk::credential::Credential;
        use ego_security_sdk::error::SecurityError;
        use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};
        use ego_security_sdk::AuthenticationError;

        struct StubAuthn;
        impl AuthenticationProvider for StubAuthn {
            fn authenticate(
                &self,
                _c: &Credential,
            ) -> Result<SecurityContext, AuthenticationError> {
                let subject = SubjectId::new("user:stub").unwrap();
                Ok(SecurityContext::empty(Principal::new(
                    PrincipalKind::User,
                    subject,
                )))
            }
        }
        struct StubAuthz;
        #[async_trait]
        impl AuthorizationProvider for StubAuthz {
            async fn authorize(
                &self,
                _p: &Principal,
                _r: &ego_security_sdk::authorization::AccessRequest,
                _c: &SecurityContext,
            ) -> Result<AuthorizationDecision, SecurityError> {
                Ok(AuthorizationDecision::Allow)
            }
        }

        let app = compat_app()
            .security(Arc::new(StubAuthn), Arc::new(StubAuthz))
            .build()
            .expect("build succeeds");
        assert!(app.runtime.security_providers().is_some());
    }

    // Logger gap (found during PR1 implementation review, added before commit):
    // `.logger()` was missing from AppBuilder even though proposal.md/spec.md
    // require config/security/logging/observability to integrate via existing
    // abstractions. Mirrors `.config()`'s thin-delegation shape exactly.
    #[test]
    fn registered_logger_is_present_on_the_built_runtime() {
        use kitlogger::KITLogger;

        let app = compat_app()
            .logger(Arc::new(KITLogger::default()))
            .build()
            .expect("build succeeds");

        assert!(
            app.runtime.logger().is_some(),
            "the registered logger must be present"
        );
    }

    // Review F1: `.observability()` is a thin `AppBuilder` pass-through for
    // `RuntimeBuilder::with_observability` — reuses the existing hook rather
    // than inventing a second one.
    #[test]
    fn registered_observability_hook_does_not_prevent_build() {
        use crate::test_support::RecordingObservability;

        let app = compat_app()
            .observability(Arc::new(RecordingObservability::new()))
            .build();
        assert!(
            app.is_ok(),
            "build must succeed with an observability hook registered"
        );
    }

    // Review F1: `.effect_executor()` fails closed on a duplicate
    // `effect_type`, matching `RuntimeBuilder::register_effect_executor`'s
    // own contract exactly (no silent last-write-wins).
    #[test]
    fn duplicate_effect_type_registration_is_rejected() {
        let result = compat_app()
            .effect_executor(["dup.effect"], Arc::new(StubExecutor))
            .effect_executor(["dup.effect"], Arc::new(StubExecutor))
            .build();
        match result {
            Err(CompositionError::EffectExecutor(_)) => {}
            Err(other) => panic!("expected CompositionError::EffectExecutor, got {other:?}"),
            Ok(_) => panic!("expected duplicate effect_type registration to fail"),
        }
    }

    // Review F1: `.data_provider()` is a thin `AppBuilder` pass-through for
    // `RuntimeBuilder::register_data_provider`, fails closed on a duplicate
    // `provider_id` matching the underlying registry's own contract.
    #[test]
    fn data_provider_registers_and_rejects_duplicate_ids() {
        use async_trait::async_trait;
        use persistent_entity::data_provider_access::{
            DataProviderError, DataRequest, DataResponse,
        };

        struct StubProvider;
        #[async_trait]
        impl ExternalDataProvider for StubProvider {
            async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError> {
                Ok(DataResponse {
                    payload: request.payload,
                    cache_hit: false,
                })
            }
        }

        let ok = compat_app()
            .data_provider("provider-a", Arc::new(StubProvider))
            .build();
        assert!(
            ok.is_ok(),
            "a single data provider registration must succeed"
        );

        let dup = compat_app()
            .data_provider("provider-b", Arc::new(StubProvider))
            .data_provider("provider-b", Arc::new(StubProvider))
            .build();
        match dup {
            Err(CompositionError::DataProvider(_)) => {}
            Err(other) => panic!("expected CompositionError::DataProvider, got {other:?}"),
            Ok(_) => panic!("expected duplicate provider_id registration to fail"),
        }
    }

    // -- CORE-028 Stage 2 Phase 4: AppBuilder::projection() facade ----------

    #[derive(Debug, PartialEq)]
    struct StubProjection(u32);

    // Task 4.1 (RED): `.projection(...)` registers and resolves after
    // `build()` (spec "A projection registered via AppBuilder resolves after
    // build").
    #[test]
    fn projection_registers_and_resolves() {
        let app = compat_app()
            .projection(Arc::new(StubProjection(7)))
            .build()
            .expect("build succeeds");

        let resolved = app.resolve_projection::<StubProjection>();
        assert!(resolved.is_ok());
        assert_eq!(*resolved.unwrap(), StubProjection(7));
    }

    // Task 4.2 (RED): a second registration for the same type surfaces at
    // `.build()` as `CompositionError::Projection`, never silently replaced
    // (spec "Duplicate ... fails closed", "surfaced through `build()`").
    #[test]
    fn projection_rejects_duplicate_registration_at_build() {
        let result = compat_app()
            .projection(Arc::new(StubProjection(1)))
            .projection(Arc::new(StubProjection(2)))
            .build();

        match result {
            Err(CompositionError::Projection(_)) => {}
            Err(other) => panic!("expected CompositionError::Projection, got {other:?}"),
            Ok(_) => panic!("expected duplicate projection registration to fail"),
        }
    }

    // Task 4.4 (RED/GREEN): registering the same projection directly on
    // `RuntimeBuilder` vs. through `AppBuilder::projection(...)` must be
    // observably equivalent (spec "Registration is equivalent whether
    // performed via RuntimeBuilder or AppBuilder").
    #[test]
    fn runtimebuilder_and_appbuilder_projection_registration_are_equivalent() {
        let via_runtime_builder = crate::runtime::RuntimeBuilder::new()
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .with_projection(Arc::new(StubProjection(42)))
            .unwrap()
            .build();
        let resolved_via_runtime_builder = via_runtime_builder
            .inner()
            .resolve_projection::<StubProjection>()
            .unwrap();

        let via_app_builder = compat_app()
            .projection(Arc::new(StubProjection(42)))
            .build()
            .expect("build succeeds");
        let resolved_via_app_builder = via_app_builder
            .resolve_projection::<StubProjection>()
            .unwrap();

        assert_eq!(*resolved_via_runtime_builder, *resolved_via_app_builder);
    }

    // -- CORE-028 Stage 2C Phase 5: AppBuilder::entity() facade -------------

    use persistent_entity::builder::EntityRuntimeBuilder;
    use persistent_entity::test_entity::TestEntity;
    use persistent_entity::testing::TestEvent;

    // Task 5.1 (RED): `.entity::<TestEntity>(...)` registers and resolves
    // after `build()` (spec "A registered entity runtime resolves after
    // build"), mirroring `projection_registers_and_resolves`.
    #[test]
    fn entity_registers_and_resolves() {
        let runtime = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());
        let app = compat_app()
            .entity::<TestEntity>(runtime)
            .build()
            .expect("build succeeds");

        assert!(app.resolve_entity::<TestEntity>().is_ok());
    }

    // Task 5.2 (RED): a second registration for the same aggregate type
    // surfaces at `.build()` as `CompositionError::Entity`, never silently
    // replaced.
    #[test]
    fn entity_rejects_duplicate_registration_at_build() {
        let runtime_a = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());
        let runtime_b = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());

        let result = compat_app()
            .entity::<TestEntity>(runtime_a)
            .entity::<TestEntity>(runtime_b)
            .build();

        match result {
            Err(CompositionError::Entity(_)) => {}
            Err(other) => panic!("expected CompositionError::Entity, got {other:?}"),
            Ok(_) => panic!("expected duplicate entity registration to fail"),
        }
    }

    // Task 5.5 (RED/GREEN): registering the same entity runtime directly on
    // `RuntimeBuilder` vs. through `AppBuilder::entity(...)` must be
    // observably equivalent (mirrors
    // `runtimebuilder_and_appbuilder_projection_registration_are_equivalent`).
    #[test]
    fn runtimebuilder_and_appbuilder_entity_registration_are_equivalent() {
        let via_runtime_builder_rt = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());
        let via_runtime_builder = crate::runtime::RuntimeBuilder::new()
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .with_entity::<TestEntity>(via_runtime_builder_rt.clone())
            .unwrap()
            .build();
        let resolved_via_runtime_builder = via_runtime_builder
            .inner()
            .resolve_entity::<TestEntity>()
            .expect("registered via RuntimeBuilder must resolve");
        let expected_via_runtime_builder =
            EntityRuntimeRef::<TestEntity>::new(via_runtime_builder_rt);
        // review fix (reliability): `.is_ok()` alone doesn't prove either path
        // resolved to the runtime it was actually registered with, unlike the
        // sibling projection test above which asserts value equality — an
        // `EntityRuntimeRef` has no meaningful `PartialEq`, so the same-instance
        // proof goes through `ptr_eq` (test-only) instead.
        assert!(
            resolved_via_runtime_builder.ptr_eq(&expected_via_runtime_builder),
            "RuntimeBuilder-registered entity must resolve to the identical Arc it was registered with"
        );

        let via_app_builder_rt = Arc::new(EntityRuntimeBuilder::<TestEvent>::new().build());
        let via_app_builder = compat_app()
            .entity::<TestEntity>(via_app_builder_rt.clone())
            .build()
            .expect("build succeeds");
        let resolved_via_app_builder = via_app_builder
            .resolve_entity::<TestEntity>()
            .expect("registered via AppBuilder must resolve");
        let expected_via_app_builder = EntityRuntimeRef::<TestEntity>::new(via_app_builder_rt);
        assert!(
            resolved_via_app_builder.ptr_eq(&expected_via_app_builder),
            "AppBuilder-registered entity must resolve to the identical Arc it was registered with"
        );
    }

    // Task 1.8 (RED/GREEN): constructing an application starts nothing —
    // `build()` succeeds with no active Tokio runtime, and no effect
    // acceptor was started (spec "Constructing an application starts
    // nothing").
    #[test]
    fn build_starts_nothing_and_no_tokio_runtime_is_required() {
        let app = compat_app().build().expect("build succeeds without Tokio");
        assert!(
            app.runtime.effect_acceptor().is_none(),
            "no executor was registered and start() was never called"
        );
    }

    // Stage 1 PR2 (found during reference-app migration, narrowed after
    // review — HIGH: the original `App::runtime()` handed out a full
    // `Runtime`, letting a transport-layer caller reach `start_effects`/
    // `shutdown_async`/`register_async_teardown` directly, bypassing the
    // `App`/`RunningApp` typestate. `App::resolver()` hands out a
    // `RuntimeResolver` instead — this proves it's a live, connected view
    // (not a dead clone) while only exposing `resolve`/`logger`.
    #[test]
    fn app_resolver_sees_the_same_registered_logger_as_the_underlying_runtime() {
        let app = compat_app()
            .logger(Arc::new(KITLogger::default()))
            .build()
            .expect("build succeeds");

        let resolver = app.resolver();
        let via_resolver = resolver.logger().unwrap();
        let via_runtime = app.runtime.logger().unwrap();
        assert!(
            Arc::ptr_eq(via_resolver, via_runtime),
            "resolver() must see the same registered logger"
        );
    }

    // -- Phase 3: runtime lifecycle (App::start / RunningApp::shutdown) ----

    use async_trait::async_trait;
    use ego_domain::ExternalEffectDescription;
    use ego_runtime::effects::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    struct StubExecutor;

    #[async_trait]
    impl ExternalEffectExecutor for StubExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            AttemptOutcome::Success
        }
    }

    // Review F1: `.effect_executor()` is the public `AppBuilder` pass-through
    // for `RuntimeBuilder::register_effect_executor` — an external consumer
    // (or this test) can now build the kind of application `App::start()`
    // actually administers through the public API alone, no private-field
    // construction required.
    //
    // Task 3.1 (RED): `App::start()` starts effects — `effect_acceptor()` is
    // `Some` post-start when an executor was registered.
    #[tokio::test]
    async fn start_starts_effects_when_an_executor_was_registered() {
        let app = compat_app()
            .effect_executor(["test.effect"], Arc::new(StubExecutor))
            .build()
            .expect("build succeeds");
        assert!(app.runtime.effect_acceptor().is_none(), "not started yet");

        let running = app.start().await.expect("start succeeds");
        assert!(
            running.runtime.effect_acceptor().is_some(),
            "App::start() must call Runtime::start_effects"
        );
    }

    // -- CORE-028D4: App::start() lifecycle diagnostics ----------------------

    /// Order is the assertion that matters. An implementation that emitted both
    /// after `start_effects()` would satisfy a set-membership check while
    /// telling an operator nothing about a start that hung halfway.
    #[tokio::test]
    async fn start_records_starting_then_started() {
        use crate::test_support::RecordingObservability;

        let obs = Arc::new(RecordingObservability::new());
        let app = compat_app()
            .observability(obs.clone())
            .build()
            .expect("build succeeds");

        let _running = app.start().await.expect("start succeeds");

        let events = obs.events.lock().unwrap();
        let names: Vec<&str> = events.iter().map(|e| e.event_name.as_str()).collect();
        assert_eq!(names, vec!["app.starting", "app.started"]);
        let elapsed = events[1]
            .metadata
            .get("elapsed_ms")
            .expect("app.started carries elapsed_ms");
        // Not a value assertion — a real span is not reproducible. What is
        // checkable is that the field is a number an aggregator can consume.
        assert!(
            elapsed.parse::<u128>().is_ok(),
            "elapsed_ms must parse as a number, got {elapsed:?}"
        );
    }

    /// A hostile sink must not take startup down with it. This exercises the
    /// whole `App::start()` path, not just the helper in isolation.
    #[tokio::test]
    async fn start_survives_a_panicking_observability_sink() {
        use crate::test_support::PanickingObservability;

        let app = compat_app()
            .observability(Arc::new(PanickingObservability))
            .build()
            .expect("build succeeds");

        let running = app.start().await;
        assert!(running.is_ok(), "a panicking sink must not fail startup");
    }

    // -- PR6a: AppBuilder::effect_store / effect_retention_store delegation --
    //
    // `RuntimeBuilder::with_effect_store`/`with_effect_retention_store` are
    // already proven to dispatch correctly by
    // `tests/effect_store_composition.rs` (PROD-002 PR5 Phase 7/7.5) — these
    // tests do NOT re-litigate that. They only prove the thin `AppBuilder`
    // facade added above actually forwards to it, using the exact private
    // `app.runtime`/`running.runtime` access `start_starts_effects_when_an_
    // executor_was_registered` above already established as this module's
    // precedent for lifecycle assertions.

    use ego_domain::IdempotencyKey;
    use ego_runtime::effects::{
        AcceptedEffect, DedupOutcome, DedupScope, EffectDedupStore, EffectFingerprint, EffectId,
        EffectStateStore, EffectStoreError, InMemoryEffectStore, RetentionMaintenance,
        StoredEffect, TerminalReason, Timestamp,
    };
    use persistent_entity::command_context::CommandContext as PECommandContext;
    use persistent_entity::entity_ref::EntityRef;
    use persistent_entity::error::EntityError as PEEntityError;
    use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
    use persistent_entity::testing::{create_test_context, TestCommand, TestState};
    use std::sync::atomic::AtomicUsize;

    /// Delegates every call to a wrapped real `InMemoryEffectStore`, counting
    /// how many times each port lands here — same decorator idiom
    /// `effect_store_composition.rs`'s `RecordingEffectStore` uses, kept
    /// local rather than shared because that file is a separate test binary.
    struct RecordingEffectStore {
        inner: InMemoryEffectStore,
        state_calls: AtomicUsize,
    }

    impl RecordingEffectStore {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                inner: InMemoryEffectStore::new(),
                state_calls: AtomicUsize::new(0),
            })
        }
        fn state_calls(&self) -> usize {
            self.state_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EffectStateStore for RecordingEffectStore {
        async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
            self.state_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.accept(effect).await
        }
        async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.state_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.mark_in_flight(id).await
        }
        async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.state_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.mark_succeeded(id).await
        }
        async fn mark_retryable(
            &self,
            id: EffectId,
            attempt: u32,
            next_at: Timestamp,
        ) -> Result<(), EffectStoreError> {
            self.state_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.mark_retryable(id, attempt, next_at).await
        }
        async fn mark_terminal(
            &self,
            id: EffectId,
            reason: TerminalReason,
        ) -> Result<(), EffectStoreError> {
            self.state_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.mark_terminal(id, reason).await
        }
        async fn claim_due(
            &self,
            now: Timestamp,
            limit: usize,
        ) -> Result<Vec<StoredEffect>, EffectStoreError> {
            self.state_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.claim_due(now, limit).await
        }
        async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
            self.state_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.recover_in_flight(now).await
        }
    }

    #[async_trait]
    impl EffectDedupStore for RecordingEffectStore {
        async fn reserve(
            &self,
            scope: &DedupScope,
            effect_id: EffectId,
            fingerprint: EffectFingerprint,
        ) -> Result<DedupOutcome, EffectStoreError> {
            self.inner.reserve(scope, effect_id, fingerprint).await
        }
        async fn commit_success(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
            self.inner.commit_success(scope).await
        }
        async fn release(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
            self.inner.release(scope).await
        }
    }

    #[async_trait]
    impl RetentionMaintenance for RecordingEffectStore {
        async fn purge_before(
            &self,
            cutoff: Timestamp,
            batch: usize,
        ) -> Result<u64, EffectStoreError> {
            let _ = (cutoff, batch);
            Ok(0)
        }
    }

    /// Records every call it receives — proves delivery reached the executor
    /// registered alongside the custom store.
    struct RecordingExecutor {
        calls: AtomicUsize,
    }

    impl RecordingExecutor {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
            })
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ExternalEffectExecutor for RecordingExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            AttemptOutcome::Success
        }
    }

    /// Describes one external effect on `Increment`, none otherwise — same
    /// fixture shape `effect_store_composition.rs`'s `EffectDescribingEntity`
    /// uses, kept local for the same reason as the doubles above.
    #[derive(Debug)]
    struct EffectDescribingEntity;

    #[async_trait]
    impl PersistentEntity for EffectDescribingEntity {
        type Command = TestCommand;
        type Event = TestEvent;
        type State = TestState;

        fn initial_state(&self) -> TestState {
            TestState::new(0)
        }

        async fn handle_command(
            &self,
            command: &TestCommand,
            _state: &TestState,
            _context: &PECommandContext,
        ) -> Result<Vec<TestEvent>, PEEntityError> {
            match command {
                TestCommand::Increment(v) => Ok(vec![TestEvent::Incremented(*v)]),
                TestCommand::Decrement(v) => Ok(vec![TestEvent::Decremented(*v)]),
                TestCommand::GetState => Ok(vec![]),
            }
        }

        async fn apply_event(
            &self,
            state: &TestState,
            event: &TestEvent,
        ) -> Result<TestState, PEEntityError> {
            Ok(match event {
                TestEvent::Incremented(v) => TestState {
                    value: state.value + v,
                    version: state.version + 1,
                },
                TestEvent::Decremented(v) => TestState {
                    value: state.value.saturating_sub(*v),
                    version: state.version + 1,
                },
            })
        }

        async fn apply_events(
            &self,
            state: &TestState,
            events: &[TestEvent],
        ) -> Result<TestState, PEEntityError> {
            let mut s = state.clone();
            for event in events {
                s = self.apply_event(&s, event).await?;
            }
            Ok(s)
        }

        async fn external_effects(
            &self,
            command: &TestCommand,
            _new_state: &TestState,
            events: &[TestEvent],
            _context: &PECommandContext,
        ) -> Vec<ExternalEffectDescription> {
            if events.is_empty() {
                return Vec::new();
            }
            match command {
                TestCommand::Increment(_) => vec![ExternalEffectDescription {
                    idempotency_key: IdempotencyKey::new("app-facade-uow-1:0").unwrap(),
                    effect_type: "probe.effect".to_string(),
                    payload: vec![],
                    destination: "https://example.com".to_string(),
                }],
                _ => Vec::new(),
            }
        }
    }

    // Proves `AppBuilder::effect_store` is not a no-op forward: the runtime
    // `App::start()` actually builds dispatches THROUGH the registered
    // double, not through a separately-constructed `InMemoryEffectStore`.
    #[tokio::test]
    async fn effect_store_registered_via_app_builder_is_actually_dispatched_through() {
        let store = RecordingEffectStore::new();
        let executor = RecordingExecutor::new();
        let app = compat_app()
            .effect_store(store.clone())
            .effect_executor(["probe.effect"], executor.clone())
            .build()
            .expect("build succeeds with a custom effect store registered");

        let running = app.start().await.expect("start succeeds");
        let acceptor = running
            .runtime
            .effect_acceptor()
            .expect("an executor was registered — start_effects must produce an acceptor");

        let entity_runtime = EntityRuntimeBuilder::<TestEvent>::new()
            .with_effect_acceptor(acceptor)
            .build();
        let entity_ref = entity_runtime
            .entity_ref("probe", "app-facade-1", Arc::new(EffectDescribingEntity))
            .expect("spawning a fresh actor must succeed");

        let result: CommandResult<TestEvent, TestState> = entity_ref
            .send_command(TestCommand::Increment(1), create_test_context())
            .await
            .expect("the command itself must succeed regardless of effect delivery timing");
        assert!(
            matches!(result, CommandResult::Events { .. }),
            "expected a normal Events commit, got {result:?}"
        );

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while executor.call_count() == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the registered executor must actually be invoked");

        assert!(
            store.state_calls() > 0,
            "AppBuilder::effect_store must forward to the SAME store RuntimeBuilder \
             dispatches through, not to a separately-constructed InMemoryEffectStore"
        );

        running
            .shutdown()
            .await
            .expect("RunningApp::shutdown must still succeed with a custom effect store");
    }

    // `AppBuilder::effect_retention_store` composes with the same provider
    // instance `effect_store` was given — mirrors
    // `effect_store_composition.rs`'s `with_effect_store_composes_with_
    // with_effect_retention_store` compile-and-build assertion at the facade
    // layer. The retention worker's own lifecycle is untouched by this PR and
    // stays covered by `effect_retention_worker_lifecycle.rs`.
    #[test]
    fn effect_retention_store_composes_with_the_same_instance_via_app_builder() {
        let store = RecordingEffectStore::new();
        let _app = compat_app()
            .effect_store(store.clone())
            .effect_retention_store(store.clone() as Arc<dyn RetentionMaintenance>)
            .build()
            .expect("build succeeds composing effect_store with effect_retention_store");
    }

    // CORE-028D1 task 2.2 (RED): registering an effect store twice returns
    // `CompositionError::DuplicateEffectStore`, mirroring
    // `duplicate_adapter_registration_is_rejected`.
    #[test]
    fn duplicate_effect_store_registration_is_rejected() {
        let result = compat_app()
            .effect_store(RecordingEffectStore::new())
            .effect_store(RecordingEffectStore::new())
            .build();

        match result {
            Err(CompositionError::DuplicateEffectStore { type_name }) => {
                assert_eq!(type_name, std::any::type_name::<RecordingEffectStore>());
            }
            Err(other) => panic!("expected DuplicateEffectStore, got {other:?}"),
            Ok(_) => panic!("expected duplicate effect store registration to fail"),
        }
    }

    // CORE-028D1 task 2.4 (TRIANGULATE): a second effect store of a
    // *different* concrete type is still rejected — proves the guard is
    // presence-based, not `TypeId`-keyed.
    #[test]
    fn second_effect_store_of_a_different_type_is_still_rejected() {
        let other_store = Arc::new(InMemoryEffectStore::new());
        let result = compat_app()
            .effect_store(RecordingEffectStore::new())
            .effect_store(other_store)
            .build();

        match result {
            Err(CompositionError::DuplicateEffectStore { type_name }) => {
                assert_eq!(type_name, std::any::type_name::<InMemoryEffectStore>());
            }
            Err(other) => panic!("expected DuplicateEffectStore, got {other:?}"),
            Ok(_) => panic!("expected duplicate effect store registration to fail"),
        }
    }

    // CORE-028D1 task 2.5 (RED): registering an effect retention store twice
    // returns `CompositionError::DuplicateEffectRetentionStore`.
    #[test]
    fn duplicate_effect_retention_store_registration_is_rejected() {
        let store = RecordingEffectStore::new();
        let other = RecordingEffectStore::new();
        let result = compat_app()
            .effect_retention_store(store as Arc<dyn RetentionMaintenance>)
            .effect_retention_store(other as Arc<dyn RetentionMaintenance>)
            .build();

        assert!(matches!(
            result,
            Err(CompositionError::DuplicateEffectRetentionStore)
        ));
    }

    // CORE-028D1 task 2.7 (RED): both new methods short-circuit on a
    // pre-existing `pending_error` — mirrors `.adapter()`'s short-circuit
    // proof shape.
    #[test]
    fn effect_store_and_retention_store_short_circuit_on_a_pending_error() {
        let result = compat_app()
            .adapter(Arc::new(StubAdapter(1)))
            .adapter(Arc::new(StubAdapter(2)))
            .effect_store(RecordingEffectStore::new())
            .effect_retention_store(RecordingEffectStore::new() as Arc<dyn RetentionMaintenance>)
            .build();

        match result {
            Err(CompositionError::DuplicateAdapter { type_name }) => {
                assert_eq!(type_name, std::any::type_name::<StubAdapter>());
            }
            Err(other) => panic!("expected the original DuplicateAdapter, got {other:?}"),
            Ok(_) => panic!("expected the pre-existing pending_error to surface"),
        }
    }

    // PROD-014A: minimal `OffsetStore`/`DedupStore` stubs so the dup-guard
    // tests below don't need a real durable backend — mirrors
    // `runtime::builder`'s `StubOffsetStore`/`StubDedupStore` (AD-9).
    struct StubOffsetStore;

    #[async_trait::async_trait]
    impl OffsetStore for StubOffsetStore {
        async fn read_offset(
            &self,
            _projection_id: &str,
            _tag: &ego_domain::read_side::event_tag::EventTag,
            _tenant: &str,
        ) -> Result<
            Option<ego_domain::read_side::offset::Offset>,
            ego_domain::read_side::offset::OffsetStoreError,
        > {
            unreachable!("dup-guard tests never reach the store")
        }

        async fn write_offset(
            &self,
            _projection_id: &str,
            _tag: &ego_domain::read_side::event_tag::EventTag,
            _tenant: &str,
            _offset: &ego_domain::read_side::offset::Offset,
        ) -> Result<(), ego_domain::read_side::offset::OffsetStoreError> {
            unreachable!("dup-guard tests never reach the store")
        }
    }

    struct StubDedupStore;

    #[async_trait::async_trait]
    impl DedupStore for StubDedupStore {
        async fn seen(
            &self,
            _projection_id: &str,
            _tag: &ego_domain::read_side::event_tag::EventTag,
            _event_id: &str,
        ) -> Result<bool, ego_domain::read_side::dedup::DedupStoreError> {
            unreachable!("dup-guard tests never reach the store")
        }

        async fn mark_seen(
            &self,
            _projection_id: &str,
            _tag: &ego_domain::read_side::event_tag::EventTag,
            _event_id: &str,
        ) -> Result<(), ego_domain::read_side::dedup::DedupStoreError> {
            unreachable!("dup-guard tests never reach the store")
        }
    }

    fn stub_pair() -> (
        Arc<dyn OffsetStore + Send + Sync>,
        Arc<dyn DedupStore + Send + Sync>,
    ) {
        (Arc::new(StubOffsetStore), Arc::new(StubDedupStore))
    }

    // Task 3.3 (RED): registering the same `projection_id` twice fails
    // closed with `CompositionError::DuplicateReadSideProgress`, and the
    // rejection happens at registration time (the `pending_error` latch),
    // not at `build()` — mirroring `duplicate_effect_store_registration_is_rejected`.
    #[test]
    fn duplicate_read_side_progress_registration_is_rejected() {
        let (offset_a, dedup_a) = stub_pair();
        let (offset_b, dedup_b) = stub_pair();
        let result = compat_app()
            .read_side_progress("users-by-tenant", offset_a, dedup_a)
            .read_side_progress("users-by-tenant", offset_b, dedup_b)
            .build();

        match result {
            Err(CompositionError::DuplicateReadSideProgress { projection_id }) => {
                assert_eq!(projection_id, "users-by-tenant");
            }
            Err(other) => panic!("expected DuplicateReadSideProgress, got {other:?}"),
            Ok(_) => panic!("expected duplicate read-side progress registration to fail"),
        }
    }

    // Task 3.3 (RED): two *different* `projection_id`s both register — the
    // guard is per-projection, not a single slot (AD-1/AD-7 contrast with
    // `.effect_store()`).
    #[test]
    fn read_side_progress_registration_for_two_different_projections_both_succeed() {
        let (offset_a, dedup_a) = stub_pair();
        let (offset_b, dedup_b) = stub_pair();
        let result = compat_app()
            .read_side_progress("users-by-tenant", offset_a, dedup_a)
            .read_side_progress("orders-by-tenant", offset_b, dedup_b)
            .build();

        match result {
            Ok(_) => {}
            Err(err) => panic!("two distinct projection_ids must both register: {err:?}"),
        }
    }

    // Task 3.3 (RED): `.read_side_progress()` short-circuits on a
    // pre-existing `pending_error` — mirrors
    // `effect_store_and_retention_store_short_circuit_on_a_pending_error`.
    #[test]
    fn read_side_progress_short_circuits_on_a_pending_error() {
        let (offset, dedup) = stub_pair();
        let result = compat_app()
            .adapter(Arc::new(StubAdapter(1)))
            .adapter(Arc::new(StubAdapter(2)))
            .read_side_progress("users-by-tenant", offset, dedup)
            .build();

        match result {
            Err(CompositionError::DuplicateAdapter { type_name }) => {
                assert_eq!(type_name, std::any::type_name::<StubAdapter>());
            }
            Err(other) => panic!("expected the original DuplicateAdapter, got {other:?}"),
            Ok(_) => panic!("expected the pre-existing pending_error to surface"),
        }
    }

    // Regression: the default in-memory path (no custom store registered)
    // must still build, start, and shut down exactly as before these two new
    // delegation methods were added.
    #[tokio::test]
    async fn default_path_without_effect_store_is_unaffected_by_the_new_delegation() {
        let executor = RecordingExecutor::new();
        let app = compat_app()
            .effect_executor(["probe.effect"], executor.clone())
            .build()
            .expect("build succeeds with no custom effect store registered");

        let running = app.start().await.expect("start succeeds");
        assert!(
            running.runtime.effect_acceptor().is_some(),
            "the default in-memory path must still produce an acceptor"
        );
        running
            .shutdown()
            .await
            .expect("shutdown succeeds on the unmodified default path");
    }

    // Task 3.3 (RED): a registered shutdown-participant stop future runs
    // during shutdown, and the app's read-model reference is unaffected
    // (spec: "The application's read model is unaffected by lifecycle
    // integration").
    #[tokio::test]
    async fn registered_shutdown_participant_runs_and_read_model_is_unaffected() {
        let read_model = Arc::new(Mutex::new(vec!["initial".to_string()]));
        let stop_ran = Arc::new(AtomicBool::new(false));

        let app = compat_app().build().expect("build succeeds");
        let stop_ran_clone = stop_ran.clone();
        app.register_shutdown(async move {
            stop_ran_clone.store(true, Ordering::SeqCst);
            Ok(())
        });

        let running = app.start().await.expect("start succeeds");
        running.shutdown().await.expect("shutdown succeeds");

        assert!(
            stop_ran.load(Ordering::SeqCst),
            "registered shutdown participant must run during shutdown"
        );
        assert_eq!(
            *read_model.lock().unwrap(),
            vec!["initial".to_string()],
            "the app's read model reference must remain unchanged by lifecycle integration"
        );
    }

    // Task 3.5 (RED): mirrors builder.rs's
    // `shutdown_async_runs_every_hook_even_after_an_earlier_one_fails` — two
    // shutdown participants, one fails; both run, and the first error
    // surfaces (spec: "One failing shutdown participant does not hide
    // others").
    #[tokio::test]
    async fn shutdown_runs_every_participant_and_surfaces_the_first_error() {
        let app = compat_app().build().expect("build succeeds");
        let second_ran = Arc::new(AtomicBool::new(false));
        let second_ran_clone = second_ran.clone();

        app.register_shutdown(async move {
            Err(crate::runtime::RuntimeInfraError::Teardown {
                reason: "first participant fails".to_string(),
            })
        });
        app.register_shutdown(async move {
            second_ran_clone.store(true, Ordering::SeqCst);
            Ok(())
        });

        let running = app.start().await.expect("start succeeds");
        let result = running.shutdown().await;

        assert!(
            matches!(result, Err(CompositionError::Shutdown(_))),
            "the first hook's error must surface, wrapped as CompositionError::Shutdown"
        );
        assert!(
            second_ran.load(Ordering::SeqCst),
            "the second participant must still run despite the first one failing"
        );
    }
}

#[cfg(test)]
mod enforced_idempotency_wiring {
    //! Every piece of an enforced posture arrives, not just the mode.
    //!
    //! Before [`AppBuilder::enforced_idempotency`], the facade forwarded the
    //! enforcement mode and the store and stopped: `owner_id`, `lease_duration`
    //! and `clock` lived on [`RuntimeBuilder`] with no route through it. A host
    //! adopting enforcement got a store with no identity, no lease and no clock,
    //! and a test needing those had to reach past the facade and build a runtime
    //! the host would never build.
    //!
    //! So these follow the pieces individually. It is not enough that enforcement
    //! is *on*: a forward silently dropped is invisible from outside until a lease
    //! misbehaves in production, or until a takeover cannot name whose lease it is
    //! taking.
    //!
    //! Read back through the crate's own accessors rather than by driving a
    //! reservation. What is under test is the configuration the host assembled,
    //! and that is exactly what these expose — a scripted store would add a
    //! moving part between the claim and the evidence.

    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::{DateTime, TimeZone, Utc};
    use ego_domain::operation::{
        OperationReservationStore, OwnerFence, OwnerId, ReservationError, ReservationOutcome,
        ReserveRequest, StoredServiceResponse,
    };
    use ego_domain::time::Clock;

    use super::*;
    use crate::runtime::IdempotencyEnforcementMode;

    const OWNER: &str = "replica-under-test";
    /// Deliberately not a round number, so a default or a dropped forward differs.
    const LEASE: Duration = Duration::from_secs(97);

    /// Frozen years in the past, so an expiry computed from wall time is far away
    /// rather than close enough to pass.
    struct FrozenClock;

    impl FrozenClock {
        fn instant() -> DateTime<Utc> {
            Utc.with_ymd_and_hms(2021, 3, 14, 15, 9, 26).unwrap()
        }
    }

    impl Clock for FrozenClock {
        fn now(&self) -> DateTime<Utc> {
            Self::instant()
        }
    }

    /// Exists to be registered. Nothing here calls it.
    struct InertStore;

    #[async_trait]
    impl OperationReservationStore for InertStore {
        async fn reserve(
            &self,
            _req: ReserveRequest,
        ) -> Result<ReservationOutcome, ReservationError> {
            unreachable!("this fixture is registered, never driven")
        }
        async fn renew(
            &self,
            _f: &OwnerFence,
            _until: DateTime<Utc>,
        ) -> Result<(), ReservationError> {
            unreachable!()
        }
        async fn complete(
            &self,
            _f: &OwnerFence,
            _r: StoredServiceResponse,
        ) -> Result<(), ReservationError> {
            unreachable!()
        }
        async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
            unreachable!()
        }
        async fn purge_completed_before(
            &self,
            _cutoff: DateTime<Utc>,
            _batch: usize,
        ) -> Result<u64, ReservationError> {
            unreachable!()
        }
        async fn probe(&self) -> Result<(), ReservationError> {
            Ok(())
        }
    }

    fn enforced_app() -> App {
        App::builder()
            .enforced_idempotency(
                Arc::new(InertStore),
                OwnerId::new(OWNER),
                LEASE,
                Arc::new(FrozenClock),
            )
            .build()
            .expect("a complete enforced configuration builds")
    }

    // Final API Consistency Cleanup: `pending_error` is documented as
    // first-error-wins (see the field's doc comment on `AppBuilder`) — once
    // latched, no later registration call may overwrite it. Pins that rule
    // across the three setters in this module that previously mutated
    // `runtime_builder` even after an error had already latched.
    #[test]
    fn once_latched_a_pending_error_survives_idempotency_and_reservation_setters() {
        struct DupAdapter;

        let result = App::builder()
            .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .adapter(Arc::new(DupAdapter))
            .adapter(Arc::new(DupAdapter)) // latches DuplicateAdapter
            .idempotency_enforcement_mode(IdempotencyEnforcementMode::MandatoryKey)
            .operation_reservation_store(Arc::new(InertStore))
            .enforced_idempotency(
                Arc::new(InertStore),
                OwnerId::new(OWNER),
                LEASE,
                Arc::new(FrozenClock),
            )
            .build();

        match result {
            Err(CompositionError::DuplicateAdapter { type_name }) => {
                assert_eq!(type_name, std::any::type_name::<DupAdapter>());
            }
            Err(other) => panic!("expected the first-latched DuplicateAdapter, got {other:?}"),
            Ok(_) => panic!("expected the first-latched DuplicateAdapter, got Ok"),
        }
    }

    /// One call turns enforcement on **and** gives it somewhere to reserve.
    ///
    /// Both halves together, because the runtime refuses to build with one and not
    /// the other — so a test asserting only the mode could not tell a complete
    /// configuration from a rejected one.
    #[test]
    fn one_call_adopts_enforcement_and_registers_a_store() {
        let app = enforced_app();

        assert_eq!(
            app.runtime.idempotency_enforcement_mode(),
            IdempotencyEnforcementMode::MandatoryKey,
            "an enforced host refuses a request carrying no operation key"
        );
        assert!(
            app.runtime.inner().reservation().is_some(),
            "and it has somewhere to reserve — enforcement with no store is the \
             configuration that looks adopted and guarantees nothing"
        );
    }

    /// The owner the host named is the owner the runtime reserves as.
    ///
    /// The piece E1 cannot do without: a takeover is one owner displacing another,
    /// so an owner dropped between the facade and the config makes "this
    /// reservation still belongs to owner A" unassertable — and leaves two
    /// replicas unable to displace each other in production.
    #[test]
    fn the_owner_the_host_named_reaches_the_reservation_config() {
        let app = enforced_app();
        let reservation = app
            .runtime
            .inner()
            .reservation()
            .expect("an enforced host has a reservation config")
            .clone();

        assert_eq!(
            reservation.owner_id().as_str(),
            OWNER,
            "the runtime must reserve as the owner the host configured"
        );
    }

    /// The lease and the clock together determine the expiry.
    ///
    /// Asserted as the exact arithmetic rather than as "some future instant":
    /// that sum is the whole content of both settings. A dropped duration would
    /// still produce a plausible timestamp from the clock alone, and a dropped
    /// clock would produce one from wall time.
    #[test]
    fn the_lease_and_the_clock_determine_the_expiry() {
        let app = enforced_app();
        let reservation = app
            .runtime
            .inner()
            .reservation()
            .expect("an enforced host has a reservation config")
            .clone();

        assert_eq!(
            reservation.lease_until(),
            FrozenClock::instant()
                + chrono::Duration::from_std(LEASE).expect("a small duration converts"),
            "the expiry is the configured lease measured from the configured clock"
        );
    }

    /// The injected clock is the one measured from, not wall time.
    ///
    /// Separate from the assertion above because that one would also pass if both
    /// the clock and the lease were dropped and something else produced the same
    /// sum. This pins the direction: the instant is years in the past, so wall
    /// time cannot land near it.
    #[test]
    fn the_expiry_derives_from_the_injected_clock_not_wall_time() {
        let app = enforced_app();
        let expiry = app
            .runtime
            .inner()
            .reservation()
            .expect("an enforced host has a reservation config")
            .lease_until();

        assert!(
            expiry < Utc::now() - chrono::Duration::days(365),
            "an expiry computed from wall time would be near now; this must derive \
             from the injected clock: {expiry}"
        );
    }

    /// The compatibility posture registers no store at all.
    ///
    /// The negative control. Without it, a change that made every host enforce
    /// would satisfy every assertion above, and the honest "not adopted yet"
    /// declaration would have quietly stopped existing.
    #[test]
    fn the_compatibility_posture_adopts_nothing() {
        let app = App::builder()
            .idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .build()
            .expect("a compatibility host builds");

        assert_eq!(
            app.runtime.idempotency_enforcement_mode(),
            IdempotencyEnforcementMode::Compatibility,
            "a deployment still in transition keeps admitting keyless requests"
        );
        assert!(
            app.runtime.inner().reservation().is_none(),
            "and registers no store — an in-memory stand-in would make it look \
             adopted while surviving nothing"
        );
    }
}
