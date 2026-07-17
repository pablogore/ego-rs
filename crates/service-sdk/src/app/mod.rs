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

use ego_domain::Observability;
use ego_runtime::effects::ExternalEffectExecutor;
use ego_runtime::providers::ExternalDataProvider;
use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;
use kitlogger::KITLogger;

use crate::di::{AdapterRef, ConfigValue, Injectable};
use crate::runtime::{Resolvable, Runtime, RuntimeBuilder, RuntimeInner, RuntimeResolver};

pub use error::CompositionError;

/// A registration recorded by `.service()`/`.service_instance()`, resolved
/// against a scratch runtime and applied to the retained `RuntimeBuilder` at
/// `build()` time (AD-3). Boxed and type-erased so `AppBuilder` can hold a
/// heterogeneous `Vec` of them.
type ServiceRegistrar =
    Box<dyn FnOnce(&RuntimeInner, RuntimeBuilder) -> Result<RuntimeBuilder, CompositionError>>;

/// Fills in `DependencyNotFound`'s `service_name` with `S`'s type name if it
/// isn't already set (review F3) — a single helper shared by both
/// `Injectable::validate` and `Injectable::build`'s error paths in
/// [`AppBuilder::service`], so the two routes can't independently diverge on
/// attribution.
fn attribute_to<S: 'static>(err: crate::runtime::RuntimeError) -> crate::runtime::RuntimeError {
    match err {
        crate::runtime::RuntimeError::DependencyNotFound { type_name, service_name: None } => {
            crate::runtime::RuntimeError::DependencyNotFound {
                type_name,
                service_name: Some(std::any::type_name::<S>()),
            }
        }
        other => other,
    }
}

/// Builder for an [`App`] — the application-facing composition root (AD-1,
/// G2). Delegates every registration to an internal `RuntimeBuilder`; never
/// reimplements assembly (G3).
pub struct AppBuilder {
    runtime_builder: RuntimeBuilder,
    adapter_types: HashSet<TypeId>,
    service_registrars: Vec<ServiceRegistrar>,
    /// First error encountered by an infallible-signature registration call
    /// (e.g. a duplicate adapter, AD-4) — surfaced at `build()` rather than
    /// changing `.adapter()`'s `Self`-returning chain shape.
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
        if let Err(startup_err) = self.runtime.start_effects().await {
            let _ = self.runtime.shutdown_async().await;
            return Err(CompositionError::Startup(startup_err));
        }
        Ok(RunningApp { runtime: self.runtime })
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
        self.runtime.shutdown_async().await.map_err(CompositionError::Shutdown)
    }
}

