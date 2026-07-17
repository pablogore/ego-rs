# Tasks: CORE-028 Stage 1 — Application Composition API (`App`/`AppBuilder`)

> Folder keeps its historical `core-026-developer-experience-refinement` label; initiative is CORE-028.

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~650-850 (new `app/mod.rs` + `app/error.rs` ~350-450, tests ~200-250, reference-app migration ~150) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (service-sdk `app` module + tests) → PR 2 (reference-app migration + e2e) |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | `CompositionError` + `AppBuilder` registration/build (Phases 1-2) | PR 1 | `cargo test -p ego-service-sdk app::` | N/A — sync unit tests, no Tokio | delete `crates/service-sdk/src/app/{mod,error}.rs`, revert `lib.rs` re-export |
| 2 | `App::start`/`RunningApp::shutdown` lifecycle (Phase 3) | PR 1 or 1b | `cargo test -p ego-service-sdk app::` | `#[tokio::test]` start/shutdown scenarios | same file boundary as Unit 1 |
| 3 | lib.rs wiring + AD-9 same-contract test (Phase 4) | PR 1 | `cargo test -p ego-service-sdk` | N/A | revert `lib.rs` `pub mod app;` line |
| 4 | Reference-app migration (Phase 5) | PR 2 | `cargo test -p reference-app` | `cargo run -p reference-app` boot + `tests/e2e_register.rs` | revert `lib.rs`/`main.rs`/`application.rs` to `build_runtime` (App module is additive, no dependency to unwind) |

## Phase 1: `CompositionError` + `AppBuilder` skeleton

