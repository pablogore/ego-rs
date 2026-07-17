# CORE-028 — Stage 1: Service Developer API / Application Composition — Exploration

> This document lives under `openspec/changes/core-026-developer-experience-refinement/`
> because that folder and its stage-0 spec were historically labeled "core-026"
> before the initiative was renumbered CORE-028. The folder is not renamed and
> no new CORE-titled folder was created, per explicit instruction — this is a
> known historical label mismatch, not an error.

Status: current-state investigation only. No proposed solutions, no
recommendations beyond noting gaps and open questions. Every claim below cites
a real file path.

## 1. `RuntimeBuilder` — full current API surface

File: `crates/service-sdk/src/runtime/builder.rs`

`RuntimeBuilder` (line 67) is `#[derive(Clone)]`, consuming-builder style
(`self -> Self`, most methods). Fields (lines 67–107): `registry:
ServiceRegistry`, `interceptor_chain: Arc<InterceptorChain>`, `authn: Option<Arc<dyn
AuthenticationProvider>>`, `authz: Option<Arc<dyn AuthorizationProvider>>`,
`logger: Option<Arc<KITLogger>>`, `adapters: HashMap<TypeId, Arc<dyn Any +
Send + Sync>>`, `configs: HashMap<TypeId, Arc<dyn Any + Send + Sync>>`,
`tenant_enforcement_mode: TenantEnforcementMode`, `validators:
Vec<ValidatorEntry>`, `observability: Option<Arc<dyn Observability>>`,
`effect_executors: ExecutorRegistry`, `delivery_config: DeliveryConfig`,
`effect_drain_deadline: Duration`, `data_provider_registry:
ExternalDataProviderRegistry`, `data_providers_for_teardown:
Vec<Arc<dyn ExternalDataProvider>>`.

Public methods, in the order a caller would typically chain them:

- `RuntimeBuilder::new() -> Self` (line 111) — all-empty defaults;
  `tenant_enforcement_mode` defaults to `TenantEnforcementMode::AuthenticatedOnly`.
- `with_security(self, authn: Arc<dyn AuthenticationProvider>, authz: Arc<dyn
  AuthorizationProvider>) -> Self` (line 136) — both-or-nothing pairing.
- `with_logger(mut self, logger: Arc<KITLogger>) -> Self` (line 154) — takes
  an **already-constructed and initialized** logger; never constructs one
  itself.
- `with_adapter<A: Send + Sync + 'static>(mut self, adapter: Arc<A>) -> Self`
  (line 162) — infallible, last-write-wins per concrete type `A`, keyed by
  `TypeId`.
- `with_config<C: Send + Sync + 'static>(mut self, value: Arc<C>) -> Self`
  (line 170) — same shape as `with_adapter`, separate `TypeId` namespace
  (adapter and config of the identical concrete type do not collide — proven
  by test `adapter_and_config_of_same_concrete_type_do_not_collide`, line
  905).
- `with_service<Tag: Resolvable + 'static>(mut self, svc:
  Arc<Tag::Service>) -> Result<Self, RegistryError>` (line 182) — fallible;
  duplicate `(Tag, version)` registration is `Err(RegistryError::
  DuplicateService)`, unlike `with_adapter`/`with_config`'s silent
  last-write-wins.
- `with_injectable<S: Injectable>(mut self) -> Self` (line 195) — records
  `S::validate` for later `try_build()`; has zero effect on `build()`.
- `with_tenant_enforcement_mode(mut self, mode: TenantEnforcementMode) ->
  Self` (line 209).
- `with_observability(mut self, obs: Arc<dyn Observability>) -> Self` (line
  218).
- `register_effect_executor(mut self, effect_types: impl
  IntoIterator<Item = impl Into<String>>, executor: Arc<dyn
  ExternalEffectExecutor>) -> Result<Self, DuplicateEffectType>` (line 235) —
  fails closed on a duplicate `effect_type`; registering at least one
  executor is what makes `build()` construct the effects subsystem at all
  (zero-cost gate, line 323).
- `register_data_provider(mut self, provider_id: impl Into<String>, provider:
  Arc<dyn ExternalDataProvider>) -> Result<Self, DuplicateProviderId>` (line
  260) — same zero-cost-gate shape; also tracks every distinct `Arc` for
  single-owner teardown (line 349, `data_providers_for_teardown`),
  deduplicated by `Arc::ptr_eq` so an aliased provider under two IDs is torn
  down once.
- `with_delivery_config(mut self, config: DeliveryConfig) -> Self` (line 280).
- `with_effect_drain_deadline(mut self, deadline: Duration) -> Self` (line
  291) — default `DEFAULT_EFFECT_DRAIN_DEADLINE = Duration::from_secs(5)`
  (line 37).
