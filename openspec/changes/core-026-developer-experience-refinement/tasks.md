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

## Phase 4b: PR1 review round — contract gaps (REQUEST CHANGES, F1-F4)

- [x] 4b.1 (F1, HIGH) `AppBuilder::observability()`/`effect_executor()`/`data_provider()` added — thin pass-throughs to `RuntimeBuilder::with_observability`/`register_effect_executor`/`register_data_provider`. `.effect_executor()`'s test (`start_starts_effects_when_an_executor_was_registered`) now builds through the public `AppBuilder` API end-to-end instead of constructing `App { runtime }` directly (private-field white-box access). Duplicate `effect_type`/`provider_id` fail closed via new `CompositionError` variants that already existed but had no producer.
- [x] 4b.2 (F2, HIGH) Formalized, not silently accepted: the `.service::<S, Tag>(to_trait_object)` coercion closure is now explicitly documented in proposal.md/design.md (AD-3) as reviewed, accepted interim DX debt — no new macro work this stage (non-goal), collapsing to `.service::<S>()` alone is future work once macro metadata allows it.
- [x] 4b.3 (F3, MEDIUM) Unified attribution: `AppBuilder::service`'s `DependencyNotFound` fixup (naming the requesting service) now applies to both `Injectable::validate`'s and `Injectable::build`'s error paths via a single shared `attribute_to::<S>` helper, closing the gap where a `build()`-only failure (e.g. an incompletely-declared `dependencies()`) could surface with `service_name: None`. Regression test: `build_time_dependency_failure_is_attributed_even_when_dependencies_omit_it` (`tests/app_composition.rs`).
- [x] 4b.4 (F4, MEDIUM) `App::resolve_adapter<A>()`/`resolve_config<C>()` added — public counterparts to `resolve<Tag>()`, so a constructed-but-not-started `App` (or an external integration test) can verify a registered adapter/config without reaching into the private `runtime` field. Existing unit tests updated to use them instead of `app.runtime.inner()...`.
- [x] 4b.5 (Logger scope gap) proposal.md corrected: `App` does not absorb the kit-config→`build_logger` pipeline itself (that was never decided by any AD) — `.logger()` is a thin pass-through over an already-built `KITLogger`, exactly like `.config()`. Success Criteria's boilerplate-(b) claim narrowed to match.

## Phase 5: Reference-app migration (proof)

- [x] 5.1 `examples/reference-app/src/application.rs`: **resolves open question** (AD-3 FLAG) — confirmed empirically (no `impl Injectable for RegisterUserImpl` exists anywhere in the crate) that DI modeling is not feasible: its dependencies (`Arc<EntityRuntime<_>>`s, hand-wired `ReadSideSink`) aren't DI-resolvable types. Registered via `.service_instance::<RegisterUserTag>()` instead, documented inline at the call site in `lib.rs`.
- [x] 5.2 RED — upgraded `examples/reference-app/tests/e2e_register.rs`'s `real_http_request_with_valid_jwt_registers_both_entities_end_to_end` to exercise the full production lifecycle (`App::builder()...build()`, `App::register_shutdown`, `App::start()`, `RunningApp::shutdown()`) instead of only the request/response path, so the composition/lifecycle surface is proven end-to-end, not just build_runtime's plumbing.
- [x] 5.3 GREEN — `examples/reference-app/src/lib.rs`: `build_runtime` migrated to `App::builder()` (`.security()`, `.service_instance::<RegisterUserTag>()`, conditional `.logger()`, `.build()`); `BuiltRuntime.runtime: Runtime` field replaced with `BuiltRuntime.app: App`.
- [x] 5.4 GREEN — `examples/reference-app/src/main.rs`: hand-sequenced `Arc::new(rt)` + `shutdown_async` replaced with `App::start()` → `ego_transport::serve(...)` (unchanged, host-owned) → `RunningApp::shutdown()`.
- [x] 5.5 Ran full existing reference-app suite (`cargo test -p reference-app`) — 0 failures across all ~14 test files, including `e2e_register.rs`, `http_route.rs`, `effects_e2e.rs`, `providers_e2e.rs`. Also ran full `cargo test --workspace` — 0 failures, confirming the `Runtime: Clone` addition (5b) didn't regress any other crate.

## Phase 5b: PR2 gap found during migration

- [x] 5b.1 (Integration gap) `ego_transport::AppState::new` requires `Arc<Runtime>` for its own generic per-request `resolve::<Tag>()` dispatch, but `App`/`RunningApp` never exposed the inner `Runtime` — this transport layer predates `App`/`AppBuilder` and was never covered by PR1's design. Fixed by adding `#[derive(Clone)]` to `Runtime` (cheap — wraps only `Arc<RuntimeInner>`) and adding `App::runtime(&self) -> Runtime`, callable pre-`start()` since request-time resolution doesn't depend on effects having started. Regression test: `app_runtime_resolves_a_registered_adapter_identically_to_app_resolve_adapter` (`crates/service-sdk/src/app/mod.rs`).

## Threat Matrix

N/A — no routing/shell/subprocess/process-integration boundary (design.md).
