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

use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;
use kitlogger::KITLogger;

use crate::di::Injectable;
use crate::runtime::{Resolvable, Runtime, RuntimeBuilder, RuntimeInner};

pub use error::CompositionError;

/// A registration recorded by `.service()`/`.service_instance()`, resolved
/// against a scratch runtime and applied to the retained `RuntimeBuilder` at
/// `build()` time (AD-3). Boxed and type-erased so `AppBuilder` can hold a
/// heterogeneous `Vec` of them.
type ServiceRegistrar =
    Box<dyn FnOnce(&RuntimeInner, RuntimeBuilder) -> Result<RuntimeBuilder, CompositionError>>;

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
    pub async fn start(self) -> Result<RunningApp, CompositionError> {
        self.runtime.start_effects().await.map_err(CompositionError::Startup)?;
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
            // (AD-3/AD-7): `Injectable::validate`'s generic default leaves
            // `service_name: None` (it doesn't know who's asking); fill it in
            // with the requesting service's type name here, exactly as
            // `try_build`'s validator loop does for `with_injectable`.
            S::validate(scratch).map_err(|err| {
                CompositionError::Validation(match err {
                    crate::runtime::RuntimeError::DependencyNotFound { type_name, .. } => {
                        crate::runtime::RuntimeError::DependencyNotFound {
                            type_name,
                            service_name: Some(std::any::type_name::<S>()),
                        }
                    }
                    other => other,
                })
            })?;
            let instance = S::build(scratch).map_err(CompositionError::Validation)?;
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
        assert!(app.runtime.inner().resolve_adapter::<StubAdapter>().is_ok());
        assert!(app.runtime.inner().resolve_adapter::<OtherAdapter>().is_ok());
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
        let resolved = app.runtime.inner().resolve_adapter::<StubAdapter>().unwrap();
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

        let resolved = app.runtime.inner().resolve_config::<StubConfig>();
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

    // Task 3.1 (RED): `App::start()` starts effects — `effect_acceptor()` is
    // `Some` post-start when an executor was registered. Constructs `App`
    // directly (white-box, private field — AppBuilder exposes no
    // `register_effect_executor` pass-through in Stage 1's scope) purely to
    // prove `App::start`'s wiring to `Runtime::start_effects`.
    #[tokio::test]
    async fn start_starts_effects_when_an_executor_was_registered() {
        let runtime = RuntimeBuilder::new()
            .register_effect_executor(["test.effect"], Arc::new(StubExecutor))
            .expect("registration succeeds")
            .build();
        let app = App { runtime };
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
