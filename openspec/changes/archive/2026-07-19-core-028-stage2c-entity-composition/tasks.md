# Tasks: CORE-028 Stage 2C — Entity Composition (`.entity::<E>()`)

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~380-470 (di/mod.rs ~55, builder.rs ~110, runtime_builder.rs ~55, app/mod.rs + error.rs ~140, lib.rs ~5, reference-app ~40) |
| 400-line budget risk | Medium |
| Chained PRs recommended | No |
| Suggested split | Single PR |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: pending
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | `DuplicateEntity` + `EntityRuntimeRef<E>` + `with_entity` + `DependencyTable` wiring + `check_dependency` flip + `NeedsEntity` proof + `AppBuilder::entity`/`App::resolve_entity` + reference-app registration | PR 1 (single) | `cargo test -p ego-service-sdk` | `cargo test -p reference-app` (existing pipeline/e2e suite, real `App::builder()` build) | Revert `with_entity`/`.entity()`/`EntityRuntimeRef`/`DuplicateEntity`/`resolve_entity`, drop `entities` map, restore always-`Err` `check_dependency` arm + its pinning test, revert reference-app's `.entity(...)` call — purely additive per proposal.md rollback plan |

## Phase 1: `DuplicateEntity` error + `EntityRuntimeRef<E>` type (AD-1, AD-3, AD-4)

- [x] 1.1 RED — `crates/service-sdk/src/di/mod.rs`: test `duplicate_entity_carries_type_name` — `DuplicateEntity { type_name }` carries the concrete type name (mirrors `duplicate_projection_carries_type_name`).
- [x] 1.2 GREEN — add `pub struct DuplicateEntity { pub type_name: &'static str }` (thiserror, `#[error("entity runtime already registered for type `{type_name}`")]`) beside `ProjectionRef`/`DuplicateProjection`.
- [x] 1.3 GREEN — `crates/service-sdk/src/app/error.rs`: add `CompositionError::Entity(#[from] DuplicateEntity)` variant, mirroring `Projection`; add import `use crate::di::DuplicateEntity;`.
- [x] 1.4 RED/GREEN — `error.rs` test `entity_wraps_duplicate_entity_variant`: `CompositionError::Entity` round-trips a `DuplicateEntity` via `.into()`, mirroring `projection_wraps_duplicate_projection_variant`.
- [x] 1.5 GREEN — `di/mod.rs`: add `pub struct EntityRuntimeRef<E: PersistentEntity> { inner: Arc<EntityRuntime<E::Event>> }` with `new(inner: Arc<EntityRuntime<E::Event>>) -> Self` and the `entity_ref<C, S>(...)` passthrough to `EntityRuntime::entity_ref` (AD-3 exact signature/bounds); import `persistent_entity::persistent_entity::PersistentEntity`, `persistent_entity::runtime::EntityRuntime`, `persistent_entity::entity_ref::EntityRef`, `persistent_entity::error::EntityError`, `ego_domain::event::DomainEvent`.
- [x] 1.6 GREEN — correct the stale comment at `di/mod.rs` lines 6-8: remove the `entity_sdk::EntityRef`/CORE-006-pending note; replace with a short note distinguishing `EntityRuntimeRef<E>` (this file, composition-time) from `persistent_entity::EntityRef<E>` (per-dispatch handle, unchanged, owned by `persistent-entity`).

## Phase 2: `RuntimeBuilder::with_entity` (AD-1, AD-2, AD-4)

