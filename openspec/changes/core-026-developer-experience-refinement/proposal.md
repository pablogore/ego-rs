# Proposal: CORE-028 Stage 1 — Application Composition API (`App` / `AppBuilder`)

> Folder is historically labeled `core-026-developer-experience-refinement`;
> the initiative is CORE-028 (see explore.md header). Folder intentionally not
> renamed. Stage 0 (`spawn_projection`, specs/read-side/spec.md) is shipped
> and untouched by this stage.

## Intent

The infrastructure exists; composing it doesn't. Standing up a real Ego
service (explore.md #14, reference-app `build_runtime` + `main.rs`) still
means hand-writing: (a) authn/authz provider construction, (b) the
kit-config → `ConfigurationProvider` → `build_logger` pipeline, (c) one
`EntityRuntimeBuilder` per aggregate, (d) read-side wiring + manual spawn +
manual teardown registration, (e) `Arc::new(rt)` + two-phase shutdown
sequencing. Nothing orchestrates shutdown ordering today (explore.md #10) —
every host re-derives it. Stage 1 gives **application developers** (composition-root
authors) a single entry point; **framework/infra developers and tests keep
using `RuntimeBuilder` directly**, unchanged.

Illustrative only (final surface is spec/design work):

```rust
let app = App::builder()
    .service::<RegisterUserImpl, RegisterUserTag>()
    .adapter(postgres)
    .config(app_config)
    .security(authn, authz)
    .build()?;                // validates + constructs, starts nothing

let running = app.start().await?;   // starts effects; App owns no transport
// host owns transport serve/drain here, e.g. ego_transport::serve(...).await?;
running.shutdown().await?;          // owns the two-phase shutdown ordering
```

`.service::<S, Tag>()` is illustrative only; the two-parameter form reflects
design.md's resolved observable contract (AD-3) — future macro work may
collapse this to `.service::<S>()` alone once `#[service]` can carry the
service/tag binding itself (see AD-3 "Known limitation / technical debt").

## Scope

### In Scope
- New public `App` + `AppBuilder` in `service-sdk`, a thin facade **delegating
  to `RuntimeBuilder` internally** — the same wrapping pattern testkit's
  `FixtureBuilder` already proves (explore.md #13, "same-contract principle").
  No parallel assembly path.
- `.service()`, `.adapter()`, `.config()`, `.security()` mapping onto existing
  `with_injectable`/`with_service`, `with_adapter`, `with_config`,
  `with_security`.
- `build()` vs `start()`/`shutdown()` split, grounded in what exists: `build()`
  validates and constructs without starting anything (per infallible
  `RuntimeBuilder::build` + `try_build` validators); `App::start()` calls the
  already-separate `Runtime::start_effects`; `RunningApp::shutdown()` takes
  over the shutdown ordering `main.rs` hand-sequences today (async hooks →
  sync `TeardownStack`). The host still owns transport serve/drain, sequencing
  it between `start()` and `shutdown()` — `App` receives and awaits no
  transport future.
- App absorbing the kit-config → logger pipeline (boilerplate b), while
  preserving the CORE-016 frozen constraint: `RuntimeBuilder` still receives
  only pre-materialized values.
- A composition error type distinct from `RuntimeError`/`RegistryError`,
  aggregating today's builder-time `Result`s (`RegistryError`,
  `DuplicateEffectType`, `DuplicateProviderId`, `try_build`'s `RuntimeError`,
  logger `RuntimeInfraError`).
- Reference-app migrated to `App` as proof (implementation-level; its spec
  encodes no wiring requirements).

### Out of Scope (non-goals)
- **`.entity::<E>()`** — no stable entity contract exists to delegate to
  (per-aggregate `EntityRuntimeBuilder<E>` + manual `entity_ref` only;
  `DepKey::Entity` intentionally always fails validation until CORE-006 —
  explore.md #4, #15). Future direction, not this slice.
- Unifying the three config entry points (`with_config`, kit-config subtree,
  `EntityRuntimeBuilder::from_value`) into one config object.
- Replacing or changing `RuntimeBuilder`; DI redesign; new macros; HTTP/gRPC
  declarative routing; hot reload; plugins; module discovery;
  convention-over-configuration; read-side DSL.
- Framework-owned read models or `spawn_projection` handle ownership changes —
  stage 0's ownership split stands.

## Capabilities

### New Capabilities
- `application-composition`: single-entry-point composition facade (`App`/
  `AppBuilder`) — registration, validated non-starting build, run-time
  startup, and framework-owned shutdown ordering.

### Modified Capabilities
- None. `service-sdk`, `read-side`, `testkit`, `reference-service` spec
  requirements are unchanged; `RuntimeBuilder` behavior is untouched.

## Approach — settled vs. open

| Question | Position | Status |
|---|---|---|
| User of the API | App developers; `RuntimeBuilder` remains the public infra/test path | Settled |
| `AppBuilder` ↔ `RuntimeBuilder` | Delegation, never reimplementation (FixtureBuilder precedent) | Settled direction |
| `.service::<S, Tag>()` meaning | Construction through the existing `Injectable` contract (`Injectable::validate`+`build`), registered resolvable under `Tag` — the same construction path production/testkit already use (explore.md #4–#5, #13). The two-parameter form is required only because `#[service]` doesn't link `S` to its generated `Tag` today; flagged in design.md as technical debt, not the intended long-term shape (`.service::<S>()` alone, once macro metadata allows it) | Settled per design.md (AD-3); construction mechanism itself left to tasks |
| `.adapter()` duplicates | Delegates to `with_adapter`; whether App surfaces duplicates instead of silent last-write-wins | **Open → design.md** |
| `.security()` | Pass-through of pre-constructed providers, both-or-nothing preserved; provider *construction* stays application code | Settled |
| build/start/shutdown split | Build never starts tasks; `start()` starts effects, `shutdown()` owns the async-hooks-then-sync-stack ordering; `App` owns no transport future | Settled per design.md; exact identifiers (`RunningApp`, method names) **open → tasks.md** |
| Shutdown ownership | `RunningApp::shutdown()` owns the ordering hosts hand-sequence today; `App` owns no transport — the host sequences transport serve/drain between `start()` and `shutdown()`; read-side handle stays app-spawned, its stop registered via existing `register_async_teardown` | Outcome settled per design.md; hand-off ergonomics **open → tasks.md** |
| Error model | New composition error, distinct from runtime errors | Settled; shape **open → design.md** |
| Testing without running | testkit `FixtureBuilder` stays the fixture path (no second DI path); `App::build()` itself is assertable without starting | Settled |

## Affected Areas

| Area | Impact | Description |
|---|---|---|
| `crates/service-sdk/src/` (new `app` module) | New | `App`, `AppBuilder`, composition error |
| `examples/reference-app/src/{lib.rs,main.rs}` | Modified | `build_runtime`/`main.rs` migrate to `App` |
| `crates/service-sdk/src/runtime/builder.rs` | Unchanged | Delegation target only |
| `crates/testkit/` | Unchanged | Keeps wrapping `RuntimeBuilder` |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| App layer drifts into parallel assembly duplicating `RuntimeBuilder` | Med | Delegate-only rule; same-contract tests as testkit uses |
| Framework-owned shutdown ordering doesn't fit some hosts | Med | `RuntimeBuilder` + manual sequencing remains fully supported escape hatch |
| Scope creep toward entity/config unification | Med | Explicit non-goals above; `.entity()` deferred until an entity contract exists |

## Rollback Plan

Purely additive: delete the `app` module and revert reference-app to its
current `build_runtime` (which keeps working throughout, since `AppBuilder`
only delegates). No `RuntimeBuilder`, runtime, or stage-0 surface changes to
unwind.

## Dependencies

- None hard. `.entity()` future work depends on CORE-006 (entity table).

## Success Criteria

- [ ] Reference-app composes via `App`; boilerplate items (a-pass-through, b,
      d-teardown, e) disappear from `build_runtime`/`main.rs`.
- [ ] `App::build()` constructs + validates with no Tokio runtime and no
      started tasks; existing `RuntimeBuilder` tests pass unchanged.
- [ ] Shutdown ordering (async hooks → sync stack) is framework-executed via
      `RunningApp::shutdown()`, not hand-sequenced in `main.rs`; the host
      continues to own transport serve/drain between `start()` and
      `shutdown()`.
- [ ] Composition failures surface through the new error type, not
      `RuntimeError`/panics.
