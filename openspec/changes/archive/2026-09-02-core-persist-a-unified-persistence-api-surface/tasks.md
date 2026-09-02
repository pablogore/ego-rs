# Tasks: CORE-PERSIST-A — Unified Persistence API Surface (Domain-Owned Ports)

> Canonical / source of truth. Spanish companion: `tasks.es.md` (1:1 identifiers).
> Strict TDD: each PR's `reexport_identity.rs` grows RED (new identity witnesses fail to
> resolve) before its own GREEN relocation, per design.md's Testing Strategy. Relocated
> `#[cfg(test)]` modules move verbatim with their file — assertion count before/after must
> match exactly (D-6, SC-3). Slice order is design AD-6's mandatory
> `read_side/` → `operation/` → `persistence/` (EC-3), each independently compiling
> workspace-wide before the next starts.

## Review Workload Forecast

**Estimate only — not confirmed by a change owner conversation.** Based on explore §11's
1,500–2,000 total relocated-line estimate (verbatim moves count full add+delete even with
zero logic change) and design AD-6's three-slice split.

| Field | Value |
|-------|-------|
| Estimated changed lines | ~1,600–2,000 total — PR1 ~350–500 (7 small leaf files + skeleton + layers.rs test), PR2 ~700–950 (largest: `reservation.rs` is the biggest single file plus the `id_type!` macro relocation), PR3 ~550–750 (`event_store.rs`, `repository.rs`, `event.rs`, plus final whole-change diff checks) |
| 400-line budget risk | High for all three slices — verbatim relocation with doc comments and existing tests moves full text, not a summary diff |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (S1 — read side) → PR 2 (S2 — operation) → PR 3 (S3 — persistence) |
| Delivery strategy | ask-on-risk (session default) |
| Chain strategy | **confirmed** — strict stacked chain, merged in order PR1 → PR2 → PR3, each built on the previous; PR2 branches from PR1, PR3 branches from PR2, matching AD-6's "each slice compiles workspace-wide before the next starts" (change owner decision, 2026-09-02) |

Decision needed before apply: No — resolved by change owner (2026-09-02)
Chained PRs recommended: Yes
Chain strategy: confirmed — strict stacked chain, PR1 → PR2 → PR3, merged in order, never three independent PRs against `develop`
400-line budget risk: High — PR2 exception conditionally pre-approved (see note below)

**Review-budget note:** every slice is expected to exceed 400 lines because relocation is
verbatim (D-6) — no line is rewritten to shrink the diff. Splitting further than S1/S2/S3
would violate AD-6's item-closure boundaries (EC-3): `persistence/` needs S2's
`OperationKey`/`OperationReceipt`, so it cannot move before `operation/`. If PR2 exceeds
budget most severely, that is an accepted deviation of the same shape as PROD-014B's PR2 —
never split an item's definition from its own relocated tests to force it under budget.

**PR2 budget exception (change owner decision, 2026-09-02):** conditionally pre-approved —
the excess over 400 lines must come exclusively from mechanical `operation/` file relocation
plus the `id_type!` macro move (Phase 7), with zero behavior mixed in. If a diff-read finds
any change in PR2 that is not move/import/re-export, the exception is void: stop and re-split
rather than merge. Splitting the macro artificially away from the rest of `operation/` to fit
the budget is rejected — it would fragment one item's own relocated tests (same principle as
the review-budget note above).

### Semantic Zero-Diff Gate (change owner decision, 2026-09-02)

Each PR must demonstrate **semantic zero-diff** before merge — `cargo build`/`cargo test`
passing is necessary but not sufficient. Per PR, before merge:

- Diff the public signatures (`pub fn`/`pub struct`/`pub trait`/`pub enum`/`pub const`/…) of
  every relocated item at its old path vs. its new path — must be byte-identical apart from
  the module path itself.
- Diff `ego_domain`'s externally-visible re-export surface (every path still reachable from
  `ego_domain::*` after the PR) before vs. after — identical paths, identical visibility.
- Diff the assertion count of every relocated `#[cfg(test)]` module before vs. after —
  identical (already required by SC-3/SC-5; restated here as an explicit gate condition).