- [x] 2.1 RED — `crates/service-sdk/src/runtime/builder.rs` tests: `with_entity_registers_and_resolves` (mirrors `with_projection_registers_and_resolves`) — build `Arc<EntityRuntime<persistent_entity::testing::TestEvent>>` via `EntityRuntimeBuilder::new().build()`, register for `persistent_entity::test_entity::TestEntity` (existing test-only fixture, reused as-is — no new fixture invented), assert `rt.inner().resolve_entity::<TestEntity>()` succeeds.
- [x] 2.2 RED — `with_entity_rejects_duplicate_and_retains_first` — second registration for the same aggregate type returns `Err(DuplicateEntity { type_name })`; the runtime built from the first `Ok` still resolves the FIRST instance (AD-4, no `replace_entity`).
- [x] 2.3 RED — `resolve_entity_unregistered_returns_dependency_not_found_naming_aggregate` — unregistered aggregate type fails with `RuntimeError::DependencyNotFound` naming the aggregate `E` (not `E::Event`), no panic, no fabricated default.
- [x] 2.4 GREEN — add `entities: HashMap<TypeId, Arc<dyn Any + Send + Sync>>` field to `RuntimeBuilder` (+ `RuntimeBuilder::new()` initializer), doc-commented like `projections`.
- [x] 2.5 GREEN — implement `pub fn with_entity<E>(mut self, runtime: Arc<EntityRuntime<E::Event>>) -> Result<Self, DuplicateEntity> where E: PersistentEntity + 'static, E::Event: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static` (AD-2 exact bound stack): checks `self.entities.contains_key(&TypeId::of::<E>())`; `Err(DuplicateEntity { type_name: std::any::type_name::<E>() })` if present (first untouched), else inserts under `TypeId::of::<E>()` and returns `Ok(self)`.
- [x] 2.6 RED — `two_aggregates_sharing_an_event_type_register_and_resolve_without_collision` (proves AD-1's actual claim, not just error naming): define a second test-only `PersistentEntity` aggregate whose `Event` type is the SAME `persistent_entity::testing::TestEvent` already used for `TestEntity` (e.g. `TestEntity2` or reuse of an existing second fixture if the crate has one); register both `TestEntity` and this second aggregate via two `with_entity` calls with distinct `Arc<EntityRuntime<TestEvent>>` instances; assert `resolve_entity::<TestEntity>()` and `resolve_entity::<TestEntity2>()` each return their OWN registered instance (not `DuplicateEntity`, not the other aggregate's runtime) — this is the scenario that actually falsifies event-keyed identity, distinct from 2.3's naming-only check.

## Phase 3: `DependencyTable` wiring + `resolve_entity` + retire `check_dependency` stub (AD-1, AD-7 pinning-test replacement)

- [x] 3.1 GREEN — `crates/service-sdk/src/runtime/runtime_builder.rs`: add `entities: HashMap<TypeId, Arc<dyn Any + Send + Sync>>` field to `DependencyTable`; add `entities: HashMap::new()` to the `#[cfg(test)] fn new()` initializer.
- [x] 3.2 GREEN — change `DependencyTable::with_registrations` to accept `entities` as a fourth named parameter (`adapters, configs, projections, entities`); update its doc comment.
- [x] 3.3 GREEN — add `fn resolve_entity<E: PersistentEntity + 'static>(&self) -> Result<EntityRuntimeRef<E>, RuntimeError>` on `DependencyTable`, mirroring `resolve_projection`'s downcast-and-wrap shape, keyed by `TypeId::of::<E>()` (AD-1) and erasing/downcasting `Arc<EntityRuntime<E::Event>>`; add public `RuntimeInner::resolve_entity::<E>()` wrapper mirroring `resolve_projection`.
- [x] 3.4 GREEN — `builder.rs`'s `build()`: thread `self.entities` into the `with_registrations(...)` call site.
- [x] 3.5 Update the 4 existing `DependencyTable::with_registrations(adapters, configs, projections)` call sites to pass a fourth `HashMap::new()` arg: `runtime_builder.rs:556,577,600,1225`.
- [x] 3.6 RED — `runtime_builder.rs` test `check_dependency_entity_present_is_ok` (mirrors `check_dependency_projection_present_is_ok`) — insert into `rt.resolved.entities`, assert `check_dependency(&DepKey::Entity(...))` is `Ok`.
- [x] 3.7 GREEN — flip the `check_dependency` `DepKey::Entity` arm (currently `(false, *name)` at `runtime_builder.rs:374`) to `(self.resolved.entities.contains_key(id), *name)`, mirroring the `Projection` arm; update the stale doc comment above it (lines ~363-371) describing the old always-`Err` behavior.
- [x] 3.8 GREEN — retire `check_dependency_entity_is_always_err_regardless_of_table_state` (`runtime_builder.rs:1088-1095`); replace with `check_dependency_entity_missing_is_err_named` (mirrors `check_dependency_projection_missing_is_err_named`) — confirms 3.6 and this new test are the only two `DepKey::Entity` pinning tests remaining.

## Phase 4: `Injectable` integration proof (AD-7 item 1 — consumption, not just registration)