- `build(self) -> Runtime` (line 301) — **always succeeds** (infallible).
  Constructs `RuntimeInner::new_with_logger(...)` (11 positional args, line
  352). Never calls `.start()` on the effects acceptor — that only happens
  inside `Runtime::start_effects`, because `build()` may run before any Tokio
  runtime exists (comment, line 316–322).
- `try_build(mut self) -> Result<Runtime, RuntimeError>` (line 391) — calls
  the same infallible `build()`, then runs every recorded validator against
  the built runtime's resolved tables; fails fast on the **first**
  registered missing dependency (proven by test
  `try_build_reports_only_the_first_registered_service_when_multiple_are_missing_dependencies`,
  line 995) and names both the missing type and the requesting service.
  `Injectable::build` is never called here — only `Injectable::validate`.

Registration order is NOT independently ordered/validated by `RuntimeBuilder`
itself beyond the `validators: Vec<...>` linear scan in `try_build`; adapters
and configs are plain `HashMap`s (no ordering guarantee beyond insertion into
the map, though last-write-wins is deterministic per key).

`RuntimeBuilder::build()` returns a `Runtime` (not `Result`) — the only
fallible construction paths are the `Result`-returning individual builder
calls (`with_service`, `register_effect_executor`,
`register_data_provider`) and `try_build()`.

## 2. `Runtime` / `RuntimeInner` — responsibilities, public vs internal

`Runtime` (file: `crates/service-sdk/src/runtime/builder.rs`, line 419) is a
thin `Arc<RuntimeInner>` wrapper. Public API:

- `inner(&self) -> &Arc<RuntimeInner>` (line 425).
- `security_providers(&self) -> Option<&SecurityProviders>` (line 430).
- `logger(&self) -> Option<&Arc<KITLogger>>` (line 437).
- `async fn start_effects(&self) -> Result<(), RuntimeInfraError>` (line
  466) — MUST be called exactly once, from inside an active Tokio runtime,
  after `build()`, if any effect executor was registered; idempotent no-op
  on later calls and in the zero-cost path. Until called,
  `Runtime::effect_acceptor()` returns `None` even if an executor was
  registered (`effect_started: AtomicBool`, guarded via
  `compare_exchange`).
- `effect_acceptor(&self) -> Option<Arc<dyn EffectAcceptor>>` (line 516).
- `data_provider_access(&self) -> Option<Arc<dyn DataProviderAccess>>` (line
  533) — available immediately after `build()`, no separate start step
  (`RuntimeDataProviderAccess` never spawns a task).
- `resolve<Tag: Resolvable + 'static>(&self) -> Result<Tag::Proxy,
  RuntimeError>` (line 544) — the canonical service-resolution path; not
  cached, constructs a fresh proxy per call wrapping the same registered
  `Arc`.
- `shutdown(&self) -> Result<(), RuntimeInfraError>` (line 563) — sync,
  drains the `TeardownStack` in reverse construction order; idempotent.