- **Any change that is not move/import/re-export halts execution immediately** — stop the
  task, do not continue to the next task, phase, or PR, and surface it to the change owner
  before proceeding.

### Suggested Work Units

| Unit | Goal | PR | Branches from | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----|----------------|----------------------|-----------------|-------------------|
| 1 | S1 — read side: crate skeleton, `layers.toml`, AD-1 gate relaxation + its test, `read_side/{offset,dedup,store,projection_state,event_tag,state,event_stream}`, module re-exports | PR 1 | `develop` | `cargo build -p ego-persistence-api && cargo test -p xtask` | N/A — structural relocation, no runtime behavior to prove (OOS-6) | Drop `crates/persistence-api/`, restore the 7 `ego-domain` modules, remove the `Cargo.toml`/`layers.toml` edges and `layers.rs` relaxation |
| 2 | S2 — operation: `operation/{key,receipt,reservation}`, `id_type!` macro + `TenantId` relocation, module re-exports | PR 2 | PR 1 | `cargo build -p ego-persistence-api && cargo test --workspace` | N/A — structural relocation, no runtime behavior to prove (OOS-6) | Drop the 3 relocated `operation/` files + macro, restore `ego-domain`'s originals; PR 1 stays valid and unused by anything outside this slice |
| 3 | S3 — persistence: `persistence/{error,event_store,repository,snapshot,stored_event,tenant}`, `event.rs`/`DomainEvent`, module re-exports, final whole-change diff verification | PR 3 | PR 2 | `cargo build -p ego-persistence-api && cargo test --workspace` | N/A — structural relocation, no runtime behavior to prove (OOS-6) | Drop the 7 remaining relocated files, restore `ego-domain`'s originals; PR 1–2 remain valid for any other consumer |

## Phase 1: Crate Skeleton & Layer Gate (Foundation) — PR 1