- [x] 4.1 RED — `builder.rs` tests: add `NeedsEntity` fixture mirroring `NeedsProjection` — `dependencies()` returns `vec![DepKey::Entity(TypeId::of::<TestEntity>(), type_name::<TestEntity>())]`; `build()` resolves via `rt.resolve_entity::<TestEntity>()`.
- [x] 4.2 RED — `try_build_succeeds_when_declared_entity_dependency_is_registered` — `with_entity(...).with_injectable::<NeedsEntity>().try_build()` succeeds and the built service's field holds the registered runtime (mirrors `try_build_succeeds_when_declared_projection_dependency_is_registered`).
- [x] 4.3 RED — `try_build_fails_before_startup_when_declared_entity_dependency_is_missing` — `with_injectable::<NeedsEntity>().try_build()` with no registration fails with `DependencyNotFound` naming both the aggregate type (`TestEntity`, not `TestEvent`) and `NeedsEntity`.
- [x] 4.4 GREEN — confirm 4.2/4.3 pass with only Phase 2/3's code (no new production code expected — proves `try_build`/`Injectable::validate` already composes correctly with `with_entity`).

## Phase 5: `AppBuilder::entity()` facade + `App::resolve_entity()` (AD-5, AD-8)

- [x] 5.1 RED — `crates/service-sdk/src/app/mod.rs` tests: `entity_registers_and_resolves` (mirrors `projection_registers_and_resolves`) — `.entity::<TestEntity>(runtime)` then `.build()` then `app.resolve_entity::<TestEntity>()`.
- [x] 5.2 RED — `entity_rejects_duplicate_registration_at_build` — `.entity(...).entity(...)` (same aggregate type) surfaces `CompositionError::Entity` at `.build()`, never silently replaced.
- [x] 5.3 GREEN — implement `pub fn entity<E>(mut self, runtime: Arc<EntityRuntime<E::Event>>) -> Self` (AD-5 exact clone-then-call + `pending_error` pattern, same bounds as `with_entity`): `Ok` swaps in `runtime_builder`; `Err` sets `pending_error = Some(CompositionError::Entity(err))`.
- [x] 5.4 GREEN — implement `pub fn resolve_entity<E>(&self) -> Result<EntityRuntimeRef<E>, RuntimeError>` on `App` (AD-8), mirroring `resolve_projection` at `app/mod.rs:177-181`.
- [x] 5.5 RED/GREEN — `runtimebuilder_and_appbuilder_entity_registration_are_equivalent` — build one app via `RuntimeBuilder::new().with_entity(...).build()` and one via `App::builder().entity(...).build()`; assert both resolve the same registered runtime with no observable difference.

## Phase 6: Reference-app reachability proof (AD-7 item 2)

- [x] 6.1 `examples/reference-app/src/lib.rs`: in `build_runtime`, after `user_runtime` is constructed (line ~229), register it via `builder = builder.entity::<UserEntity>(user_runtime.clone())`; import `crate::domain::user::UserEntity` (or its actual module path). Explicitly do NOT touch `RegisterUserImpl::new(...)`'s hand-threaded `org_runtime`/`user_runtime`/`.service_instance()` call (AD-9 non-goal, unrelated to this slice).
- [x] 6.2 RED — nearest existing e2e/pipeline test file (mirrors `build_runtime_registers_the_read_model_as_a_resolvable_projection`): assert the built `App` resolves `EntityRuntimeRef<UserEntity>` via `App::resolve_entity::<UserEntity>()` after `build_runtime()`.
- [x] 6.3 GREEN — confirm 6.2 passes with only 6.1's registration line; no other reference-app file changes.
- [x] 6.4 Run full existing reference-app suite (`cargo test -p reference-app`) — confirm 0 regressions across all existing test files.

## Phase 7: Wiring + verification

- [x] 7.1 `crates/service-sdk/src/lib.rs`: confirm `EntityRuntimeRef`/`DuplicateEntity` are reachable at `ego_service_sdk::{EntityRuntimeRef, DuplicateEntity}` via the existing `pub use di::*;` wildcard — add explicit re-export lines only if the wildcard doesn't cover it.
- [x] 7.2 Run `cargo test -p ego-service-sdk` (full crate) and `cargo test --workspace` — confirm 0 failures.
- [x] 7.3 Update `proposal.md`'s Success Criteria checkboxes once all five are demonstrably true by the tests above.
- [x] 7.4 Merge the delta specs (`specs/application-composition/spec.md`, `specs/service-sdk/spec.md`) into `openspec/specs/application-composition/spec.md` and `openspec/specs/service-sdk/spec.md`, retiring the stale CORE-006-deferral Non-Goals entries in both.

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file
classification, or process-integration boundary. This adds one in-memory
`HashMap` insertion guarded by a `TypeId` presence check plus a downcast on
resolve; nothing spawns, serves, or executes at composition (design.md).