- [x] 1.1 RED — `crates/service-sdk/src/app/error.rs`: test `CompositionError::DuplicateAdapter` carries `type_name`; test `CompositionError::Validation` wraps `RuntimeError` preserving type+service (AD-8).
- [x] 1.2 GREEN — implement `CompositionError` (thiserror, `#[from]` per AD-8's 8 variants).
- [x] 1.3 `crates/service-sdk/src/app/mod.rs`: declare `App`, `AppBuilder`, `RunningApp` types. **Resolves open question**: pin identifiers `RunningApp` (new struct, not `App` reused), `start`/`shutdown` method names, `start` consumes `App` (per AD-6 interface sketch) — document the decision in the module doc comment.
- [x] 1.4 RED — test registering an adapter twice for the same type returns `CompositionError::DuplicateAdapter` (spec "Duplicate adapter registration has one documented, testable outcome").
- [x] 1.5 GREEN — `AppBuilder { runtime_builder: RuntimeBuilder, adapter_types: HashSet<TypeId> }`; `.adapter()` dup-guards then delegates to `with_adapter`; `.replace_adapter()` bypasses the guard (AD-4).
- [x] 1.6 RED — test `.config()`/`.security()` pass-through resolve after `build()` (spec "A registered config value is resolvable", "Security providers are both-or-nothing").
- [x] 1.7 GREEN — implement `.config()`, `.security()` as thin delegation to `RuntimeBuilder`.
- [x] 1.7b GAP FOUND DURING PR1 REVIEW — `.logger(Arc<KITLogger>)` was missing (proposal.md/spec.md require config/security/logging/observability to integrate via existing abstractions; design.md's Interfaces/Contracts sketch had silently dropped it). Added as the same thin-delegation pattern over `RuntimeBuilder::with_logger`, with a test (`registered_logger_is_present_on_the_built_runtime`), before this PR's commit.
- [x] 1.8 RED — test `App::build()` succeeds with no active Tokio runtime and no effect acceptor started (spec "Constructing an application starts nothing").
- [x] 1.9 GREEN — `build(self) -> Result<App, CompositionError>` delegates to `RuntimeBuilder::try_build` (AD-2, AD-7); wraps `RuntimeError` as `CompositionError::Validation`.

## Phase 2: Service registration via `Injectable` (AD-3)

- [x] 2.1 Spike (no code): read `ServiceRegistry`/`InterceptorChain` internals to confirm no hidden constraint blocks the scratch-runtime clone-and-discard mechanism. **Resolves open question** (explore.md gap) — record the confirmation (or the blocking finding + chosen alternative) in `app/mod.rs` doc comment before 2.3. **Finding**: no hidden constraint — both are plain in-memory structures with no lazy init/global state/I-O; `RuntimeBuilder::build()` never spawns a Tokio task. Recorded in `app/mod.rs` module doc.
- [x] 2.2 RED — test `.service::<S, Tag>()` with satisfied deps resolves via `Tag` after `build()` (spec "A registered service with satisfied dependencies resolves").
- [x] 2.3 GREEN — implement `.service::<S, Tag>()` + chosen AD-3 construction mechanism (clone-and-discard scratch runtime) inside `build()`; construct via `Injectable::validate` + `Injectable::build`, register resulting `Arc` via `with_service`. **Deviation**: `.service()` takes an extra `fn(Arc<S>) -> Arc<Tag::Service>` coercion parameter — design.md's `S: Tag::Service` bound is not valid Rust (confirmed via isolated `rustc` repro, E0405); documented in `app/mod.rs`.
- [x] 2.4 RED — test a missing dependency surfaces the missing type + requesting service name (spec "A missing dependency names both the missing type and the requester").
- [x] 2.5 GREEN — ensure `CompositionError::Validation`/`Service` wraps carry that attribution (reuse `try_build`'s existing `DependencyNotFound`).
- [x] 2.6 RED — test `.service_instance::<Tag>(Arc<_>)` registers a pre-built instance resolvable under `Tag` (AD-3 escape hatch).
- [x] 2.7 GREEN — implement `.service_instance()` as `with_service` + optional `with_injectable` validation. **Resolves open question**: keep `.service()`/`.service_instance()` as two documented methods (not collapsed); record this decision plus the re-evaluation trigger ("revisit if a third escape hatch appears") in the doc comment — do not let it silently grow into `service_factory()`/`service_lazy()`.

## Phase 3: Runtime lifecycle

- [x] 3.1 RED — `#[tokio::test]`: `App::start()` starts effects (`effect_acceptor()` is `Some` post-start when an executor was registered).
- [x] 3.2 GREEN — `App::start(self) -> Result<RunningApp, CompositionError>` calls `start_effects`, wraps error as `CompositionError::Startup`.
- [x] 3.3 RED — test a registered shutdown-participant stop future runs during shutdown and the app's read-model reference is unchanged (spec "The application's read model is unaffected by lifecycle integration").
- [x] 3.4 GREEN — implements design.md's resolved M1 naming: `App::register_shutdown(impl Future)` (on the built `App`, before `start()` — not on `AppBuilder`), delegating to `register_async_teardown`. Named for the shared "knows how to shut down" contract (schedulers, consumers, pushers), not `with_background`'s implied background-task shape — no new registry.
- [x] 3.5 RED — mirror `shutdown_async_runs_every_hook_even_after_an_earlier_one_fails` (builder.rs:1140): two shutdown participants, one fails; both run, first error surfaces (spec "One failing shutdown participant does not hide others").
- [x] 3.6 GREEN — `RunningApp::shutdown(self) -> Result<(), CompositionError>` delegates to `shutdown_async`, wraps error as `CompositionError::Shutdown`.

## Phase 4: Wiring + same-contract proof

- [x] 4.1 `crates/service-sdk/src/lib.rs`: add `pub mod app;` + re-export `App`/`AppBuilder`/`RunningApp`/`CompositionError`.
- [x] 4.2 RED/GREEN — integration test mirroring `fixtures.rs:304 macro_generated_service_resolves_config_identically_to_hand_rolled_case`: assert `App`-constructed and `FixtureBuilder`-constructed instances of the same `#[service]` struct resolve identically (AD-9, spec "A test substitutes an adapter through the existing fixture path").
- [x] 4.3 Regression check (design.md AD-10 invariant): run the existing `crates/service-sdk/src/runtime/builder.rs` test suite unchanged (`cargo test -p ego-service-sdk runtime::`) and confirm zero modifications were needed to any `RuntimeBuilder` test or source line to introduce `App` — a cheap, standing proof that `RuntimeBuilder` consumers remain source-compatible. **Result**: 117 passed, 0 failed, zero modifications.
- [x] 4.4 Same-contract test (design.md AD-10, review G4): construct one equivalent application two ways — once via `RuntimeBuilder::new()...with_service(...)...build()` directly, once via `App::builder().service::<S, Tag>()...build()` — and assert both resolve the identical registered services under the same `Tag`s. Makes the "optional migration, same contract" principle a permanent, checkable test rather than only a documented claim.

## Phase 5: Reference-app migration (proof)

- [ ] 5.1 `examples/reference-app/src/application.rs`: **resolves open question** (AD-3 FLAG) — model `RegisterUserImpl`'s `read_side_sink` as a DI dependency (`AdapterRef<ReadSideSink>`) if feasible under 2.1's confirmed mechanism; otherwise register `RegisterUserImpl` via `.service_instance::<RegisterUserTag>()` and document why DI modeling was rejected. Decide and record inline.
- [ ] 5.2 RED — extend `examples/reference-app/tests/e2e_register.rs` (or add a composition-level test) asserting the app boots via `App::builder()...build()` and a register-user request still succeeds end-to-end.
- [ ] 5.3 GREEN — `examples/reference-app/src/lib.rs`: migrate `build_runtime` to `App::builder()` composition (security, config, logger pipeline, adapters/services per 5.1).
- [ ] 5.4 GREEN — `examples/reference-app/src/main.rs`: replace hand-sequenced `Arc::new(rt)` + `shutdown_async` with `App::start()` → `ego_transport::serve(...)` (unchanged, host-owned) → `RunningApp::shutdown()`.
- [ ] 5.5 Run full existing reference-app suite (`cargo test -p reference-app`) to confirm no regression in `e2e_register.rs`, `register_user_guard_chain.rs`, `effects_e2e.rs`, `providers_e2e.rs`.

## Threat Matrix

N/A — no routing/shell/subprocess/process-integration boundary (design.md).