- [x] 1.1 RED: add `#[cfg(test)]` in `xtask/src/layers.rs` asserting `domain → domain` passes `check_direction` and `domain → foundation`/`domain → infrastructure`/`domain → sdk` still fail, following the existing `graph_from`/`layers_from` test shape (`layers.rs:164-208`). Fails to compile/pass against today's `Some(&[])` (AD-1, SC-7).
- [x] 1.2 GREEN: `xtask/src/layers.rs:76` — change `"domain" => Some(&[])` to `"domain" => Some(&["domain"])`. Turns 1.1 green.
- [x] 1.3 Create `crates/persistence-api/` skeleton: `Cargo.toml` (package `ego-persistence-api`, deps derived from `ego-domain`'s block per AD-5 — no workspace `path` dependency), `src/lib.rs`. Add it as a workspace member in the root `Cargo.toml`.
- [x] 1.4 Add `layers.toml` entry: `"ego-persistence-api" = "domain"` (IS-5, FR-001).
- [x] 1.5 Add one `path` dependency edge in `crates/domain/Cargo.toml` on `ego-persistence-api` (D-2, the only new crate-graph edge this change makes).

## Phase 2: RED — Re-export Identity Test, S1 Items — PR 1

- [x] 2.1 Create `crates/persistence-api/tests/reexport_identity.rs` with one identity witness per S1 item (`OffsetStore`, `Offset`, `OffsetStoreError`, `DedupStore`, `DedupStoreError`, `ReadSideStore`, `ReadSideStoreError`, `ProjectionStateStore`, `ProjectionStateStoreError`, `EventTag`, `ProjectionState`, `EventStreamElement`) — object-safe traits get an identity coercion, generic items a `where`-clause witness carrying both bounds. Fails to compile: none of these paths exist in `ego_persistence_api::*` yet (IS-6, SC-1).

## Phase 3: GREEN — Relocate `read_side/` Files — PR 1

- [x] 3.1 Move `crates/domain/src/read_side/offset.rs` verbatim (doc comments, `#[cfg(test)]`, the `Arc<T>` forwarding impl at line 92) to `crates/persistence-api/src/read_side/offset.rs` (D-6, SC-4).
- [x] 3.2 Move `read_side/dedup.rs` verbatim (including the `Arc<T>` forwarding impl at line 60) to `crates/persistence-api/src/read_side/dedup.rs` (SC-4).
- [x] 3.3 Move `read_side/store.rs` verbatim to `crates/persistence-api/src/read_side/store.rs`.
- [x] 3.4 Move `read_side/projection_state_store.rs` verbatim to `crates/persistence-api/src/read_side/projection_state.rs` — zero implementations, zero consumers, unchanged (D-8, AD-7).
- [x] 3.5 Move `read_side/event_tag.rs`, `read_side/state.rs`, `read_side/event_stream.rs` verbatim to the same paths under `crates/persistence-api/src/read_side/` (AD-2, EC-1).

## Phase 4: GREEN — Module Re-exports & Consumer Check, S1 — PR 1

- [x] 4.1 Replace each vacated `crates/domain/src/read_side/{offset,dedup,store,projection_state_store,event_tag,state,event_stream}.rs` with a module re-export (`pub use ego_persistence_api::read_side::{...};`), leaving existing item-level `pub use` lines byte-identical (AD-4, D-5).
- [x] 4.2 Confirm `read_side/scheduler.rs:5-10`, `session.rs:5-13`, `runner.rs:3-10` compile unedited — module re-export resolves their `super::`/`crate::` imports without any change (IS-4 collapses to zero edits, per AD-4).

## Phase 5: Verification — PR 1

- [x] 5.1 `cargo build -p ego-persistence-api` succeeds standalone (FR-005, AD-5).
- [x] 5.2 `cargo build --workspace` succeeds; turns 2.1's identity witnesses green (IS-6, SC-1).
- [x] 5.3 `cargo run -p xtask -- verify-layers` passes: `ego-persistence-api` mapped, edge permitted, no cycle (SC-6).
- [x] 5.4 `cargo test --workspace` passes, zero new failures, zero changed assertion counts in the moved `#[cfg(test)]` modules (SC-3, SC-5).
- [x] 5.5 Diff-read: no `use` or `Cargo.toml` edit outside `crates/domain/` and `crates/persistence-api/` (SC-2).
- [x] 5.6 Semantic zero-diff gate: diff public signatures and `ego_domain`'s re-export surface for every S1 item, old path vs. new path — identical apart from module path. Any non-move/import/re-export change halts the PR (change owner gate, 2026-09-02).

## Phase 6: RED — Re-export Identity Test Extension, S2 Items — PR 2

- [x] 6.1 Extend `reexport_identity.rs` with witnesses for `OperationKey`, `OperationKeyError`, `OperationFingerprint`, `OperationKeyHash`, `MAX_LEN`, `OperationReceipt`, `AggregateOutcome`, `AggregateOutcomeError`, `OperationReservationStore`, `ReservationError`, `ReserveRequest`, `ReservationOutcome`, `Lease`, `OwnerFence`, `FencingToken`, `OldestCompleted`, `OperationId`, `OwnerId`, `StoredServiceResponse`, `TenantId`, `TenantIdError`. Fails to compile until Phase 7/8 land (IS-6, SC-1).

## Phase 7: GREEN — Relocate `operation/` Files & `id_type!` Macro — PR 2

- [x] 7.1 Move `crates/domain/src/operation/key.rs` verbatim to `crates/persistence-api/src/operation/key.rs` (includes `MAX_LEN`, `OperationFingerprint`, `OperationKeyHash`, D-7).
- [x] 7.2 Move `operation/receipt.rs` verbatim to `crates/persistence-api/src/operation/receipt.rs`.
- [x] 7.3 Move `operation/reservation.rs` verbatim to `crates/persistence-api/src/operation/reservation.rs`.
- [x] 7.4 Move the `macro_rules! id_type` block (`context.rs:7-54`) verbatim into `ego-persistence-api`, add `#[macro_export]`, and invoke it there to generate `TenantId`/`TenantIdError` (AD-3, EC-2). One definition of the generator, not two.

## Phase 8: GREEN — Module Re-exports & Macro Re-invocation, S2 — PR 2

- [x] 8.1 Replace `crates/domain/src/operation/{key,receipt,reservation}.rs` with module re-exports of `ego_persistence_api::operation::{key,receipt,reservation}` (AD-4).
- [x] 8.2 `crates/domain/src/context.rs`: remove the local `id_type!` definition, re-invoke the re-exported macro for `AggregateId`, `EntityId`, `CorrelationId`, `CausationId`, `RequestId`; re-export `TenantId`/`TenantIdError` at `ego_domain::context::TenantId` and `ego_domain::TenantId` (`lib.rs:103-107`) (AD-3).

## Phase 9: Verification — PR 2

- [x] 9.1 `cargo build -p ego-persistence-api` succeeds standalone.
- [x] 9.2 `cargo build --workspace` succeeds.
- [x] 9.3 `cargo run -p xtask -- verify-layers` still passes; `cargo test --workspace` zero new failures, zero changed assertion counts (SC-3, SC-5, SC-6). **`verify-layers` passes (18 crates, 0 violations). One trybuild golden required updating outside the strict two-crate boundary: `ego-service-sdk`'s `tests/compile_fail/cross_tenant_permit_new_external.stderr` hardcoded rustc's synthesized "help: provide the arguments" text, which spells a moved type's canonical defining path (`ego_domain::context::TenantId` → `ego_persistence_api::context::TenantId`), not its re-exported path. Change owner authorized a scoped one-file exception to SC-2 for this golden-only, diagnostics-text-only update (2026-09-02) — `TenantId`'s public signature, fields, and behavior are unchanged. Regenerated via `TRYBUILD=overwrite`, diff confirmed to touch only the two `TenantId` path occurrences; `cargo test -p ego-service-sdk` and full `cargo test --workspace` green.**
- [x] 9.4 Diff-read: still no `use`/`Cargo.toml` edit outside the two crates; confirm exactly one `id_type!` definition workspace-wide (SC-2, spec's "only one `id_type!` definition exists workspace-wide" scenario). Confirmed: only `Cargo.lock` (generated) changed outside `crates/domain/`/`crates/persistence-api/`; exactly one `macro_rules! id_type` in the workspace (`crates/persistence-api/src/context.rs`).
- [x] 9.5 Semantic zero-diff gate: diff public signatures and `ego_domain`'s re-export surface for every S2 item + the relocated `id_type!` macro, old path vs. new path — identical apart from module path. This is also the check that decides the PR2 budget exception above: any non-move/import/re-export change voids the exception and halts the PR (change owner gate, 2026-09-02). **PASS** — `key.rs`/`receipt.rs`/`reservation.rs` are byte-identical old vs. new path (verified via `diff`); the `id_type!` macro block is byte-identical apart from the explicitly-authorized `#[macro_export]` addition; `lib.rs`'s `TenantId`/`TenantIdError` re-export block and `operation/mod.rs`'s item-level `pub use` lines are byte-identical to the branch point. Total diff vs. branch point: 12 files, +277/-91 (well under both the 400-line default budget and the 1000-line ledger cap — git's rename detection on the verbatim `git mv`s keeps the authored diff small).

## Phase 10: RED — Re-export Identity Test Extension, S3 Items — PR 3

- [x] 10.1 Extend `reexport_identity.rs` with witnesses for `PersistenceError`, `EventStore`, `EventStoreUnitOfWork`, `Repository`, `Snapshot`, `StoredEvent`, `resolve_tenant`, `DomainEvent` — the file now covers all 35 items (design EC-4). Fails to compile until Phase 11/12 land. `resolve_tenant` (a bare function, not a type) gets a function-pointer equality `#[test]` instead of an identity coercion. Confirmed RED via `cargo test -p ego-persistence-api --test reexport_identity` (10 `E0433`/`E0432` errors before Phase 11/12).

## Phase 11: GREEN — Relocate `persistence/` Files & `event.rs` — PR 3

- [x] 11.1 Move `crates/domain/src/persistence/error.rs` verbatim to `crates/persistence-api/src/persistence/error.rs`.
- [x] 11.2 Move `persistence/event_store.rs` verbatim (async-trait shape byte-identical, OOS-4) to `crates/persistence-api/src/persistence/event_store.rs`.
- [x] 11.3 Move `persistence/repository.rs` verbatim to `crates/persistence-api/src/persistence/repository.rs`.
- [x] 11.4 Move `persistence/snapshot.rs` verbatim to `crates/persistence-api/src/persistence/snapshot.rs`.
- [x] 11.5 Move `persistence/stored_event.rs` verbatim to `crates/persistence-api/src/persistence/stored_event.rs`.
- [x] 11.6 Move `persistence/tenant.rs` verbatim (the `resolve_tenant` three-way rule unchanged, OOS-5) to `crates/persistence-api/src/persistence/tenant.rs`.
- [x] 11.7 Move `crates/domain/src/event.rs` verbatim (62 lines, `chrono` + `serde_json` deps) to `crates/persistence-api/src/event.rs` (AD-2). `serde_json` moves to `persistence-api`'s `[dependencies]` alongside it (`snapshot.rs`'s `Value` payload also needs it); `ego-domain` keeps its own `serde_json`/`chrono` edges since other domain modules still use both (unlike S2's `sha2`, this is not a single-consumer drop). `persistence-api/src/operation/mod.rs` gained item-level `pub use key::OperationKey;`/`pub use receipt::OperationReceipt;` so the verbatim-moved `event_store.rs`/`stored_event.rs` (which refer to them via the bare `crate::operation::*` path, matching what `ego-domain`'s own `operation/mod.rs` already provides) resolve in the new crate.

