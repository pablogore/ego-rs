# Tasks: CORE-028D1 — Effect Store Duplicate Registration Guard

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~90-120 |
| 400-line budget risk | Low |
| Chained PRs recommended | No |
| Suggested split | single PR |
| Delivery strategy | single-pr |
| Chain strategy | size-exception |

Decision needed before apply: Yes
Chained PRs recommended: No
Chain strategy: size-exception
400-line budget risk: Low

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | Guard + short-circuit on both methods, two new error variants, full test set | PR 1 (single) | `cargo test -p service-sdk app::` | N/A — pure builder-state unit tests, no runtime/transport involved | Revert the single commit; additive guard, no signature change |

## Phase 1: Error Variants (`app/error.rs`)

- [x] 1.1 RED — add `duplicate_effect_store_carries_type_name` test (mirrors `duplicate_adapter_carries_type_name`, `error.rs:71`) asserting `CompositionError::DuplicateEffectStore { type_name }` round-trips.
- [x] 1.2 GREEN — add `DuplicateEffectStore { type_name: &'static str }` variant with `#[error("effect store already registered; second registration of `{type_name}` rejected")]`, placed after `DuplicateAdapter` (`error.rs:27`).
- [x] 1.3 RED — add `duplicate_effect_retention_store_display_text` test pinning the fieldless variant's `Display` message.
- [x] 1.4 GREEN — add fieldless `DuplicateEffectRetentionStore` variant with `#[error("effect retention store already registered")]`, adjacent to 1.2's variant.

## Phase 2: `AppBuilder` State + Guard (`app/mod.rs`)

- [x] 2.1 Add `effect_store_registered: bool` and `effect_retention_store_registered: bool` fields to `AppBuilder` (`app/mod.rs:118-126`), immediately after `adapter_types`; initialize both `false` at every existing `AppBuilder` construction site.
- [x] 2.2 RED — add `duplicate_effect_store_registration_is_rejected` test (mirrors `duplicate_adapter_registration_is_rejected`, `app/mod.rs:820`) using `compat_app()`/`RecordingEffectStore`, asserting `build()` fails with `CompositionError::DuplicateEffectStore { type_name }` matching `type_name::<RecordingEffectStore>()`.
- [x] 2.3 GREEN — rewrite `effect_store<T>` (`app/mod.rs:533-539`) to: short-circuit on `pending_error`, latch `DuplicateEffectStore` and return unmodified if `effect_store_registered`, else set the flag then delegate to `with_effect_store`.
- [x] 2.4 TRIANGULATE (RED then GREEN, no new production code expected) — add `second_effect_store_of_a_different_type_is_still_rejected`: register `RecordingEffectStore` then a distinct `InMemoryEffectStore`; assert still `Err(DuplicateEffectStore)` with `type_name` naming the second type — proves the guard is presence-based, not `TypeId`-keyed.
- [x] 2.5 RED — add `duplicate_effect_retention_store_registration_is_rejected` (mirrors 2.2) asserting `Err(DuplicateEffectRetentionStore)`.
- [x] 2.6 GREEN — rewrite `effect_retention_store` (`app/mod.rs:546-549`) with the same short-circuit + presence-flag-latch + delegate shape as 2.3, using `effect_retention_store_registered`.
- [x] 2.7 RED — add `effect_store_and_retention_store_short_circuit_on_a_pending_error`: pre-latch a `DuplicateAdapter` error, call both `.effect_store(...)` and `.effect_retention_store(...)` afterward, assert the builder is unmodified and `build()` still surfaces the original `DuplicateAdapter`.
- [x] 2.8 GREEN — confirm 2.3/2.6's short-circuit clauses already satisfy 2.7 (no new production code expected; run to verify).
- [x] 2.9 Run `effect_retention_store_composes_with_the_same_instance_via_app_builder` (`app/mod.rs:1542`) unmodified — confirm it stays green, proving the two flags are independent per design's non-goal.

## Phase 3: Verification

- [x] 3.1 Run `cargo test -p service-sdk app::` — all new and existing tests green.
- [x] 3.2 Run `cargo test --workspace` — full suite green.
- [x] 3.3 Diff `crates/service-sdk/src/runtime/builder.rs` against `develop` — confirm zero changes (proposal/design invariant).
