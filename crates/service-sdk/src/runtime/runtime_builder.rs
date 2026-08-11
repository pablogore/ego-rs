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
use crate::runtime::idempotency::{ReservationConfig, ReservationDecision, ReservationRejection};
use ego_domain::context::TenantId;
use ego_domain::event::DomainEvent;
use ego_domain::operation::OperationFingerprint;
use ego_domain::{Observability, SemanticEvent};
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
        reservation: Option<ReservationConfig>,
        observability: Option<Arc<dyn Observability>>,
        effect_acceptor_impl: Option<Arc<RuntimeEffectAcceptor>>,
        effect_drain_deadline: Duration,
        data_provider_access: Option<Arc<dyn DataProviderAccess>>,
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
            reservation,
            observability,
            effect_acceptor_impl,
            effect_started: AtomicBool::new(false),
            effect_drain_deadline,
            data_provider_access,
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

        let tenant = ctx
            .canonical_tenant()
            .and_then(|resolved| resolved.tenant_id().cloned());

        let decision = config.reserve(tenant, key, fingerprint.clone()).await?;

        // Stamped only after the store accepted it. A fingerprint left on a
        // context whose reservation was refused would be carried into an
        // aggregate's `CommandContext` by a body that never ran — and would sit
        // in a receipt describing work no reservation ever authorised.
        ctx.set_operation_fingerprint(fingerprint);
        Ok(Some(decision))
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
            None,
            None,
            None,
            Duration::from_secs(5),
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
            None,
            Some(obs),
            None,
            Duration::from_secs(5),
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
            None,
            None,
            None,
            Duration::from_secs(5),
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
            None,
            None,
            None,
            Duration::from_secs(5),
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
}
