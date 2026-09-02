# Tasks: PROD-014A — Read-Side Durable Progress Composition

> Canonical / source of truth. Spanish companion: `tasks.es.md` (1:1 identifiers).
> Strict TDD: every task is RED (failing test) before GREEN. `cargo clippy --workspace -- -W clippy::cognitive-complexity` after each split.

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~450–600 (2 trait mods, RuntimeBuilder+AppBuilder changes, 13 mechanical call sites, new host types, unit/integration/E2E tests) |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (framework) → PR 2 (host) |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending — ask maintainer: stacked-to-main or feature-branch-chain |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | Framework: `is_durable()` + `Arc<T>` forwarding on both SPIs; `RuntimeBuilder` registration + validator split; `AppBuilder` registration + dup guard | PR 1 | `cargo test -p ego-domain read_side`, `cargo test -p ego-service-sdk read_side_progress` | N/A — unit/integration only, no host needed to prove PR 1 | Revert the two trait defaults, `Arc<T>` impls, `RuntimeBuilder`/`AppBuilder` fields+methods, `CompositionError` variant; nothing else depends on them yet |
| 2 | Host: reference-app `ReadSideProgressStores`/`FakeDurable*` rewiring, 13 mechanical call sites, `main.rs` `None`, `profile.rs` doc fix | PR 2 | `cargo test -p reference-app production_profile_guard` | `examples/reference-app` existing Dev-profile scenario (no new harness) | Revert `ReadSideHandles::new`/`build_runtime_with` signatures and the 13 call sites; PR 1 stays valid at zero registrations |

## Phase 1: Domain SPIs (Foundation) — PR 1

- [x] 1.1 RED `crates/domain/src/read_side/offset.rs`: bare impl defaults `is_durable()` false; `Arc::new(durable).is_durable() == true`; verify `?Sized` under `#[async_trait]` first (AD-3 landmine), fall back to `T: ... + 'static` if impractical
- [x] 1.2 GREEN: add `is_durable(&self) -> bool { false }` default + `impl OffsetStore for Arc<T>` forwarding `read_offset`/`write_offset`/`is_durable` (AD-3, AD-4)
- [x] 1.3 RED+GREEN: mirror 1.1–1.2 in `dedup.rs` for `DedupStore`

## Phase 2: RuntimeBuilder Registration + Validator Split — PR 1

- [x] 2.1 RED `crates/service-sdk/src/runtime/builder.rs`: matrix {Dev,Production}×{none, durable, volatile-offset, volatile-dedup, both-volatile}; assert EC-1 regression — zero effect executors + volatile read-side still refused; `build()`/`try_build()` agree
- [x] 2.2 GREEN: add `read_side_progress: BTreeMap<String, ReadSideProgressPair>` field, private `ReadSideProgressPair{offset,dedup}`, `with_read_side_progress(...)`
- [x] 2.3 GREEN: split `validate_persistence_profile` into unchanged `validate_effect_store_profile` + new `validate_read_side_progress_profile` (AD-6); sequencer calls both, effect-store first
- [x] 2.4 Add minimal `#[cfg(test)]` durable/volatile `OffsetStore`/`DedupStore` stubs for the matrix (AD-9, framework side)

## Phase 3: AppBuilder Registration + Dup Guard — PR 1

- [x] 3.1 RED `crates/service-sdk/src/app/error.rs`: `DuplicateReadSideProgress` message names `projection_id`, suggests no replace API
- [x] 3.2 GREEN: add `CompositionError::DuplicateReadSideProgress { projection_id }`
- [x] 3.3 RED `crates/service-sdk/src/app/mod.rs`: same `projection_id` twice fails closed at `build()` with first registration intact; two different ids both register; a pre-existing `pending_error` is not overwritten
- [x] 3.4 GREEN: add `read_side_progress_ids: HashSet<String>` + `read_side_progress(projection_id, offset, dedup)` (latch before delegating to `RuntimeBuilder`)
- [x] 3.5 Integration test `crates/service-sdk/tests/`: refusal surfaces as `CompositionError::Validation(RuntimeError::PersistenceNotConfigured(..))` through the full `build()` path

## Phase 4: Reference-App Rewiring — PR 2

- [x] 4.1 RED+GREEN `examples/reference-app/src/read_side/store.rs`: `FakeDurableOffsetStore`/`FakeDurableDedupStore` delegate to `InMemory*`, override `is_durable() -> true` (AD-9)
- [x] 4.2 `read_side/mod.rs`: add `ReadSideProgressStores{offset,dedup}` with `in_memory()`/`fake_durable()`; change `ReadSideHandles::new(store, progress)` (AD-8)
- [x] 4.3 `lib.rs`: `build_runtime_with` gains `read_side_progress: Option<ReadSideProgressStores>`; `None` → `in_memory()` + no registration; `Some(pair)` registers via `AppBuilder::read_side_progress(PROJECTION_ID, ..)` and passes the same clone to `ReadSideHandles::new`
- [x] 4.4 `main.rs`: pass `None` (no durable backend exists — F-1)
- [x] 4.5 Mechanical: update the 13 call sites per design's Blast Radius list (5× `ReadSideHandles::new`, 8× `build_runtime_with`, across `tests/` and `integration-tests/`)
- [x] 4.6 `crates/persistent-entity/src/profile.rs`: replace `Profile::Production` doc comment verbatim (AD-10, IS-10) — no signature change

## Phase 5: End-to-End Verification — PR 2

- [x] 5.1 Update `examples/reference-app/tests/users_by_tenant_projection.rs`: erased pair flows through `ReadSideProgressStores` to the real scheduler (AD-3 works end to end)
- [x] 5.2 Update `examples/reference-app/tests/production_profile_guard.rs`: `None` still builds under Dev; `Some(fake_durable())` registers and builds under Production; a volatile registered pair is refused
- [x] 5.3 `cargo test --workspace` zero failures; `cargo clippy --workspace -- -D warnings` clean; confirm no function from 2.3 exceeds complexity 10
