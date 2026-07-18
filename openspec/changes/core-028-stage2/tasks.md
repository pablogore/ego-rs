# Tasks: CORE-028 Stage 2 — Projection Registration (Slice 2A)

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~300-400 (di/mod.rs ~30, builder.rs ~150, runtime_builder.rs ~15, app/mod.rs + error.rs ~135, reference-app ~30) |
| 400-line budget risk | Low |
| Chained PRs recommended | No |
| Suggested split | Single PR |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: pending
400-line budget risk: Low

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | `DuplicateProjection` + `with_projection` + `DependencyTable` wiring + `AppBuilder::projection` + reference-app registration | PR 1 (single) | `cargo test -p ego-service-sdk` | N/A — sync unit/integration tests, no Tokio required except existing e2e suite | revert `with_projection`/`.projection()`/`DuplicateProjection`/`CompositionError::Projection`, restore `with_registrations(adapters, configs)`, revert reference-app's `.projection(...)` call — purely additive per proposal.md rollback plan |

## Phase 1: `DuplicateProjection` error (AD-1, AD-4)

- [x] 1.1 RED — `crates/service-sdk/src/di/mod.rs`: test `DuplicateProjection { type_name }` carries the concrete type name (mirrors `di_primitives_are_recognizable`'s style, and `CompositionError`'s `duplicate_adapter_carries_type_name`).
- [x] 1.2 GREEN — add `pub struct DuplicateProjection { pub type_name: &'static str }` (thiserror, `#[error("projection already registered for type `{type_name}`")]`) beside `ProjectionRef` in `di/mod.rs` — resolves design.md's Open Question (placement confirmed here).
- [x] 1.3 GREEN — `crates/service-sdk/src/app/error.rs`: add `CompositionError::Projection(#[from] DuplicateProjection)` variant, mirroring `EffectExecutor`/`DataProvider` (AD-4). Add import `use crate::di::DuplicateProjection;`.
- [x] 1.4 RED/GREEN — `error.rs` test: `CompositionError::Projection` round-trips a `DuplicateProjection` via `.into()`, mirroring `validation_wraps_service_not_found_variant_too`.

## Phase 2: `RuntimeBuilder::with_projection` (AD-1, AD-2)

- [x] 2.1 RED — `crates/service-sdk/src/runtime/builder.rs` tests module: `with_projection_registers_and_resolves` (mirrors `with_adapter_registers_and_resolves`) — register a stub projection, assert `rt.inner().resolve_projection::<Stub>()` succeeds with identity preserved (spec: "A registered projection is resolvable").
- [x] 2.2 RED — `with_projection_rejects_duplicate_and_retains_first` — second registration for the same type returns `Err(DuplicateProjection { type_name })`, and the runtime built from the `Ok` first instance still resolves the FIRST value (spec: "A second registration is rejected, not silently replaced" — no `replace_projection`, AD-2).
- [x] 2.3 RED — `resolve_projection_unregistered_returns_dependency_not_found` (mirrors `resolve_adapter_unregistered_returns_dependency_not_found`) — unregistered type fails with `RuntimeError::DependencyNotFound` naming the type, no panic, no fabricated default (spec: "Resolving an unregistered projection type fails closed").
- [x] 2.4 GREEN — add `projections: HashMap<TypeId, Arc<dyn Any + Send + Sync>>` field to `RuntimeBuilder` (+ `RuntimeBuilder::new()` initializer).
- [x] 2.5 GREEN — implement `pub fn with_projection<P: Send + Sync + 'static>(mut self, projection: Arc<P>) -> Result<Self, DuplicateProjection>`: checks `self.projections.contains_key(&TypeId::of::<P>())`; `Err(DuplicateProjection { type_name: std::any::type_name::<P>() })` if present (first untouched), else inserts and returns `Ok(self)`.
- [x] 2.6 GREEN — `crates/service-sdk/src/runtime/runtime_builder.rs`: change `DependencyTable::with_registrations` to accept `projections: HashMap<TypeId, Arc<dyn Any + Send + Sync>>` as a third named parameter instead of hardcoding `HashMap::new()`.
- [x] 2.7 GREEN — `builder.rs`'s `build()`: thread `self.projections` into the `with_registrations(...)` call site.
- [x] 2.8 Update the 5 existing `DependencyTable::with_registrations(adapters, configs)` call sites to pass a third `HashMap::new()` arg (unaffected fixtures): `runtime_builder.rs:553,574,597,1222` and `builder.rs`'s own call site (2.7 covers the last one).

## Phase 3: `Injectable` integration proof (spec: "declared dependency satisfiable at build")