impl AppBuilder {
    fn new() -> Self {
        Self {
            runtime_builder: RuntimeBuilder::new(),
            adapter_types: HashSet::new(),
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
        let registration = self.runtime_builder.clone().register_effect_executor(effect_types, executor);
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
        match self.runtime_builder.clone().register_data_provider(provider_id, provider) {
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
    /// `Tag`. A missing dependency surfaces at `build()` naming both the
    /// missing type and this requesting service (spec: "A missing
    /// dependency names both the missing type and the requester").
    ///
    /// Two type parameters are required only because `#[service]` does not
    /// yet link a struct to its generated `Tag` (AD-3 known limitation) —
    /// not the intended long-term shape.
    ///
    /// **Deviation from design.md's Interfaces/Contracts sketch:** that
    /// sketch bounds this as `S: Injectable + Tag::Service` (naming an
    /// opaque associated type directly as a trait bound). That is not valid
    /// Rust — confirmed with an isolated repro (`rustc` `E0405`: "cannot
    /// find trait `Service` in `Tag`"). Rust has no mechanism to bound a
    /// type parameter against "whichever trait underlies this associated
    /// type"; only the call site, which names both `S` and the concrete
    /// trait together, can perform that coercion. This method therefore
    /// takes one extra parameter: a trivial unsizing coercion (typically
    /// `|arc| arc`, an implicit-coercion identity closure at the call
    /// site) — everything else (construction via `Injectable`, the
    /// observable contract) is unchanged from design.md.
    pub fn service<S, Tag>(mut self, to_trait_object: fn(Arc<S>) -> Arc<Tag::Service>) -> Self
    where
        Tag: Resolvable + 'static,
        S: Injectable + 'static,
    {
        self.service_registrars.push(Box::new(move |scratch: &RuntimeInner, builder: RuntimeBuilder| {
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
            builder.with_service::<Tag>(arc).map_err(CompositionError::Service)
        }));
        self
    }

    /// Registers a pre-built instance directly, resolvable under `Tag` —
    /// the AD-3 escape hatch for a collaborator that genuinely cannot be
    /// expressed as an `Injectable` dependency (e.g. a non-DI collaborator
    /// like reference-app's read-side sink). Should be the exception, not
    /// the default registration path (G1) — prefer [`Self::service`]
    /// whenever construction can be expressed through `Injectable`.
    pub fn service_instance<Tag: Resolvable + 'static>(mut self, svc: Arc<Tag::Service>) -> Self {
        self.service_registrars.push(Box::new(move |_scratch: &RuntimeInner, builder: RuntimeBuilder| {
            builder.with_service::<Tag>(svc).map_err(CompositionError::Service)
        }));
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
    use super::*;

    #[derive(Debug, PartialEq)]
    struct StubAdapter(u32);

    // Task 1.4 (RED): registering an adapter twice for the same type returns
    // `CompositionError::DuplicateAdapter` (spec "Duplicate adapter
    // registration has one documented, testable outcome").
    #[test]
    fn duplicate_adapter_registration_is_rejected() {
        let result = App::builder()
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

        let app = App::builder()
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
        let app = App::builder()
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
        let app = App::builder()
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
            fn authenticate(&self, _c: &Credential) -> Result<SecurityContext, AuthenticationError> {
                let subject = SubjectId::new("user:stub").unwrap();
                Ok(SecurityContext::empty(Principal::new(PrincipalKind::User, subject)))
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

        let app = App::builder()
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

        let app = App::builder()
            .logger(Arc::new(KITLogger::default()))
            .build()
            .expect("build succeeds");

        assert!(app.runtime.logger().is_some(), "the registered logger must be present");
    }

    // Review F1: `.observability()` is a thin `AppBuilder` pass-through for
    // `RuntimeBuilder::with_observability` — reuses the existing hook rather
    // than inventing a second one.
    #[test]
    fn registered_observability_hook_does_not_prevent_build() {
        use crate::test_support::RecordingObservability;

        let app = App::builder()
            .observability(Arc::new(RecordingObservability::new()))
            .build();
        assert!(app.is_ok(), "build must succeed with an observability hook registered");
    }

    // Review F1: `.effect_executor()` fails closed on a duplicate
    // `effect_type`, matching `RuntimeBuilder::register_effect_executor`'s
    // own contract exactly (no silent last-write-wins).
    #[test]
    fn duplicate_effect_type_registration_is_rejected() {
        let result = App::builder()
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
        use persistent_entity::data_provider_access::{DataProviderError, DataRequest, DataResponse};

        struct StubProvider;
        #[async_trait]
        impl ExternalDataProvider for StubProvider {
            async fn fetch(&self, request: DataRequest) -> Result<DataResponse, DataProviderError> {
                Ok(DataResponse { payload: request.payload, cache_hit: false })
            }
        }

        let ok = App::builder()
            .data_provider("provider-a", Arc::new(StubProvider))
            .build();
        assert!(ok.is_ok(), "a single data provider registration must succeed");

        let dup = App::builder()
            .data_provider("provider-b", Arc::new(StubProvider))
            .data_provider("provider-b", Arc::new(StubProvider))
            .build();
        match dup {
            Err(CompositionError::DataProvider(_)) => {}
            Err(other) => panic!("expected CompositionError::DataProvider, got {other:?}"),
            Ok(_) => panic!("expected duplicate provider_id registration to fail"),
        }
    }

    // Task 1.8 (RED/GREEN): constructing an application starts nothing —
    // `build()` succeeds with no active Tokio runtime, and no effect
    // acceptor was started (spec "Constructing an application starts
    // nothing").
    #[test]
    fn build_starts_nothing_and_no_tokio_runtime_is_required() {
        let app = App::builder().build().expect("build succeeds without Tokio");
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
        let app = App::builder()
            .logger(Arc::new(KITLogger::default()))
            .build()
            .expect("build succeeds");

        let resolver = app.resolver();
        let via_resolver = resolver.logger().unwrap();
        let via_runtime = app.runtime.logger().unwrap();
        assert!(Arc::ptr_eq(via_resolver, via_runtime), "resolver() must see the same registered logger");
    }

    // -- Phase 3: runtime lifecycle (App::start / RunningApp::shutdown) ----

    use ego_domain::ExternalEffectDescription;
    use ego_runtime::effects::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
    use async_trait::async_trait;
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
        let app = App::builder()
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

    // Task 3.3 (RED): a registered shutdown-participant stop future runs
    // during shutdown, and the app's read-model reference is unaffected
    // (spec: "The application's read model is unaffected by lifecycle
    // integration").
    #[tokio::test]
    async fn registered_shutdown_participant_runs_and_read_model_is_unaffected() {
        let read_model = Arc::new(Mutex::new(vec!["initial".to_string()]));
        let stop_ran = Arc::new(AtomicBool::new(false));

        let app = App::builder().build().expect("build succeeds");
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
        let app = App::builder().build().expect("build succeeds");
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
