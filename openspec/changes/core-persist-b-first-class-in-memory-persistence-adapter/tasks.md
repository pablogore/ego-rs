# Tasks: CORE-PERSIST-B — First-Class In-Memory Persistence Adapter

> Canonical / source of truth. Spanish companion: `tasks.es.md` (1:1 identifiers).
> Strict TDD (`openspec/config.yaml` → `apply.tdd: true`): every slice's RED is a compile
> failure naming a path that does not exist yet (design AD-9) — a valid RED per
> `ego-rs-testing-tdd`. This change writes no new behavior, so no task adds a behavioral
> assertion; the assertions that matter already exist and must keep passing **unmodified**.
> Slice order is design AD-9's mandatory S1 (infrastructure) → S2 (reservation store) → S3
> (reference-app), each independently compiling workspace-wide before the next starts.
>
> **OQ-2 traceability note**: S2 makes `InMemoryOperationReservationStore` production-reachable
> for the first time (D-7, AD-8). The user explicitly GRANTED this reachability change — no
> further sign-off task exists; Phase 9 records it as fact, not as an open question.

## Review Workload Forecast

Measured source line counts (not estimates): `event_store.rs` 268, `read_side_store.rs` 225,
`repository.rs` 69, `snapshot.rs` 49 (S1, 611 lines total) · `reservation.rs` 573, of which the
moving store/`Record`/`RecordState` block is the majority (S2) · reference-app `store.rs` 413,
of which the two moving structs are a ~90-line block (S3). Per proposal risk R-4 and design
AD-9's own framing, verbatim relocation counts full add+delete, not a summary diff — CORE-PERSIST-A
measured 1,600–2,000 raw lines for a smaller, two-crate version of this same shape.

| Field | Value |
|-------|-------|
| Estimated changed lines | ~1,400–1,900 total — S1 ~600–850 (4 files + skeleton + identity test), S2 ~500–700 (largest single file, plus `Cargo.toml`/`lib.rs` edges), S3 ~250–350 (2 files + 2 `mod.rs` edits) |
| 400-line budget risk | High for the combined total; individual slices land closer to the 800-line budget but S1 and S2 each risk exceeding it too once doc comments and the identity-witness tests are counted |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (S1 — infrastructure) → PR 2 (S2 — reservation store) → PR 3 (S3 — reference-app) |
| Delivery strategy | stacked-to-main, 3 PRs (decided — supersedes the single-pr session preflight) |
| Chain strategy | stacked-to-main — matches AD-9's mandatory order and per-slice revertibility |

Decision recorded: the user explicitly accepted trading single-pr for the stacked 3-PR chain
below, given the 800-line budget was exceeded by the combined total (per proposal R-4 and design
AD-9). No `size:exception` was granted — the delivery strategy itself changed instead.
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | PR | Branches from | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----|----------------|----------------------|-----------------|-------------------|
| 1 | S1 — new crate skeleton, `layers.toml`, 4 `ego-infrastructure` implementations relocated + re-exported | PR 1 | `develop` | `cargo build -p ego-persistence-memory && cargo test -p ego-infrastructure` | N/A — pure structural relocation, no new behavior to exercise (design Testing Strategy) | Drop `crates/persistence-memory/`, restore the 4 files + `in_memory/mod.rs`, remove the `layers.toml`/root-`Cargo.toml`/`infrastructure/Cargo.toml` edges |
| 2 | S2 — `InMemoryOperationReservationStore` split out of `ego-testkit`, `ego-domain`+`chrono` edges added | PR 2 | PR 1 | `cargo build -p ego-persistence-memory && cargo test -p ego-testkit` | N/A — same reason | Restore `reservation.rs`'s pre-split declarations, drop `operation/reservation.rs`, revert the two `Cargo.toml` edges; PR 1 stays valid |
| 3 | S3 — `InMemoryOffsetStore`/`InMemoryDedupStore` relocated out of `examples/reference-app` | PR 3 | PR 2 | `cargo build -p reference-app && cargo test --workspace` | N/A — same reason | Restore the example's two declarations and its two import sites; PR 1–2 remain valid |

## Phase 1: Crate Skeleton & Layer Gate — S1 — PR 1

- [x] 1.1 Create `crates/persistence-memory/Cargo.toml` (package `ego-persistence-memory`): deps `ego-persistence-api` (path), `async-trait` (workspace), `serde_json`; dev-deps `tokio` (`macros`, `rt`), `chrono` — dev-only in S1, promoted to normal in S2 (AD-2, EC-4, EC-7).
- [x] 1.2 Create `crates/persistence-memory/src/lib.rs`: `pub mod persistence;` + `pub mod read_side;` only, no crate-root re-exports, no `#![deny(missing_docs)]` (AD-3 Refinements 2–3).
- [x] 1.3 Create `src/persistence/mod.rs` and `src/read_side/mod.rs` skeletons declaring their S1 submodules (AD-3).
- [x] 1.4 Add `layers.toml` entry `"ego-persistence-memory" = "foundation"`. Do not open `xtask/src/layers.rs` (AD-1).
- [x] 1.5 Add `"crates/persistence-memory",` to the root `Cargo.toml` workspace members.
- [x] 1.6 Add the `ego-persistence-memory` path dependency to `crates/infrastructure/Cargo.toml`.