## Phase 12: GREEN — Module Re-exports, S3 — PR 3

- [x] 12.1 Replace `crates/domain/src/persistence/{error,event_store,repository,snapshot,stored_event,tenant}.rs` and `crates/domain/src/event.rs` with module re-exports of `ego_persistence_api::{persistence::{...}, event}`, leaving existing item-level `pub use` lines byte-identical (AD-4). `cargo build --workspace` and `cargo build -p ego-persistence-api` standalone both pass; `cargo test -p ego-persistence-api --test reexport_identity`: 1 passed, 0 failed (RED to GREEN).

## Phase 13: Verification & Whole-Change Diff Checks — PR 3

- [x] 13.1 `cargo build -p ego-persistence-api` succeeds standalone; `cargo build --workspace` succeeds. **PASS.**
- [x] 13.2 `cargo run -p xtask -- verify-layers` passes; `cargo test --workspace` zero new failures, zero changed assertion counts across every relocated `#[cfg(test)]` module (SC-3, SC-5, SC-6). **PASS** — `verify-layers: OK (18 crates, 0 violations)`. `cargo test --workspace`: every `test result:` line reports `0 failed` (no compile errors, no panics). `persistence::stored_event::tests` (3 tests) and `persistence::tenant::tests` (4 tests) landed in `ego-persistence-api`'s 70-test unit suite with the same test names and counts as their pre-move `ego-domain` originals — zero changed assertion counts. The only new test in the whole diff is `reexport_identity.rs`'s `resolve_tenant_old_path_is_the_new_path_function` (RED→GREEN witness, not a relocated module).
- [x] 13.3 Diff-read over the whole three-PR change: zero SQL/migration/schema file in the diff (SC-8, OOS-3); `crates/runtime/`, `crates/effect-store/`, and every OOS-1-named implementation struct are absent from every file list (SC-9); no crate outside `ego-domain`/`ego-persistence-api` has an edited `use` or added `Cargo.toml` dependency across all three PRs combined (SC-2). **PASS** — `git diff --name-only 885d1da..HEAD` (pre-change baseline to end of PR3) lists only: `Cargo.lock`, root `Cargo.toml` (workspace member registration), `layers.toml`/`xtask/src/layers.rs` (PR1's FR-002 domain-self-edge layer config, no `use`/dependency edit), `crates/domain/**`, `crates/persistence-api/**`, the six `openspec/changes/core-persist-a-unified-persistence-api-surface/*.md` artifacts, and `crates/service-sdk/tests/compile_fail/cross_tenant_permit_new_external.stderr` — PR2's change-owner-authorized golden-data-file exception (2026-09-02), not a `use` statement or `Cargo.toml` edit, so it does not violate this check's literal criterion. No SQL/migration file, no `crates/runtime/`, no `crates/effect-store/`, no implementation struct anywhere in the list.
- [x] 13.4 Confirm KD-1 (`ProjectionStateStore`), KD-2 (`PostgreSQLRepository`'s `ON CONFLICT`/tenant defect), KD-3 (`persistent-entity/src/types.rs`), KD-4 (conformance asymmetry) are unmodified — carried, not fixed or deleted (SC-11). **PASS** — none of `crates/persistence/`, `crates/persistent-entity/`, or any conformance-harness file appear in the `885d1da..HEAD` file list; `KD-1`'s `ProjectionStateStore` relocated dead in PR1 (S1) and is untouched by PR3.
- [x] 13.5 Semantic zero-diff gate: diff public signatures and `ego_domain`'s re-export surface for every S3 item + `event.rs`, old path vs. new path — identical apart from module path. Any non-move/import/re-export change halts the PR — this is the final gate before the whole three-PR change is apply-complete (change owner gate, 2026-09-02). **PASS** — `error.rs`, `event_store.rs`, `repository.rs`, `snapshot.rs`, `stored_event.rs`, `tenant.rs`, and `event.rs` are byte-identical old vs. new path (verified via `diff`); `ego_domain::persistence::mod.rs`'s item-level `pub use error::PersistenceError`/`event_store::{EventStore, EventStoreUnitOfWork}`/`repository::Repository`/`snapshot::Snapshot`/`stored_event::StoredEvent`/`tenant::resolve_tenant` lines and `lib.rs`'s `pub use event::DomainEvent;` line are byte-identical to the branch point. Total diff vs. branch point (`9e5fca2..HEAD`): 14 files, +138/-22. **CORE-PERSIST-A apply-complete** — S1 (PR1), S2 (PR2), and S3 (PR3) have all landed; only Phase 14 (post-merge `sdd-archive` traceability) remains, out of scope for this session.

## Phase 14: Post-Merge Traceability — Future `sdd-archive`

- [x] 14.1 `sdd-archive` merged `openspec/changes/core-persist-a-unified-persistence-api-surface/spec.md`'s `persistence-api-surface` (NEW) capability into `openspec/specs/persistence-api-surface/spec.md`, and merged the `foundation-integrity` (MODIFIED) delta's `FR-002` block into `openspec/specs/foundation-integrity/spec.md` with a dev-dependency clarification (2026-09-02).

## Traceability Audit

All spec requirements mapped to at least one covering task:

| Requirement | Capability | Covering task(s) |
|---|---|---|
| Every Relocated Item Moves Verbatim | `persistence-api-surface` | 3.1–3.5, 7.1–7.4, 11.1–11.7 |
| Old Path Resolves To The Same Item | `persistence-api-surface` | 4.1, 8.1, 12.1, 2.1, 6.1, 10.1 |
| Trait Shape Is Byte-Identical | `persistence-api-surface` | 3.1–3.3, 7.2–7.3, 11.2, 13.2 |
| `Arc<T>` Forwarding Impls Move Intact | `persistence-api-surface` | 3.1, 3.2, 5.4 |
| The `id_type!` Macro Relocates And Is Reinvoked, Not Duplicated | `persistence-api-surface` | 7.4, 8.2, 9.4 |
| No Consumer Outside The Two Crates Is Edited | `persistence-api-surface` | 4.2, 5.5, 9.4, 13.3 |
| `ego-persistence-api` Depends On No Workspace Crate | `persistence-api-surface` | 1.3, 5.1, 9.1, 13.1 |
| Known-Dead Items Relocate Without New Behavior | `persistence-api-surface` | 3.4, 13.4 |
| FR-002 — Dependency Direction Enforcement (domain self-edge) | `foundation-integrity` (MODIFIED) | 1.1, 1.2 |

**Scope-boundary cross-check against proposal's OOS-1..14 — zero findings.** No task in this
list touches an implementation struct (OOS-1), `ego-runtime`/`ego-effect-store` (OOS-2), SQL
or migrations (OOS-3), a trait signature (OOS-4), tenant semantics (OOS-5), a crate merge
(OOS-7), a new capability (OOS-8), `ProjectionStateStore` deletion (OOS-9), a conformance
harness (OOS-14), or `PostgreSQLRepository`'s `42P10` defect (OOS-12, KD-2, F-2 — a
standalone follow-up, not gated on this series).