- [x] 3.1 RED — `builder.rs` tests: add `NeedsProjection` fixture mirroring `NeedsAdapter`/`NeedsConfig` (builder.rs:982-1036) — `dependencies()` returns `vec![DepKey::Projection(TypeId::of::<StubProjection>(), type_name)]`; `build()` resolves via `rt.resolve_projection::<StubProjection>()`.
- [x] 3.2 RED — `try_build_succeeds_when_declared_projection_dependency_is_registered` — `with_projection(...).with_injectable::<NeedsProjection>().try_build()` succeeds and the built service's field holds the registered value (spec scenario, mirrors `try_build_succeeds_identically_to_build_when_all_dependencies_present`).
- [x] 3.3 RED — `try_build_fails_before_startup_when_declared_projection_dependency_is_missing` — `with_injectable::<NeedsProjection>().try_build()` with no registration fails with `DependencyNotFound` naming both the projection type and `NeedsProjection` (mirrors `try_build_fails_fast_on_missing_dependency_naming_both_type_and_service`).
- [x] 3.4 GREEN — confirm 3.2/3.3 pass with only Phase 2's code (no new production code expected here — this phase proves the existing `try_build`/`Injectable::validate` path already composes correctly with `with_projection`).

## Phase 4: `AppBuilder::projection()` facade (AD-3)

- [x] 4.1 RED — `crates/service-sdk/src/app/mod.rs` tests: `projection_registers_and_resolves` (mirrors `data_provider_registers_and_rejects_duplicate_ids`'s success half) — `.projection(Arc::new(Stub))` then `.build()` then resolve via `app.resolver()`/existing resolve-adapter-style accessor for projections, or `App::builder()...build()` + `rt.inner().resolve_projection` if no public `App`-level projection accessor exists yet (spec: "A projection registered via AppBuilder resolves after build").
- [x] 4.2 RED — `projection_rejects_duplicate_registration_at_build` — `.projection(...).projection(...)` (same type) surfaces `CompositionError::Projection` at `.build()`, never silently replaced (spec: "Duplicate ... fails closed", "surfaced through `build()`").
- [x] 4.3 GREEN — implement `pub fn projection<P: Send + Sync + 'static>(mut self, projection: Arc<P>) -> Self` using the `effect_executor`/`data_provider` clone-then-call + `pending_error` pattern (app/mod.rs:334-382): on `Ok`, swap in `runtime_builder`; on `Err`, set `pending_error = Some(CompositionError::Projection(err))`.
- [x] 4.4 RED/GREEN — `runtimebuilder_and_appbuilder_projection_registration_are_equivalent` — build one app via `RuntimeBuilder::new().with_projection(...).build()` and one via `App::builder().projection(...).build()`; assert both resolve the same registered value with no observable difference (spec: "Registration is equivalent whether performed via RuntimeBuilder or AppBuilder").
- [x] 4.5 Confirm by construction (no extra test needed) — `.projection(...)` never requires the caller to construct or reach into `RuntimeBuilder` (spec: "No internal runtime type is required to register a projection") — the facade signature itself is the proof; note this in `app/mod.rs`'s doc comment.

## Phase 5: Reference-app reachability proof (AD-5)

- [x] 5.1 `examples/reference-app/src/lib.rs`: in `build_runtime`, after constructing `read_side_handles` (line ~236), register a clone via `builder = builder.projection(Arc::new(read_side_handles.query.clone()))` — `UsersByTenantStore`'s internal `Arc<RwLock<_>>` means the clone shares state with the engine-fed store (confirmed `read_side/projection.rs:41`); document inline that this is the DI *handle-access* path, distinct from the untouched read-side engine.
- [x] 5.2 RED — `examples/reference-app/tests/pipeline.rs` (or nearest existing e2e/pipeline test): assert `App::builder()`-built app resolves `UsersByTenantStore` via the projection path and that a value written by the read-side engine after `spawn()` is observable through the DI-resolved handle (proves the clone shares live state, not a frozen snapshot). Post-review correction: the reachability half now lives in `tests/pipeline.rs` (`build_runtime_registers_the_read_model_as_a_resolvable_projection`, cheap/non-async, matching design.md's Testing Strategy table); the live-write-observation half stays in `e2e_register.rs`, which needs the full HTTP/JWT stack to exercise the real guard chain.
- [x] 5.3 GREEN — confirm 5.2 passes with only 5.1's registration line; no other reference-app or read-side file changes (non-goal: `ReadSideHandles`/`TagSchedulerImpl` untouched).
- [x] 5.4 Run full existing reference-app suite (`cargo test -p reference-app`) — confirm 0 regressions across all existing test files (pipeline, e2e_register, http_route, effects_e2e, providers_e2e).

## Phase 6: Wiring + verification

- [x] 6.1 Confirm `DuplicateProjection` is reachable at `ego_service_sdk::DuplicateProjection` via the existing `pub use di::*;` wildcard in `lib.rs` — no new re-export line needed; add one only if the wildcard doesn't cover it.
- [x] 6.2 Run `cargo test -p ego-service-sdk` (full crate) and `cargo test --workspace` — confirm 0 failures, matching Stage 1's PR2 regression-check precedent.
- [x] 6.3 Update proposal.md's Success Criteria checkboxes once all four are demonstrably true by the tests above.

## Threat Matrix

N/A — in-memory `HashMap` insertion guarded by a `TypeId` presence check; no routing, shell, subprocess, or process-integration boundary (design.md).