## Phase 2: RED — Compatibility Identity Test, S1 — PR 1

- [x] 2.1 Create `crates/infrastructure/tests/in_memory_reexport_identity.rs` with one identity witness per S1 row of the restated compatibility matrix: `InMemoryEventStore`, `InMemoryRepository`, `InMemorySnapshotStore`, `{InMemoryReadSideStore, paginate}` (object-safe traits get an identity coercion; `paginate` gets a function-pointer equality test, per AD-10). `InMemoryEventStoreUnitOfWork` needs none — private, reachable only via `Box<dyn EventStoreUnitOfWork>`. Fails to compile: none of the `ego_persistence_memory::…` paths exist yet.

## Phase 3: GREEN — Relocate the Four `ego-infrastructure` Implementations — S1 — PR 1

- [x] 3.1 Move `event_store.rs` verbatim to `src/persistence/event_store.rs`; rewrite its 4 `use ego_domain::…` lines to `ego_persistence_api::…` per AD-4 row 1.
- [x] 3.2 Move `repository.rs` verbatim to `src/persistence/repository.rs`; AD-4 row 2 rewrite.
- [x] 3.3 Move `snapshot.rs` verbatim to `src/persistence/snapshot.rs`; AD-4 row 3 rewrite (`use serde_json::Value;` unchanged).
- [x] 3.4 Move `read_side_store.rs` verbatim — including its `#[cfg(test)]` module (EC-4) — to `src/read_side/store.rs` (AD-3 Refinement 1 rename); AD-4 row 4 rewrite.

## Phase 4: GREEN — Re-export at Old Paths, S1 — PR 1

- [x] 4.1 Replace `crates/infrastructure/src/persistence/in_memory/mod.rs`'s 4 `mod` declarations with 4 item-level `pub use ego_persistence_memory::…` lines (AD-6); delete the 4 now-empty source files; keep the module doc (`:1-5`) unchanged.

## Phase 5: Verification — S1 — PR 1

- [x] 5.1 `cargo build -p ego-persistence-memory` succeeds standalone.
- [x] 5.2 `cargo build --workspace` succeeds; turns 2.1's identity witnesses green.
- [x] 5.3 `cargo run -p xtask -- verify-layers` passes: new crate mapped, no matrix edit, no cycle (R11).
- [x] 5.4 `cargo test --workspace` passes; the relocated `read_side_store.rs` test module's assertion count is unchanged (R5).
- [x] 5.5 Diff-read: `crates/infrastructure/tests/in_memory_event_store_conformance.rs`, `crates/infrastructure/tests/commit_publishes_atomically.rs`, `examples/reference-app/src/lib.rs:432-439` compile byte-identical (R9).
- [x] 5.6 Semantic zero-diff gate: diff each of the 4 moved files old-path vs. new-path — identical apart from module path and AD-4's enumerated import lines (R5, R2).

## Phase 6: RED — Compatibility Identity Test, S2 — PR 2

- [ ] 6.1 Create `crates/testkit/tests/reservation_reexport_identity.rs` with an identity witness for `InMemoryOperationReservationStore` at `ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore`. Fails to compile: the path does not exist yet.

## Phase 7: GREEN — Split `reservation.rs` and Relocate the Reservation Store — S2 — PR 2

- [ ] 7.1 Add `ego-domain` (path) to `crates/persistence-memory/Cargo.toml` and promote `chrono` from dev to a normal dependency — the only slice that widens the dependency edge (AD-2, D-3).
- [ ] 7.2 Add `pub mod operation;` to `src/lib.rs`; create `src/operation/mod.rs`.
- [ ] 7.3 Move `RecordState`, `Record`, `InMemoryOperationReservationStore`, and its `impl OperationReservationStore` verbatim from `crates/testkit/src/reservation.rs` to `src/operation/reservation.rs`. Rewrite the eleven-name `operation::` import to `ego_persistence_api::operation::reservation::{…}` (EC-1, AD-4 row 7); rewrite the inline `fingerprint: ego_domain::operation::OperationFingerprint` path to `ego_persistence_api::operation::key::OperationFingerprint` (EC-2, AD-5). `use ego_domain::Clock;` stays unchanged — the crate's one surviving `ego_domain::` line.
- [ ] 7.4 Add the `ego-persistence-memory` path dependency to `crates/testkit/Cargo.toml`.

