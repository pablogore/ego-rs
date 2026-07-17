# Design: CORE-028 Stage 1 — Application Composition API (`App` / `AppBuilder`)

> Folder keeps its historical `core-026-developer-experience-refinement` label;
> the initiative is CORE-028 (see explore.md header, proposal.md). Stage 0's
> read-side spec is frozen; its read-model-ownership decision is a hard
> constraint (AD-6). This document exceeds the generic 800-word design budget
> deliberately: the orchestrator required ten evidence-cited architecture
> decisions with genuine tradeoff analysis.

## Technical Approach

A new public `app` module in `service-sdk` adds `App` + `AppBuilder` as a thin
composition facade that **delegates to `RuntimeBuilder`** and never
reimplements assembly — the same wrapping pattern testkit's `FixtureBuilder`
already ships (explore.md #13). `AppBuilder` collects registrations, `build()`
delegates to `RuntimeBuilder::build`/`try_build` (validated, starts nothing)
and constructs registered services through the existing `Injectable` contract,
and the runtime-lifecycle phase (starting effects, then the two-phase shutdown
the reference-app's `main.rs` hand-sequences today) is owned by `App` without
coupling to any transport (explore.md #10, #14). `RuntimeBuilder`, `Injectable`,
`FixtureBuilder`, and `spawn_projection` are untouched.

## Architecture Decisions

### AD-1 — `AppBuilder` is a separate facade wrapping `RuntimeBuilder`

| Option | Tradeoff |
|---|---|
| **Separate `AppBuilder` owning a `runtime_builder: RuntimeBuilder` field (chosen)** | Exactly `FixtureBuilder`'s proven shape (explore.md #13, `runtime_builder: RuntimeBuilder` at builder.rs field, "no parallel assembly"). Additive; zero risk to infra/test consumers. |
| Extend `RuntimeBuilder` directly with app methods | Forces every infra/test consumer of `RuntimeBuilder` (explore.md #1) to carry `run`/shutdown-ordering/kit-config surface they don't want; violates "no replacing/changing `RuntimeBuilder`" non-goal. |
| Separate module/plugin system | Non-goal (no module discovery/plugins); no evidence any registry supports it. |

**Decision**: separate `AppBuilder` delegating to `RuntimeBuilder`.
**Rationale**: `FixtureBuilder` is a working precedent for delegation in this
exact codebase (explore.md #13) and the precedent fits — both are thin
same-contract wrappers over one `RuntimeBuilder`, differing only in what
convenience they add (test isolation vs. composition lifecycle).

**Invariant (G3):** every successful `AppBuilder` registration MUST result in
exactly one equivalent `RuntimeBuilder` registration — no second, independent
bookkeeping structure that could drift from what `RuntimeBuilder` actually
holds. This is what "delegates, never reimplements assembly" means as a
checkable property, not just as prose, and is what protects this design
against a future refactor quietly growing a parallel assembly path.

**(G2) Explicit API boundary**: `RuntimeBuilder` remains the infrastructure
API — the public surface for framework/infra developers and tests that need
direct control. `AppBuilder` remains the application API — the public surface
for developers composing a service. Neither replaces the other (AD-10); a
consumer picks one deliberately based on which role it's in, not by accident.

**Known limitation / accepted DX debt (M2):** because `AppBuilder` is a thin
delegating facade (this AD), `.adapter()`/`.config()`/`.security()`/`.service()`
are near-1:1 wrappers over `RuntimeBuilder`'s equivalent methods. The practical
consequence is that a developer still learns `RuntimeBuilder`'s mental model
*through* `AppBuilder`, rather than `AppBuilder` offering its own independent
vocabulary — ideally that direction would be reversed. This is an accepted,
explicit tradeoff for Stage 1: the alternative (giving `AppBuilder` its own
abstractions instead of thin delegation) is exactly the "reimplement assembly"
risk the chosen option avoids, and is explicitly out of scope here (non-goals:
no DI redesign, no replacing `RuntimeBuilder`). Reducing this leakage — e.g. a
richer, `RuntimeBuilder`-independent vocabulary — is a candidate for a later
stage, not this one; not fixed now, flagged so it isn't mistaken for the
finished DX target.

### AD-2 — `build()` is separate from starting; both phases exist

`RuntimeBuilder::build()` is already infallible and starts nothing (may run
before any Tokio runtime exists — explore.md #1, builder.rs line 316-322), and
`Runtime::start_effects` is already a separate explicit, Tokio-requiring step
(explore.md #2, line 466). `AppBuilder`'s split **maps onto that existing
split, it does not invent its own**: `App::build()` → `RuntimeBuilder::build` +
`try_build` validators + `Injectable` construction (no Tokio, no tasks); the
runtime-lifecycle phase (`App::start`/`RunningApp::shutdown`, AD-6) →
`start_effects` + shutdown ownership.

- Compose-and-start in one step: rejected — the spec + proposal require
  `build()` be assertable with no Tokio runtime and no started tasks;
  `start_effects` needs Tokio.
- **Separate build vs. lifecycle phases (chosen)**: mirrors the native
  two-phase lifecycle 1:1.

### AD-3 — Service registration reuses the `Injectable` contract; construction mechanism is left to tasks

`#[service]` on a struct generates only an `Injectable` impl (explore.md #5); it
registers nothing. `with_injectable::<S>` records `S::validate` for `try_build`
and has zero construction effect (explore.md #1, line 195). `with_service::<Tag>`
registers a pre-built `Arc<Tag::Service>` for resolution and must be called
**before** `build()` (explore.md #1, line 182). `Injectable::build(rt)`
constructs `S` but needs a **built** `RuntimeInner` to resolve adapters/configs
(explore.md #4), and the registry is sealed after `build()` (explore.md #3).
These facts collide: you cannot both construct-via-`build` (needs a runtime)
and register-via-`with_service` (needs the `Arc` pre-build) in a single pass.
That collision is a real constraint the implementation must resolve; it is
evidence, **not** a decision this document makes about *how* to resolve it.

| Option | Tradeoff |
|---|---|
| Single-pass auto-construct | **Impossible** — registry sealed post-build; `with_service` is pre-build only (explore.md #1, #3). |
| New macro/trait linking `S` → its `{Trait}Tag` | Non-goal (no new macros). No type links `S` to its Tag today (explore.md #5 generates them independently). |

**Decision (observable contract only)**: `.service::<S, Tag>()` where
`S: Injectable, S: Tag::Service` records a service for construction. At
`build()`, each registered service is constructed through the **existing
`Injectable` contract** — `Injectable::validate` then `Injectable::build`
(explore.md #4, #5) — the same construction path production and testkit already
use (explore.md #13), with **no second, parallel construction path**. The
resulting service is made resolvable under its `Tag`. Missing dependencies
surface with the **same attribution `try_build` already provides**: the missing
type plus the requesting service (explore.md #1, test line 995) — satisfying
the spec's attribution requirement. **(Review F3)** This attribution applies
uniformly to a `DependencyNotFound` surfacing from either `Injectable::validate`
*or* `Injectable::build` — a hand-rolled `Injectable` with an incomplete
`dependencies()` list (so `validate()`'s presence check never catches it) but
whose `build()` still fails resolving an unregistered dependency must be
named identically to one caught by `validate()`. The observable contract does
not distinguish "caught by validate" from "caught by build"; a single shared
attribution step, applied to both error paths, is what the implementation
must guarantee (not two independently-maintained copies that could diverge).
This document commits only to that observable contract; the concrete
construction mechanism that satisfies it is deferred to the
implementation/tasks phase (see "Possible implementation approach" below).

**Known limitation / technical debt (G3):** the two-type-parameter form
`.service::<S, Tag>()` leaks an SDK-internal detail into the primary DX-facing
API. It exists **only** because today's `#[service]` macro generates a struct's
`Injectable` impl and a trait's `{Trait}Tag` independently, with nothing linking
a struct `S` to its generated `Tag` (explore.md #5). A future macro enhancement
(out of scope for this stage, non-goal: no new macros here) could expose that
link and collapse the call to `.service::<S>()` alone. This two-parameter shape
is **not** the intended long-term shape of the public API and must not silently
ossify into "the" design; it is debt to shed once the macro can carry the
binding. **(L3)** The intended long-term public API is `.service::<S>()` alone,
once macro-generated metadata can provide the service/tag binding — stated
here explicitly so the direction is unambiguous, not just implied.

**Possible implementation approach (non-binding):** one candidate mechanism is
staged construction via `RuntimeBuilder`'s `#[derive(Clone)]` — clone the
builder, `build()` a throwaway scratch `Runtime` (adapters/configs then
resolvable), run `Injectable::validate`+`build` per service against it, register
each resulting `Arc` via `with_service` on the retained builder, then
`try_build` the real runtime, requiring no `RuntimeBuilder` change. This works
**today** only because `RuntimeBuilder` is `Clone` (explore.md #1, line 67),
`build()` starts nothing and is side-effect-free (explore.md #1, line 316-322),
and `start_effects` is separately gated and idempotent (explore.md #2, line 466,
`effect_started: AtomicBool`). It is listed as a candidate, not a decision,
precisely because it couples correctness to many internal `RuntimeBuilder`/
`RuntimeInner` properties (logger construction, data-provider registry, effect
subsystem laziness) staying cheap and side-effect-free forever; a future change
to what `build()` does internally would silently invalidate it. The tasks phase
may adopt this clone-and-discard approach or replace it with another mechanism
(e.g. a dedicated construction pass) as long as the observable contract above
holds.

**FLAG (evidence-based, must reach tasks):** reference-app's `RegisterUserImpl`
is built by `RegisterUserImpl::new(...).with_read_side_sink(...)` with a
**non-DI collaborator** (explore.md #14 step 6), and `Injectable::build` fills
non-DI fields with `Default::default()` (explore.md #5). Constructing it through
`Injectable::build` alone would therefore yield a service with a defaulted,
broken sink — regardless of which construction mechanism is chosen. Migrating it
to `.service::<S, Tag>()` **requires** modeling the read-side sink as a DI
dependency (`AdapterRef`/`ConfigValue`) so `Injectable::build` resolves it.
Where a collaborator genuinely cannot be a DI dependency, a pre-constructed
escape-hatch registration (`.service_instance::<Tag>(Arc)` → straight
`with_service` + optional `with_injectable` validation) is retained; the spec's
binding requirement is the observable outcome, not the mechanism
(application-composition spec lines 97-99). **(L2)** `service_instance` exists
solely as this one escape hatch — new registration APIs MUST NOT be added
following the same pattern (no `service_factory()`, `service_lazy()`,
`service_provider()`, `service_builder()`, etc.); if a genuinely new
construction need appears, it revisits this decision rather than growing the
method surface. **(G1)** `service_instance()` SHOULD only be used when
construction cannot be expressed through the `Injectable` dependency model
(e.g. a collaborator that genuinely cannot be a DI dependency, per the
reference-app sink case above) — it is not a shortcut around declaring
dependencies, and its use should be the exception, not the default registration
path. Whether `service`/`service_instance`
should stay two named methods or collapse into one conceptual registration
point is intentionally left open — see Open Questions.

### AD-4 — Duplicate adapter registration is a fail-closed error, with an explicit replace

`with_adapter` is infallible last-write-wins by `TypeId` (explore.md #1 line
162, #6), but `with_service` in the *same builder* already fails closed on
duplicates (`RegistryError::DuplicateService`, explore.md #1 line 182).

**Decision**: `AppBuilder` tracks registered adapter `TypeId`s in its own set and
returns a `CompositionError::DuplicateAdapter { type_name }` on a second
registration of the same type, before delegating to the (still infallible)
`with_adapter`. A separate deliberate `.replace_adapter()` performs the
last-write-wins override explicitly.
**Rationale**: validates the proposal's preference against the
`DuplicateService` precedent already living in the same builder; silent
shadowing is a bootstrap footgun and is explicitly disallowed by the spec
("silent, undocumented overwrite is not an acceptable outcome"). No
`RuntimeBuilder` change — the guard lives entirely in `AppBuilder`.

**(G2)** `.replace_adapter()` SHOULD only be used during application
bootstrap/composition, not as a routine runtime operation — it exists to make
one deliberate override explicit at startup, not to normalize overriding
adapters as an everyday action.

### AD-5 — No `.entity::<E>()` (confirmed non-goal)

Explore.md found no stable entity contract: only per-aggregate
`EntityRuntimeBuilder<E>` + manual `.entity_ref(...)` per instance (explore.md
#15), and `DepKey::Entity` **always** fails `check_dependency` pending CORE-006
(explore.md #4, runtime_builder.rs line 370). There is nothing app-wide for
`.entity::<E>()` to delegate to without first inventing an abstraction — which
is out of scope.
**Decision**: defer `.entity()`. The proposal's non-goal stance is correct on
the evidence; no fictional entity abstraction is designed.

### AD-6 — `App` administers the runtime lifecycle only; the read model stays application-owned

Today `main.rs` hand-sequences: transport graceful drain first, then
`rt.shutdown_async()` (async hooks in order, then sync `TeardownStack`),
including the read-side handle's `stop` registered via
`register_async_teardown` (explore.md #10, #12, #14 line 47).

`App` is the **runtime** composition root, not a transport composition root.
Non-HTTP hosts (CLI tools, cron jobs, Kafka consumers, Temporal workers,
Lambda-style single-invocation handlers) have no "serve future" to hand in, so
an API shaped around one would implicitly design `App` around reference-app's
HTTP shape. `App` therefore owns the runtime lifecycle **without receiving or
awaiting any transport-specific future**.

**Decision**: split the lifecycle into two operations the host sequences around
its own workload:

- `App::start(self) -> Result<RunningApp, CompositionError>` — starts effects
  (`start_effects`, explore.md #2 line 466; requires an active Tokio runtime),
  returns a `RunningApp` handle. Starts nothing else and owns no transport.
- `RunningApp::shutdown(self) -> Result<(), CompositionError>` — runs the
  existing shutdown ordering: async hooks in **registration order**, then the
  sync `TeardownStack`, surfacing the **first** error after every participant
  has had the chance to run — matching existing `shutdown_async` behavior
  (explore.md #10, test line 1140). This shutdown-ordering policy is unchanged
  and as strict as before.

The host is responsible for its own middle-phase sequencing. For an HTTP host:

```rust
let running = app.start().await?;
ego_transport::serve(listener, router, shutdown_signal()).await?; // host owns transport + drain
running.shutdown().await?;
```

A non-HTTP host substitutes a completely different middle step (a CLI
run-to-completion, a consumer poll loop, a single Lambda invocation) between
`start()` and `shutdown()`.

- **Transport stays application-owned** (router built by the app; non-goal: no
  declarative routing). `App` installs no signal handlers, owns no router, and
  never awaits a serve future — the host owns transport startup, graceful drain,
  and signal handling entirely, preserving testability and avoiding hidden
  global signal ownership and any dependency-inversion coupling to an HTTP shape.
- **Read model / read-side (hard constraint, stage-0 spec):** the app spawns
  its own projection; `spawn_projection` still returns only a
  `ReadSideProjectionHandle` (explore.md #12). The app hands that handle's
  `stop` future to `App::register_shutdown(stop_future)` before `start()`,
  which registers it via the existing `register_async_teardown` (explore.md #1
  line 589, stage-0 ownership preserved). `App` tracks the handle **for
  shutdown timing only**; it never wraps, returns, or re-owns the read model.

**(M1)** This hand-off is named `register_shutdown`, not `with_background` —
not every shutdown participant is a background task (a scheduler, a metrics
pusher, a Kafka consumer, a Temporal worker, a NATS subscription are not
"background work" in the same sense a spawned poll loop is; the one thing they
all share is "something that knows how to shut down"). `register_shutdown`
names that shared contract instead of one implementation's shape, and mirrors
the existing `register_async_teardown` naming it wraps.

**FLAG:** the exact identifiers — the `RunningApp` type name, the
`start`/`shutdown` method names, whether `start` consumes `App` into a
distinct handle type or returns `Self`, **and (G1) the literal spelling of the
shutdown-participant hand-off** — are ergonomic details for spec/tasks. What's
settled (M1) is the *rationale*: it must name the shared "knows how to shut
down" contract, not one implementation's shape (`with_background` is rejected
for that reason). `register_shutdown` is the current working name and
satisfies that rationale, but alternatives that describe the same intent more
clearly (e.g. `register_shutdown_hook`, `register_lifecycle`,
`register_runtime_component`) remain open for tasks-phase bikeshedding — this
is a naming choice, not an architecture decision, so it doesn't block
implementation. All of this preserves read-model ownership and the decoupling
from transport. Explore.md is sufficient on ordering and on the transport
decoupling, insufficient to fix the remaining identifiers.

### AD-7 — Dependency validation at `build()`; duplicates at registration; cycles are unexpressible

`try_build` runs every recorded validator against the built runtime and fails
fast on the first missing dependency, naming type + service (explore.md #1 line
391, test line 995).

**Decision**: missing dependencies and incompatible providers are detected at
`build()` (delegating to `try_build`). Duplicate adapters are detected at
registration (AD-4); duplicate services at registration via `with_service`'s
`DuplicateService` (explore.md #1). **Cycles are not a concern by
construction**: `DepKey` has only `Entity/Projection/Adapter/Config` variants
(explore.md #4) — there is no `Service` dependency edge, so services cannot
depend on services and no DI cycle is expressible. Adapters/configs/projections
are leaf values. No cycle detector is added (YAGNI, evidence-backed).

### AD-8 — `CompositionError` wraps existing typed errors and names the failing component

Existing bootstrap errors (explore.md #1, "Bootstrap error propagation"):
`RuntimeError` (2-variant, `DependencyNotFound` already carries `type_name` +
`service_name`), `RegistryError`, `DuplicateEffectType`, `DuplicateProviderId`,
`RuntimeInfraError`.

**Decision**: a new public `CompositionError` **wraps** (never replaces) these
via `#[from]`, one variant per phase/component so errors are distinguishable by
phase (composition / initialization / execution / shutdown) and each names the
failing component:

```
CompositionError::DuplicateAdapter { type_name }        // registration
CompositionError::Service(RegistryError)                // registration
CompositionError::Validation(RuntimeError)              // build (carries type+service)
CompositionError::EffectExecutor(DuplicateEffectType)   // registration
CompositionError::DataProvider(DuplicateProviderId)     // registration
CompositionError::Logger(RuntimeInfraError)             // init (kit-config→logger)
CompositionError::Startup(RuntimeInfraError)            // start (start_effects)
CompositionError::Shutdown(RuntimeInfraError)           // shutdown
```

**Rationale**: wrapping (not replacing) keeps the existing typed errors intact
for infra/test consumers still on `RuntimeBuilder`, and reuses
`DependencyNotFound`'s existing type+service attribution so a `TypeId`-only or
opaque-string error never reaches the developer.

**Invariant (L1):** a `CompositionError` variant MUST wrap one of the existing
typed errors above (or a plain field like `type_name`) — never another
`CompositionError`. Exactly one layer of wrapping, always. This keeps the
error flat and inspectable and prevents it from growing into a nested
error tree as more variants are added.

### AD-9 — Testkit keeps wrapping `RuntimeBuilder`, not `App`

`FixtureBuilder` wraps a real `RuntimeBuilder` (explore.md #13) and
`ServiceTestFixture::service::<S>()` calls `S::build(runtime.inner())` directly
— the identical `Injectable::build` path production uses (explore.md #13 line
80).

**Decision**: testkit stays wrapping `RuntimeBuilder`; it does **not** wrap
`App`. Both `AppBuilder` and `FixtureBuilder` sit over the **same**
`RuntimeBuilder` → one construction path, no second DI path (spec's hard
requirement).
**Rationale**: making `FixtureBuilder` wrap `App` would drag the runtime
lifecycle (`start`/`shutdown`), transport, and shutdown-ordering concerns into
synchronous tests that only want to construct and assert; it would also invert
layering (fixtures are lower-level). `App::build()` is independently assertable
for composition-level tests (proposal success criterion), and adapter
substitution flows through the existing fixture path (spec scenario), so no new
test-construction path appears.

### AD-10 — `RuntimeBuilder` stays public and supported; migration is optional

`RuntimeBuilder` is the public infra/test path and reference-app hand-wires it
today (explore.md #1, #14); `FixtureBuilder` also depends on it.

**Decision**: `RuntimeBuilder` remains fully public and supported (non-goal:
no replacing it). `App` is purely additive (proposal rollback plan). Migration
is **optional, not forced**: reference-app migrates to `App` as proof, but the
direct `RuntimeBuilder` + manual-sequencing path keeps working unchanged and is
the documented escape hatch for hosts whose lifecycle needs
`App::start()`/`RunningApp::shutdown()` doesn't fit (proposal risk mitigation).

**Invariant (G3):** existing `RuntimeBuilder` consumers MUST remain
source-compatible after `App` is introduced — no signature, behavior, or
public-contract change to `RuntimeBuilder` is permitted as part of this stage.
This is what "purely additive" means in practice, stated as a hard constraint
rather than only implied by the rollback plan.

## Data Flow

    AppBuilder ──.service/.adapter/.config/.security──▶ (collect + dup-guard)
        │
        │ App::build()   (no Tokio; starts nothing)
        ▼
    each registered service constructed via Injectable::validate + Injectable::build
        │   (existing DI contract — AD-3; missing dep → type + requesting service)
        ▼
    services made resolvable under their Tag; runtime validated (try_build)
        │
        ▼
      App ── App::start() ──▶ RunningApp        (start_effects; requires Tokio)
                                 │
              host runs its OWN workload here (App owns no transport):
              HTTP → ego_transport::serve(...).await   (host drains in-flight)
              non-HTTP → CLI run / consumer loop / single invocation
                                 │
                                 ▼
                    RunningApp::shutdown() ──▶ async hooks (registration order)
                                               → sync TeardownStack
                                               → first error surfaces

    (AD-3's construction mechanism — e.g. a scratch-runtime clone — is a
     non-binding implementation choice; only the Injectable contract above is
     committed.)

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/service-sdk/src/app/mod.rs` | Create | `App`, `AppBuilder`, `RunningApp`; `build()` + `start()`/`shutdown()` lifecycle |
| `crates/service-sdk/src/app/error.rs` | Create | `CompositionError` (AD-8), wrapping existing typed errors |
| `crates/service-sdk/src/lib.rs` | Modify | Publicly re-export the `app` module |
| `crates/service-sdk/src/runtime/builder.rs` | Unchanged | Delegation target only (AD-1, AD-10) |
| `examples/reference-app/src/lib.rs` | Modify | `build_runtime` → `App` composition (proof; AD-3 flag applies to the sink) |
| `examples/reference-app/src/main.rs` | Modify | Hand-sequenced shutdown → `App::start()` / `RunningApp::shutdown()`; host still owns transport serve/drain between them (AD-6) |
| `crates/testkit/**` | Unchanged | Keeps wrapping `RuntimeBuilder` (AD-9) |

## Interfaces / Contracts

```rust
impl App {
    pub fn builder() -> AppBuilder;
}
impl AppBuilder {
    // Two type params AND the trailing coercion closure are required today
    // only because #[service] does not link S to its Tag — see AD-3 "Known
    // limitation / technical debt", formally accepted as interim DX debt
    // after review F2 (not a silent implementation detail); future macro
    // work could collapse this to `.service::<S>()` alone.
    pub fn service<S, Tag>(self, to_trait_object: fn(Arc<S>) -> Arc<Tag::Service>) -> Self
        where S: Injectable, Tag: Resolvable;
    pub fn service_instance<Tag: Resolvable>(self, svc: Arc<Tag::Service>) -> Self; // escape hatch, AD-3 flag
    pub fn adapter<A: Send + Sync + 'static>(self, a: Arc<A>) -> Self;      // dup-guarded, AD-4
    pub fn replace_adapter<A: Send + Sync + 'static>(self, a: Arc<A>) -> Self;
    pub fn config<C: Send + Sync + 'static>(self, c: Arc<C>) -> Self;
    pub fn logger(self, logger: Arc<KITLogger>) -> Self;                    // thin delegation over an already-built logger — the host still runs the kit-config pipeline (scope correction, review F1's logger gap)
    pub fn security(self, authn: Arc<dyn AuthenticationProvider>, authz: Arc<dyn AuthorizationProvider>) -> Self;
    // Added post-review (F1): CompositionError already had EffectExecutor/
    // DataProvider variants and App::start() already claimed to start
    // effects, but nothing in AppBuilder could register one — these close
    // that gap with the same thin-delegation shape as everything above.
    pub fn observability(self, obs: Arc<dyn Observability>) -> Self;
    pub fn effect_executor(self, effect_types: impl IntoIterator<Item = impl Into<String>>, executor: Arc<dyn ExternalEffectExecutor>) -> Self; // fails closed via CompositionError::EffectExecutor
    pub fn data_provider(self, provider_id: impl Into<String>, provider: Arc<dyn ExternalDataProvider>) -> Self; // fails closed via CompositionError::DataProvider
    pub fn build(self) -> Result<App, CompositionError>;                    // no Tokio, starts nothing
}
impl App {
    // A constructed-but-not-started App is directly resolvable/assertable
    // (spec: "An Application Is Testable Without Running"). Added post-review
    // (F4): only `resolve<Tag>` existed before; an external caller had no
    // public way to check an adapter/config was registered without reaching
    // into the private `runtime` field.
    pub fn resolve<Tag: Resolvable>(&self) -> Result<Tag::Proxy, RuntimeError>;
    pub fn resolve_adapter<A: Send + Sync + 'static>(&self) -> Result<AdapterRef<A>, RuntimeError>;
    pub fn resolve_config<C: Send + Sync + 'static>(&self) -> Result<ConfigValue<C>, RuntimeError>;
    // Found during Phase 5 (reference-app migration, PR2): ego_transport::AppState
    // predates App/AppBuilder and needs a raw Runtime handle for its own generic
    // per-request resolve::<Tag>() dispatch — a legitimate integration seam, not
    // a reach into composition internals. Cheap: Runtime is now Clone (wraps only
    // Arc<RuntimeInner>). Callable pre-start() like resolve_adapter/resolve_config.
    pub fn runtime(&self) -> Runtime;
    // App owns the RUNTIME lifecycle only. It receives NO transport/serve future
    // and awaits none (AD-6). Requires an active Tokio runtime (start_effects).
    pub async fn start(self) -> Result<RunningApp, CompositionError>;       // starts effects
}
impl RunningApp {
    // Existing shutdown ordering: async hooks (registration order) → sync
    // TeardownStack → first error surfaces (explore.md #10, test line 1140).
    pub async fn shutdown(self) -> Result<(), CompositionError>;
}
// Host sequences its own transport/workload between start() and shutdown().
// Identifiers (RunningApp, start/shutdown) are non-binding — see Open Questions.
```

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit | Dup-adapter error + `replace_adapter` (AD-4); `CompositionError` wrapping preserves type+service (AD-8) | Direct `AppBuilder` calls, assert error variants |
| Unit | `build()` starts nothing / no Tokio (AD-2/AD-3) | Build without a Tokio runtime; assert no acceptor started (`effect_acceptor()` is `None`) |
| Integration | Registered service resolves via `Injectable`; missing dep names type+requester (AD-3/AD-7) | `App::build()` then `resolve`; assert `try_build`-equivalent attribution |
| Integration | Same-contract: `App` and `FixtureBuilder` construct via one path (AD-9) | Mirror existing testkit identity tests (explore.md #13 lines 277-316) |
| Integration | `RunningApp::shutdown()` runs async hooks in registration order then sync stack, and surfaces the first error (AD-6) | `start()` then `shutdown()` with two participants, one failing; assert both ran and first error surfaced (mirror explore.md #10 test line 1140) |
| E2E | Reference-app composes via `App`; `App` owns runtime lifecycle while host sequences transport (AD-6) | Migrate reference-app; assert `start()` → host serve/drain → `shutdown()`; App owns no transport future |

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file
classification, or process-integration boundary. `App`'s lifecycle sequences
in-process async startup/shutdown only; it spawns no OS processes, awaits no
transport future, and executes no shell.

## Migration / Rollout

Purely additive (proposal rollback plan). `RuntimeBuilder` migration is optional
(AD-10). Reference-app migrates as proof: its composition moves to `App::build()`
and its `main.rs` shutdown moves to `App::start()` → host-owned transport
serve/drain → `RunningApp::shutdown()` (the host keeps owning the transport,
AD-6). Its `RegisterUserImpl` read-side sink must become a DI dependency or use
the `service_instance` escape hatch (AD-3 flag). No data migration, no feature
flag.

## Open Questions

- [ ] Final identifiers for the runtime-lifecycle split (`RunningApp` type name,
      `start`/`shutdown` method names, whether `start` consumes `App` into a
      distinct handle or returns `Self`) — AD-6 flag; the decoupling from
      transport is decided, the exact names are tasks-phase bikeshedding.
- [x] Shutdown-participant hand-off *rationale* — resolved (M1): must name the
      shared "knows how to shut down" contract, not one implementation's shape
      (`with_background` rejected on those grounds); wraps the existing
      `register_async_teardown`; preserves read-model ownership.
- [ ] Shutdown-participant hand-off *literal name* — open (G1): currently
      `register_shutdown`, still describes the mechanism more than the intent;
      alternatives (`register_shutdown_hook`, `register_lifecycle`,
      `register_runtime_component`) are tasks-phase bikeshedding, not an
      architecture decision.
- [ ] AD-3 construction mechanism (scratch-runtime clone-and-discard vs. a
      dedicated construction pass vs. another approach) — deferred to tasks; only
      the `Injectable` observable contract is committed here.
- [ ] Whether reference-app's read-side sink is modeled as a DI dependency
      (preferred, unblocks `.service::<S,Tag>()`) or uses `service_instance`
      (AD-3 flag). Decision belongs to the reference-app migration task.
- [ ] `ServiceRegistry`/`InterceptorChain` internals were not independently
      verified (explore.md open questions) — confirm no hidden constraint on the
      chosen AD-3 construction mechanism before implementation.
- [ ] `.service::<S, Tag>()` and `.service_instance::<Tag>(Arc<_>)` currently
      read as two separate mental models for what is conceptually one action
      (registering a service) — the only real difference is who constructs it
      (the framework vs. a pre-built instance). Not changed in this design;
      tasks/implementation should re-evaluate whether the escape hatch can be
      expressed as one conceptual registration point rather than a second
      named method, or confirm `service_instance` is genuinely the cleanest
      option once real usage (e.g. the reference-app read-side sink, AD-3
      flag) is in hand. Flagged now specifically so this doesn't silently grow
      into `service_factory()`/`service_lazy()`-style API fragmentation later.

## Future Direction (non-blocking, not this stage)

**(O1) Vocabulary review at Stage 2/3.** `AppBuilder`'s public surface is
already seven operations (`service`, `service_instance`, `adapter`,
`replace_adapter`, `config`, `security`, `register_shutdown`). Each is
individually justified (AD-3/AD-4/AD-6) and today's size is acceptable
precisely because nearly all of them are thin `RuntimeBuilder` wrappers
(AD-1's accepted DX debt, M2). This is not a Stage 1 gap and nothing changes
now — but this surface should not be allowed to grow indefinitely as later
stages add capabilities (e.g. a future `.entity::<E>()`, AD-5). When Stage 2
or Stage 3 is scoped, revisit whether the vocabulary can be simplified or
consolidated before adding more top-level `AppBuilder` methods.