- `register_async_teardown<F: Future<...> + Send + 'static>(&self, hook: F)`
  (line 589) — `&self`, not on the builder, because the motivating case (a
  spawned read-side scheduler's `stop()` future) is only constructible after
  `Runtime` already exists.
- `async fn shutdown_async(&self) -> Result<(), RuntimeInfraError>` (line
  610) — awaits every registered async hook in registration order (ALL of
  them, even after one fails), THEN calls sync `shutdown()` regardless,
  returning the FIRST hook error if any, else `shutdown()`'s result.

`RuntimeInner` (file: `crates/service-sdk/src/runtime/runtime_builder.rs`,
struct at line 193) is described in its own doc comment as "a façade that
delegates to smaller internal structs" (line 176). Fields: `registry:
ServiceRegistry` (`pub(crate)`), `interceptor_chain: Arc<InterceptorChain>`
(`pub(crate)`), `security_providers` (`pub(crate)`), `resolved:
DependencyTable` (private), `tenant_resolver: TenantResolver` (private),
`logger: Option<Arc<KITLogger>>` (private), `observability` (private),
`teardown: Mutex<TeardownStack>` (`pub(super)`), `async_teardown:
Mutex<Vec<AsyncTeardownHook>>` (`pub(super)`), `effect_acceptor_impl`
(`pub(crate)`), `effect_started: AtomicBool` (`pub(crate)`),
`effect_drain_deadline: Duration` (`pub(crate)`), `data_provider_access`
(`pub(crate)`).

Sole constructor is `RuntimeInner::new_with_logger(...)` (`pub(super)`, line
289) — doc comment explicitly states this closes an "external bypass" that
would let rogue instances skip the authorization check (TASK-014 note, line
282–288). A handful of `#[cfg(test)]`-only constructors exist
(`for_test`, `for_test_with_mode`, `for_test_with_observability`,
`for_test_with_authz`) — none are reachable outside the crate's own test
module.

Public-but-`#[doc(hidden)]` accessors exist solely for macro-generated code
in the separate `service-sdk-macros` crate to reach otherwise-`pub(crate)`
state: `logger()` (line 333), `authorization_provider()` (line 394),
`record_security_denial()` (line 420) — each doc comment explicitly says
"Application code MUST NOT call this method directly."

Ordinary application-facing methods on `RuntimeInner`: `resolve_projection`,
`resolve_adapter`, `resolve_config` (lines 340–358), `check_dependency`
(`pub(crate)`, line 369, used by `Injectable::validate`'s default),
`enforce_tenant` (line 460), `issue_cross_tenant_permit` (`pub(crate)`, line
494).

## 3. `DependencyTable` — registration/resolution

File: `crates/service-sdk/src/runtime/runtime_builder.rs`, struct at line
51 (`pub(super)`, i.e. not visible outside the `runtime` module at all).

Three `HashMap<TypeId, Arc<dyn Any + Send + Sync>>` maps: `projections`,
`adapters`, `configs` (lines 52–55) — three separate namespaces keyed by
`TypeId`, confirmed non-colliding by the same-concrete-type test noted
above.

Construction: `DependencyTable::with_registrations(adapters, configs)` (line
70) — takes the two `RuntimeBuilder`-collected maps as **named parameters**
specifically "so they can't be silently transposed at the call site"
(comment, line 68). `projections: HashMap::new()` — always empty at
construction; there is no `RuntimeBuilder` method that registers a
projection today (confirmed by grep: no `with_projection` exists anywhere in
`builder.rs`). The only way a `ProjectionRef<T>` resolves is if some other
code path inserts into `resolved.projections` directly — no such production
call site was found; it is exercised only by unit tests in
`runtime_builder.rs` (lines 705–716) that reach into the private field.

Resolution methods `resolve_projection<T>`, `resolve_adapter<A>`,
`resolve_config<C>` (lines 77–101) all follow the same shape: `HashMap::get`
by `TypeId::of::<T>()`, `.and_then(|arc| arc.clone().downcast::<T>().ok())`,
wrap in the typed ref (`ProjectionRef::new`/`AdapterRef::new`/
`ConfigValue::new`), else `Err(RuntimeError::DependencyNotFound { type_name,
service_name: None })` (`service_name` is filled in later only by the
`try_build()` validator path).

## 4. `Injectable` trait

File: `crates/service-sdk/src/di/mod.rs`, trait at line 88.

```rust
pub trait Injectable: Send + Sync {
    fn dependencies() -> Vec<DepKey> where Self: Sized;
    fn validate(rt: &crate::runtime::RuntimeInner) -> Result<(), crate::runtime::RuntimeError>
    where Self: Sized { /* default: loops dependencies(), calls rt.check_dependency */ }
    fn build(rt: &crate::runtime::RuntimeInner) -> Result<Self, crate::runtime::RuntimeError>
    where Self: Sized;
}
```

`DepKey` (line 76) is a 4-variant enum: `Entity(TypeId, &'static str)`,
`Projection(TypeId, &'static str)`, `Adapter(TypeId, &'static str)`,
`Config(TypeId, &'static str)`. Note: `DepKey::Entity` exists as a variant,
but `RuntimeInner::check_dependency` (runtime_builder.rs line 369–381)
**unconditionally returns `Err` for `DepKey::Entity`** — comment: "no entity
table exists yet (CORE-006 is not landed), so a declared `Entity` dependency
must not silently pass validation." This is a real, current gap: a service
declaring an `Entity` dependency via `Injectable::dependencies()` can never
pass `try_build()` validation today, by design, not oversight.

`validate()`'s default implementation is generic over `dependencies()` —
"zero per-service codegen" (doc comment, di/mod.rs line 95) — a pure
presence check that constructs nothing.

Three typed wrapper structs pair with `DepKey`/field-type detection:
`ProjectionRef<P>`, `AdapterRef<A>`, `ConfigValue<T>` (di/mod.rs lines
15–72) — each a thin `Arc`-wrapping newtype with `Deref` to the inner value.
There is no wrapper type for `Entity` in this file (consistent with #3
above — no entity table exists at this layer).

## 5. `#[service]` macro / codegen

File: `crates/service-sdk-macros/src/lib.rs`.

The single `#[proc_macro_attribute] pub fn service(...)` (line 59) dispatches
on whether it's applied to a `trait` or a `struct` (line 63–74):

- **On a trait** (`expand_service_trait`, line 77): generates a `{Trait}Tag`
  and `{Trait}Ref` type plus a `ServiceContract` impl encoding a semver
  version (parsed from `#[service(version = "x.y.z")]`, defaulting to
  `"1.0.0"` — line 82, with a compile-time `bad_semver_err` for non-numeric
  segments). Also processes `#[operation]`/`#[authorize]`/`#[tenant_scoped]`
  attributes on trait methods (via the `SdkAttr` enum, lines 6–29) to
  generate per-operation forwarding/guard code — the full body of this
  expansion continues past what was read here (file is 1600+ lines) and was
  not read to completion; this document does not claim full coverage of the
  trait-macro's generated guard logic.
- **On a struct** (`expand_service_struct`, line 554): for each named field,
  classifies the field's type (`classify_field_type` / `classify_field_init`,
  starting line 598) — recognizing `ProjectionRef<T>`, `AdapterRef<A>`,
  `ConfigValue<C>` generic wrapper types by name — and generates:
  ```rust
  impl ego_service_sdk::di::Injectable for #struct_name {
      fn dependencies() -> Vec<DepKey> { vec![#(#dep_keys),*] }
      fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> {
          Ok(Self { #(#field_inits),* })  // DI fields resolve via rt.resolve_*;
                                          // non-DI fields use Default::default()
      }
  }
  ```
  This confirms `#[service]` on a struct is exactly, and only, an
  `Injectable`-impl generator — it does not register anything with any
  runtime or builder itself; registration still requires either
  `RuntimeBuilder::with_injectable::<S>()` (validation-only) or a fixture's
  `.service::<S>()` call (testkit, see #13) to actually invoke
  `Injectable::build`.

No macro or codegen named `#[app]`, `#[entity]`, or similar was found
anywhere in `crates/service-sdk-macros` (confirmed by reading the file's
top-level macro list, which only exposes `service`, and by the `SdkAttr`
enum's three variants: `Operation`, `Authorize`, `TenantScoped`).

## 6. Adapter registration path today

`RuntimeBuilder::with_adapter<A>(self, adapter: Arc<A>) -> Self`
(builder.rs line 162) is the only registration entry point. It is infallible
and last-write-wins per concrete type. There is no adapter-specific
lifecycle/teardown hook distinct from the generic
`register_async_teardown`/sync `TeardownStack` mechanisms — an adapter that
needs shutdown work must be wired manually by the host (as reference-app
does not currently do for any adapter; reference-app registers no adapters
at all today — see #14). A host resolves a registered adapter via
`RuntimeInner::resolve_adapter::<A>()` (through a `#[service]`-generated
`Injectable::build`, or directly).

## 7. Config registration/loading path today

Two independent config layers exist and are explicitly documented as
**not unified**:

1. `RuntimeBuilder::with_config<C>(self, value: Arc<C>) -> Self`
   (builder.rs line 170) — generic DI config values resolved via
   `resolve_config::<C>()`, same shape as adapters.
2. The **logging** subtree specifically flows through
   `kit_config::ConfigLoader` (external crate) →
   `ConfigurationProvider::from_value(serde_json::Value) -> Self`
   (`crates/service-sdk/src/runtime/config_provider.rs` line 60) →
   `.logging() -> Result<LoggingSettings, RuntimeInfraError>` (line 65) →
   `build_logger(&LoggingSettings) -> Result<Option<Arc<KITLogger>>,
   RuntimeInfraError>` (`crates/service-sdk/src/runtime/logger.rs` line 24)
   → `RuntimeBuilder::with_logger(Arc<KITLogger>)`.

`RuntimeBuilder`'s own doc comment (builder.rs lines 47–54) states it "has no
configurable scalar fields of its own beyond
`with_tenant_enforcement_mode`" and explicitly redirects mailbox
capacity/concurrency/passivation-timeout/persistence-tenant-mode to
`persistent_entity::EntityRuntimeBuilder::from_value` (see #1/#11 — a
**third**, separate config-consuming builder, per entity-runtime, not
per-app). There is no single config object or loader that feeds
`RuntimeBuilder`, `EntityRuntimeBuilder`, and generic DI configs all at once
today — reference-app's `AppConfig` (lib.rs lines 82–118) is an
application-defined struct that manually fans out its own subtrees
(`runtime`, `jwt`, `scheduler`, `database`, `transport`) to whichever
service/builder owns each — this fan-out is entirely hand-written in
`build_runtime` (lib.rs lines 186–241), not framework-provided.

## 8. Security-provider registration path today

`RuntimeBuilder::with_security(self, authn: Arc<dyn AuthenticationProvider>,
authz: Arc<dyn AuthorizationProvider>) -> Self` (builder.rs line 136) — the
only entry point; both-or-nothing (stored as `Option<(Arc<dyn ...>, Arc<dyn
...>)>`, line 302–305). Doc comment states "The runtime does not
automatically enforce authentication — callers are responsible for invoking
the provider and populating `ServiceContext` on each request" (line
134–135). Confirmed by reference-app: `Hs256AuthenticationProvider` is
constructed by hand (lib.rs lines 189–195) from a `JwtProviderConfig` +
`KeyResolver` + `Clock`, entirely outside `RuntimeBuilder`, then passed in
already-constructed.

## 9. Observability and logging bootstrap

Logging: see #7 (`build_logger`). It is a **host-side, pre-`RuntimeBuilder`**
step — `RuntimeBuilder::with_logger` only ever takes ownership of an
already-initialized `KITLogger` (confirmed by builder.rs doc comment lines
148–153: "The logger is constructed and initialized by the host ... before
`RuntimeBuilder::new()` is ever called ... `RuntimeBuilder` never constructs
it").

Observability (structured event tracing, distinct from logging):
`RuntimeBuilder::with_observability(self, obs: Arc<dyn Observability>) ->
Self` (builder.rs line 218) — optional, defaults to `None`; used today only
for macro-guard security-denial events
(`RuntimeInner::record_security_denial`, runtime_builder.rs line 420) via
`SecurityDenialKind` (3 variants: `MissingContext`, `TenantMismatch`,
`AuthorizationDenied`, lines 122–131). No other production call site emits
through `Observability` was found in the files read.

## 10. Lifecycle and shutdown

Two independent teardown mechanisms on `Runtime`:

- **Sync**: `TeardownStack` (`crates/service-sdk/src/runtime/logger.rs`,
  referenced at builder.rs lines 306–309) — the logger is pushed onto it at
  `build()` time if present; drained in reverse construction order by
  `Runtime::shutdown()`.
- **Async** (additive, "Finding 6"): `Vec<AsyncTeardownHook>` on
  `RuntimeInner.async_teardown`, populated only via
  `Runtime::register_async_teardown` (post-build, `&self`, not on the
  builder — builder.rs lines 571–598). `Runtime::shutdown_async()` runs
  every hook in registration order (even after an earlier one fails, proven
  by test `shutdown_async_runs_every_hook_even_after_an_earlier_one_fails`,
  line 1140), THEN unconditionally calls the sync `shutdown()`, surfacing
  the first hook error if any (not swallowing it).

`RuntimeBuilder::build()` itself registers exactly one async teardown hook
automatically — for `data_providers_for_teardown` (builder.rs lines 373–380)
— driving every distinct registered `ExternalDataProvider::shutdown()`
exactly once. The effects subsystem's drain hook is registered separately,
by `Runtime::start_effects` (line 491), not by `build()`.

Ownership: **the caller (host `main.rs`) is who decides the overall
shutdown order**, not any framework type — reference-app's `main.rs` (lines
39–67) is explicit: (1) `ego_transport::serve(...)`'s own graceful shutdown
drains in-flight HTTP first, (2) only then does `rt.shutdown_async()` run
(which itself runs the read-side scheduler's stop hook, registered manually
by `main.rs` at line 47, before the sync stack). There is no framework-level
"shut down everything in the right order automatically" facility today —
every ordering decision above (HTTP-before-runtime, read-side-hook-before-
sync-stack) is manually sequenced/registered by the calling application.

## 11. Background tasks — spawn/track/stop

`TagSchedulerImpl<E>::spawn_projection` is the only framework-provided
spawn+stop convenience found (file: `crates/runtime/src/read_side/
scheduler.rs`, line 238). See #12 for its exact confirmed shape. No other
background-task spawn/track/stop convenience (e.g. a generic
"BackgroundTaskHandle" or a supervisor) was found anywhere in `crates/runtime`
or `crates/service-sdk`.

## 12. `spawn_projection` — exact current shape, vs. stage-0 spec

Confirmed against the stage-0 spec at
`openspec/changes/core-026-developer-experience-refinement/specs/read-side/spec.md`
— **the spec accurately describes the shipped code**; no deviation found.

```rust
// crates/runtime/src/read_side/scheduler.rs, line 238
impl<E> TagSchedulerImpl<E> where E: Clone + Send + Sync + 'static {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_projection<F, H, S, D, O, R>(
        self,
        tag_provider: F,       // Fn() -> Vec<EventTag> + Send + Sync + 'static — called FRESH each poll
        interval: Duration,    // required, explicit — no default
        projection_id: String,
        tenant: String,
        handler: H,            // Handler<E> + Clone + Send + Sync + 'static
        read_store: S,         // ReadSideStore<E> + Send + Sync + Clone + 'static
        dedup_store: D,        // DedupStore + Send + Sync + Clone + 'static
        offset_store: O,       // OffsetStore + Send + Sync + Clone + 'static
        reporter: R,           // ProgressReporter + Clone + Send + Sync + 'static
        on_error: impl Fn(Box<dyn std::error::Error>) + Send + Sync + 'static,
    ) -> ReadSideProjectionHandle
}
```

`ReadSideProjectionHandle` (line 211): two private fields, `stop_tx:
watch::Sender<bool>` and `task: tokio::task::JoinHandle<()>`. Its only
public method:

```rust
pub async fn stop(self) -> Result<(), tokio::task::JoinError>
```

— consumes `self` (compile-time double-stop prevention, confirmed), sends
the stop signal, then `.await`s the join handle, surfacing a `JoinError`
(panic/abort) rather than swallowing it (line 217–224). Internally,
`spawn_projection` creates the `watch::channel(false)` itself and calls the
pre-existing `run_until_stopped` (line 136) — confirmed `tag_provider` is
invoked fresh every loop iteration (line 164: `let tags = tag_provider();`
inside the `loop {}`), not cached.

Caller owns the `ReadSideProjectionHandle` — there is no framework-side
registry of spawned projections; reference-app's `main.rs` holds it
(implicitly, via `ReadSideRuntime` wrapping it, see #14) and is responsible
for calling `.stop()` (via `register_async_teardown`).

## 13. Testkit — constructing/testing without the full runtime

File: `crates/testkit/src/fixtures.rs`. `ServiceTestFixture` (line 49) always
owns a real, full `Runtime` internally — there is no lighter-weight
"construct without a runtime at all" path; the fixture's value is
convenience/isolation (each fixture owns its own runtime; "two fixtures
share no state," line 47), not avoiding `Runtime` construction.

`FixtureBuilder` (line 116) wraps a real `RuntimeBuilder` internally
(`runtime_builder: RuntimeBuilder` field, line 124) and exposes thin
pass-throughs: `.with_service::<Tag>()` → `RuntimeBuilder::with_service`,
`.with_observability()` → `RuntimeBuilder::with_observability`. Doc comments
repeatedly assert "no parallel...assembly happens in TestKit" (lines 138–141,
172–177) — the stated design principle (module doc, lib.rs line 4: "Same-
contract principle").

`ServiceTestFixture::service<S: Injectable>(&self) -> Result<S, RuntimeError>`
(fixtures.rs line 80) calls `S::build(self.runtime.inner())` directly —
the real `Injectable::build` path, proven identical for both hand-rolled and
`#[service]`-macro-generated `Injectable` impls by tests at lines 277–316.

`ServiceTestFixture::resolve<Tag: Resolvable>(&self) -> Result<Tag::Proxy,
RuntimeError>` (line 95) is a direct pass-through to `Runtime::resolve`.

Other testkit modules present (not read in full — flagged as unverified
beyond their existence): `crates/testkit/src/{security,authz,config,logger,
identity,context,jwt,effects,providers}.rs`, re-exported from `lib.rs`
(lines 25–37): `assert_authorized`/`assert_denied`, `TestConfig`,
`test_context`/`TestContextBuilder`, `RecordedAttempt`/`RecordingExecutor`,
`principal`/`PrincipalBuilder`, `TestJwtBuilder`,
`CapturedRecord`/`CapturingLogger`, `RecordingDataProvider`/
`StaticDataProvider`, `authenticated`/`authenticated_with_claims`,
`DenyAllAuthorizationProvider`/`ScriptedAuthorizationProvider`,
`AllowAllAuthorizationProvider` (gated behind `dev-providers` feature).

## 14. Reference application — end-to-end wiring today

Files: `examples/reference-app/src/{main.rs, lib.rs, application.rs,
read_side/mod.rs}`.

`main.rs` (39 lines of actual logic, lines 39–67):
```rust
let config = AppConfig::default();
let BuiltRuntime { runtime: rt, authn, read_side: read_side_handles } = build_runtime(&config)?;
let rt = Arc::new(rt);
let query = read_side_handles.query.clone();
let read_side_runtime = read_side_handles.spawn();
rt.register_async_teardown(read_side_runtime.stop());
let state = AppState::new(rt.clone(), authn);
let router = build_router(state, query);
let listener = TcpListener::bind("127.0.0.1:3000").await?;
ego_transport::serve(listener, router, shutdown_signal()).await?;
rt.shutdown_async().await?;
```

`build_runtime` (`lib.rs`, lines 186–241) is the actual composition root —
its doc comment names the pipeline explicitly: "Host -> AppConfig ->
service construction -> RuntimeBuilder" (line 8). Concrete steps performed
by hand, in order:

1. `config.validate()` (application-defined `Validate` impl, lib.rs 95–118,
   including a genuinely cross-subtree rule that no single subtree's
   `validate()` could express alone — lines 103–114).
2. Manually construct a `KeyResolver` + `Hs256AuthenticationProvider` (JWT
   auth) and a hand-written `ReferenceAllowAllAuthorization` (authz stub) —
   lines 189–196.
3. Manually run `kit_config::ConfigLoader` → `ConfigurationProvider::
   from_value` → `.logging()` → `build_logger` — lines 205–219 (see #7).
4. Manually build TWO separate `EntityRuntimeBuilder::new().build()` calls,
   one per aggregate event enum (`OrganizationEnsured`, `UserRegistered`) —
   lines 222–223 (see #15 for why this is two, not one).
5. Manually construct `SharedReadSideStore`, `ReadSideSink`,
   `ReadSideHandles::new(...).with_logger(...)` — lines 228–230.
6. Manually construct `RegisterUserImpl::new(...).with_read_side_sink(...)`
   — line 232.
7. Chain `RuntimeBuilder::new().with_security(...).with_service::<RegisterUserTag>(...)?`,
   conditionally `.with_logger(...)`, then `.build()` — lines 234–241.
8. Return `BuiltRuntime { runtime, authn, read_side: read_side_handles }`
   (a hand-written struct, lines 159–163) — not the `spawn`ed read-side; the
   caller (`main.rs`) decides when to spawn (line 46) and how to wire its
   stop into teardown (line 47).

`BuiltRuntime` (lib.rs, struct at line 159) is application-defined, not a
framework type — this is the "BuiltRuntime tuple to named struct" change
referenced in this initiative's context (historically labeled PR #165).

**Concrete repeated boilerplate a composition facade could plausibly
remove**, all hand-written in `build_runtime`/`main.rs` today: (a) the
authn/authz provider construction sequence, (b) the kit-config → logger
pipeline, (c) constructing N `EntityRuntimeBuilder`s (one per aggregate)
and wiring them into services, (d) constructing read-side wiring
(`SharedReadSideStore`/`ReadSideSink`/`ReadSideHandles`) and manually
spawning + registering its stop as an async teardown hook, (e) the manual
`Arc::new(rt)` + explicit two-phase shutdown sequencing in `main.rs`.

## 15. Other builders/registries a developer touches today

- `EntityRuntimeBuilder<E>` (`crates/persistent-entity/src/builder.rs`, line
  18) — generic over one domain-event enum `E` per instance. Fields include
  `mailbox_capacity`, `concurrency_budget`, `passivation_timeout`,
  `publisher`, `snapshot_strategy`, `single_tenant_mode`, `tenant_id`,
  `registry`, `event_bus_capacity`, `event_store`, `snapshot_store`,
  `effect_acceptor`. `.build() -> EntityRuntime<E>` (line 183) supplies
  in-memory defaults for anything not explicitly set (`NoopPublisher`,
  `PeriodicSnapshotStrategy::new(100)`, `InMemoryEventStore`,
  `InMemorySnapshotStore`, a fresh `EntityRegistry`). `.from_value(value:
  serde_json::Value) -> Result<Self, serde_json::Error>` (line 157) is its
  own independent kit-config-style entry point, separate from
  `RuntimeBuilder`'s config path (see #7) and from `EventBusConfig`/
  `DatabaseConfig` used elsewhere in reference-app.
- `EntityRuntime<E>::entity_ref<C, S>(&self, entity_type: &'static str,
  entity_id: impl Into<String>, entity_handler: Arc<dyn
  PersistentEntity<Command = C, Event = E, State = S>>) -> Result<impl
  EntityRef<Command = C>, EntityError>` (`crates/persistent-entity/src/
  runtime.rs`, line 154) is the per-call construction path for an
  individual aggregate instance — confirmed as the call site
  `RegisterUserImpl::register` uses twice (application.rs lines 207–211,
  231–235), once per `EntityRuntime` (org, then user).
- `ServiceRegistry` (`crates/service-sdk/src/registry/`, referenced by
  `RuntimeBuilder.registry` field) — not independently investigated beyond
  its use inside `RuntimeBuilder`/`Runtime::resolve`; flagged as
  unverified in isolation.
- `InterceptorChain` (`crates/service-sdk/src/interceptor.rs`, referenced)
  — not independently investigated; flagged as unverified in isolation.

## Open questions / gaps explicitly flagged (per task instructions)

- **`.entity::<E>()` has no confirmed referent today.** The only
  "entity" concept in the codebase is `EntityRuntimeBuilder<E>`, generic
  per-domain-event-enum, requiring the caller to separately construct one
  per aggregate and manually call `.entity_ref(...)` per instance with an
  explicit `entity_type: &'static str` string and a hand-constructed
  `Arc<dyn PersistentEntity<...>>`. There is no single, app-wide "entity
  registry" a hypothetical `AppBuilder::entity::<E>()` could delegate to
  without first deciding what new abstraction that call would construct or
  register. This is confirmed as an open question, not assumed either way.
- `DepKey::Entity` exists in the DI enum (`crates/service-sdk/src/di/
  mod.rs` line 78) but `RuntimeInner::check_dependency` always returns
  `Err` for it (runtime_builder.rs lines 370–371) — a real, intentional,
  currently-unfillable gap (comment cites "CORE-006 is not landed").
- `RuntimeBuilder` never receives raw config sources — confirmed a frozen
  constraint (lib.rs line 11, "CORE-016 frozen constraint"). Any future
  composition facade that wants to accept a single config object and fan it
  out to `RuntimeBuilder` + N `EntityRuntimeBuilder`s + generic DI configs
  would have to preserve this "always pre-materialized" contract, not
  relax it.
- No single config type spans `RuntimeBuilder`, `EntityRuntimeBuilder`, and
  generic `with_config` DI values today — three independently-shaped config
  entry points exist in the current codebase (see #7, #15).
- `ServiceRegistry` and `InterceptorChain` internals were not independently
  investigated in this pass — flagged as gaps if Stage 1 design work needs
  their exact contracts.
- The remainder of `service-sdk-macros/src/lib.rs` (~1000 lines past what
  was read for the struct-macro expansion) — specifically the
  `#[authorize]`/`#[tenant_scoped]` trait-method guard codegen — was not
  read in full; this document does not claim complete coverage of that
  logic.

## Bootstrap error propagation summary

- `RuntimeBuilder::build()` — infallible, returns `Runtime` directly.
- `RuntimeBuilder::try_build()` — `Result<Runtime, RuntimeError>`,
  `RuntimeError` is a 2-variant `thiserror::Error` enum:
  `ServiceNotFound`, `DependencyNotFound { type_name: &'static str,
  service_name: Option<&'static str> }` (runtime_builder.rs lines 632–647).
- `RuntimeBuilder::with_service` — `Result<Self, RegistryError>`.
- `RuntimeBuilder::register_effect_executor` — `Result<Self,
  DuplicateEffectType>`.
- `RuntimeBuilder::register_data_provider` — `Result<Self,
  DuplicateProviderId>`.
- `build_runtime` (reference-app, application-defined) —
  `Result<BuiltRuntime, Box<dyn std::error::Error>>`, propagating `?` from
  `config.validate()`, `ConfigLoader` errors, `serde_json` conversion,
  `build_logger`, and `with_service`.
- No panics were found in any of the bootstrap paths read; all failure
  modes surface as typed `Result` errors.

## Shutdown resource inventory (what must close, what enforces it)

| Resource | Closed by | Enforced by |
|---|---|---|
| Logger (`KITLogger`) | Sync `TeardownStack::drain()` | `Runtime::shutdown()` / `shutdown_async()` |
| External-effects `Deferred` drain loop | `EffectRuntimeHandle::shutdown_and_wait(deadline)` | Async teardown hook registered by `Runtime::start_effects` |
| Every distinct registered `ExternalDataProvider` | `provider.shutdown().await` | Async teardown hook registered automatically by `RuntimeBuilder::build()` |
| Read-side projection poll loop (`ReadSideProjectionHandle`) | `.stop()` (consumes handle, awaits `JoinHandle`) | **Manually** registered as an async teardown hook by the host (`main.rs` line 47) — not automatic |
| HTTP server (axum via `ego_transport::serve`) | Its own graceful-shutdown drain | **Manually** sequenced by `main.rs` to run *before* `rt.shutdown_async()` |

No framework-level orchestration ties these five together automatically
today — every ordering guarantee in reference-app's shutdown sequence is
hand-sequenced in `main.rs`.