## Phase 8: GREEN — Re-export Inside `reservation.rs`; `TestClock` and Tests Stay — S2 — PR 2

- [ ] 8.1 Prune `crates/testkit/src/reservation.rs`'s imports to only what `TestClock` needs (`std::sync::Mutex`, `chrono::{DateTime, Duration, Utc}`, `ego_domain::Clock`); add `pub use ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore;` inside the module, immediately after (EC-3, AD-5). Leave `TestClock`, its `impl Clock`, and both `#[cfg(test)]` modules (`:370-512`, `:514-…`) byte-identical, including their `use super::{…}` lines.
- [ ] 8.2 Confirm `crates/testkit/src/lib.rs:50` needs zero edits (EC-3).

## Phase 9: Verification — S2 — PR 2

- [ ] 9.1 `cargo build -p ego-persistence-memory` succeeds standalone with the new `ego-domain`/`chrono` edges.
- [ ] 9.2 `cargo build --workspace` succeeds; turns 6.1's identity witness green.
- [ ] 9.3 `rg '^use ego_domain::|ego_domain::' crates/persistence-memory/src` returns exactly one line (AD-2 criterion 4).
- [ ] 9.4 `cargo run -p xtask -- verify-layers` passes with the `ego-domain` edge (`foundation → domain`, already permitted, no matrix edit).
- [ ] 9.5 `cargo test --workspace` passes; both colocated reservation `#[cfg(test)]` suites pass unmodified, driving the re-exported store through `super::` (D-8, R16).
- [ ] 9.6 Diff-read: `crates/transport/tests/operation_key_extractor.rs`, `crates/service-sdk/tests/retention_worker_lifecycle.rs`, `crates/service-sdk/tests/cross_tenant_reservation_isolation.rs` compile byte-identical (R9). Record in the PR description: this slice makes `InMemoryOperationReservationStore` production-reachable, per D-7/AD-8, already GRANTED — no further sign-off task.
- [ ] 9.7 Semantic zero-diff gate for the reservation store's moved body (R5, R2).

## Phase 10: RED — Retarget Reference-App Imports Before the New Paths Exist — S3 — PR 3

- [ ] 10.1 In `examples/reference-app/src/read_side/store.rs`, remove the `InMemoryOffsetStore`/`InMemoryDedupStore` declarations and replace the file's relevant imports with `use ego_persistence_memory::read_side::{dedup::InMemoryDedupStore, offset::InMemoryOffsetStore};` (AD-7). `cargo build -p reference-app` fails: the paths do not exist in `ego-persistence-memory` yet.
- [ ] 10.2 Add the `ego-persistence-memory` path dependency to `examples/reference-app/Cargo.toml`.

## Phase 11: GREEN — Relocate the Two Read-Side Stores — S3 — PR 3

- [ ] 11.1 Add `pub mod offset;` and `pub mod dedup;` to `src/read_side/mod.rs`.
- [ ] 11.2 Create `src/read_side/offset.rs`: `InMemoryOffsetStore` + `OffsetKey` moved verbatim from reference-app `store.rs`; rewrite its import to `ego_persistence_api::read_side::{offset::{Offset, OffsetStore, OffsetStoreError}, event_tag::EventTag}` (AD-4 row 5).
- [ ] 11.3 Create `src/read_side/dedup.rs`: `InMemoryDedupStore` + `DedupKey` moved verbatim; same rewrite shape for `dedup`/`event_tag` (AD-4 row 6).
- [ ] 11.4 Update `examples/reference-app/src/read_side/mod.rs:36-39`: replace the two removed names with `pub use ego_persistence_memory::read_side::{dedup::InMemoryDedupStore, offset::InMemoryOffsetStore};`; keep `pub use store::{FakeDurableDedupStore, FakeDurableOffsetStore, ReadSideSink, SharedReadSideStore};` (AD-7).

## Phase 12: Verification — S3 — PR 3

- [ ] 12.1 `cargo build -p reference-app` succeeds — turns 10.1's RED green.
- [ ] 12.2 `cargo build --workspace` succeeds.
- [ ] 12.3 `cargo test --workspace` passes; the example's own `#[cfg(test)]` module (`store.rs:309+`) still exercises `SharedReadSideStore`, `ReadSideSink`, and both `FakeDurable*` wrappers unmodified (NG-8, R3).
- [ ] 12.4 Diff-read: `FakeDurableOffsetStore`/`FakeDurableDedupStore` remain byte-identical, declared only in the example; `OffsetKey`/`DedupKey` moved with their structs (EC-5).
- [ ] 12.5 Semantic zero-diff gate for both relocated read-side stores.

## Phase 13: Whole-Change Verification & Diff Audit — PR 3

- [ ] 13.1 `cargo run -p xtask -- verify-layers` passes end-to-end: new crate mapped, zero violations, matrix untouched (R11).
- [ ] 13.2 Diff-read across all three PRs: zero SQL/migration/`crates/persistence/` file (R13); `crates/runtime/` and `crates/effect-store/` byte-identical, `InMemoryEffectStore` and its three ports untouched (R12); `crates/persistence-api/src/**` byte-identical (R15); `crates/persistent-entity/` untouched, both duplicates still forked (R17, EC-6).
- [ ] 13.3 Confirm the workspace-wide `impl <Port> for` count per moved port is unchanged (R2, R10); the only surviving non-canonical declarations are `persistent-entity`'s two named duplicates and the declared test fakes.
- [ ] 13.4 Confirm `ProjectionStateStore` still has zero implementations, no stub or `todo!()` anywhere in the new crate (R4).
- [ ] 13.5 Confirm `presence_alone_is_not_durability` and both `try_build_rejects_explicit_in_memory_*` tests (`persistent-entity/src/builder.rs:768,793`, `profile.rs:99-117`) pass unmodified (R6).
- [ ] 13.6 Confirm `crates/persistence-memory/Cargo.toml` names exactly `ego-persistence-api` and `ego-domain` as workspace path dependencies (R11); confirm no `sqlx`/Postgres/Stoolap/HTTP/Kafka token appears anywhere under `crates/persistence-memory/` (R7).

## Deferred / Out of Scope (named debt, not tasks)

- **KD-1** — `ProjectionStateStore` stays at zero implementations. No task implements it (NG-10, D-12, R4).
- **KD-5 → F-5** — `persistent-entity`'s `InMemorySnapshotStore` (`persistence.rs:733`) ignores `tenant_id`, a confirmed tenant-isolation defect. Not fixed here (NG-1, D-9). Follow-up F-5: a standalone reviewed bugfix with its own tests, independent of the CORE-PERSIST series.
- **KD-6 → F-6** — `persistent-entity`'s `InMemoryEventStore`/`StagingUnitOfWork` (`persistence.rs:571`) is an unconsolidated fork carrying `with_version_offset()`. Not merged here (NG-2, D-9). Follow-up F-6: decide merge-into-canonical vs. keep-fork-with-stated-reason.
- **Effects-store boundary (D-9/D-10)** — `InMemoryEffectStore` and its three ports (`EffectStateStore`, `EffectDedupStore`, `RetentionMaintenance`) stay in `ego-runtime`, untouched (NG-6, R12, R18). Follow-up **CORE-PERSIST-E** (F-1): relocate the ports first, then consolidate the implementation.
- Also named, not owned by this change: no Postgres consolidation (NG-3 → CORE-PERSIST-C), no conformance-harness expansion (NG-4, KD-4 → CORE-PERSIST-D, F-4).

## Traceability Audit

| Requirement | Covering task(s) |
|---|---|
| R1 — Canonical ownership | 3.1–3.4, 4.1, 7.3, 8.1, 11.2–11.3 |
| R2 — No duplicate declaration | 5.6, 9.7, 12.5, 13.3 |
| R3 — Named fakes not promoted | 12.3, 12.4 |
| R4 — Missing stays missing | 13.4 |
| R5 — Behavior preservation | 3.1–3.4, 5.4, 5.6, 7.3, 9.5, 9.7, 11.2–11.3, 12.5 |
| R6 — Durability/production preservation | 13.5 |
| R7 — Backend neutrality | 1.1, 13.6 |
| R8 — Read-side consolidation | 10.1, 11.2–11.3, 11.4 |
| R9 — Compatibility re-exports | 2.1, 4.1, 5.2, 5.5, 6.1, 8.1, 9.2, 9.6 |
| R10 — Single ownership per port | 13.3 |
| R11 — Dependency integrity | 1.1, 1.4, 5.3, 7.1, 9.4, 13.1, 13.6 |
| R12 — Effects scope integrity | 13.2 |
| R13 — No Postgres refactor | 13.2 |
| R14 — No conformance expansion | Deferred / Out of Scope |
| R15 — No contract redesign | 13.2 |
| R16 — No test double promoted | 8.1, 9.5 |
| R17 — persistent-entity duplicates named | Deferred / Out of Scope |
| R18 — Effect-store boundary named | Deferred / Out of Scope |

**Scope-boundary cross-check against proposal's NG-1..NG-12 — zero findings.** No task touches
`crates/persistent-entity/`, `crates/runtime/`, `crates/effect-store/`, or `crates/persistence/`;
no task adds a `ProjectionStateStore` implementation, a conformance harness, or a Postgres file;
no task promotes `TestClock` or any `Fake*Durable*`/`#[cfg(test)]`-local double.
