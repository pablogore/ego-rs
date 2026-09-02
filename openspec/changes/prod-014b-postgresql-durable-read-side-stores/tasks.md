# Tasks: PROD-014B — PostgreSQL Durable Read-Side Stores

> Canonical / source of truth. Spanish companion: `tasks.es.md` (1:1 identifiers).
> Strict TDD (AD-12): the whole conformance suite (Phase 3) is written RED, against
> `PostgreSQLOffsetStore`/`PostgreSQLDedupStore` types that do not exist yet, before either
> adapter body (Phase 4). Every error assertion names the specific `Fatal`/`Transient`
> variant, never `is_err()`. The only unit-testable surface is `is_fatal` (Phase 2) —
> everything else is proved against real PostgreSQL in `integration-tests/`.

## Review Workload Forecast

**PR boundaries below are confirmed by the change owner** (supersedes the initial Unit-1/2/3
split): PR1 = schema only, PR2 = adapters + `is_fatal` + conformance tests, PR3 = production
adoption. Phase-to-PR tags throughout this document match this confirmed mapping.

| Field | Value |
|-------|-------|
| Estimated changed lines | ~620 total — PR1 ~40 (migrations + registry only), PR2 ~500 (`is_fatal` + both adapters + `mod.rs` re-exports + the 8-case real-PG conformance suite), PR3 ~85 (reference-app wiring + docs) |
| 400-line budget risk | High for PR2 only — an accepted deviation (see Condition 5 below), not a defect. PR1 and PR3 are comfortably under budget |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (schema foundation) → PR 2 (durable adapters, error mapping, exports, conformance tests) → PR 3 (production adoption: reference-app wiring + docs) |
| Delivery strategy | ask-on-risk (session default — not supplied by the orchestrator for this run) |
| Chain strategy | stacked-to-main — PR2 branches from PR1, PR3 branches from PR2 (confirmed by the change owner; not three independent branches off `develop`) |

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: stacked-to-main
400-line budget risk: High

**Review-budget note (confirmed, not re-litigated here):** PR2's ~500 lines exceed the
400-line budget primarily because of its own real-PostgreSQL conformance suite (~280 lines).
Per Condition 5 below, this is an accepted deviation — PR2's implementation is never split
from its own tests to force it under budget. PR1 and PR3 each stay well under 400 on their
own.

### PR Chain Conditions (confirmed by the change owner)

1. **Stacked chain**: PR2 branches from PR1; PR3 branches from PR2. Not three independent
   branches off `develop`.
2. **Independent green**: each PR must compile and pass its own gates on its own branch tip —
   no PR may depend on a *later* PR's code to be green.
3. **One spec, three review units**: the three PRs are review-workload slices of this single
   change, not separate capabilities. No requirement or scenario's coverage is split or
   duplicated across PRs as if they were independent changes — the Traceability Audit below
   stays keyed to the change as a whole.
4. **No misleading interim state**: no PR — including any in-between commit within a PR — may
   introduce a temporary fallback, a fake-durable substitute, or wording implying
   exactly-once semantics. `FakeDurableOffsetStore`/`FakeDurableDedupStore` (existing,
   pre-PROD-014B) are never reused as a stand-in for the real adapters at any commit.
5. **PR2's size is an accepted deviation**: PR2 may moderately exceed the ~400-line budget
   because of its own conformance suite. This is not a defect to work around — PR2's
   implementation is never split from its own tests just to fit under budget.

### Suggested Work Units

| Unit | Goal | PR | Branches from | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----|----------------|----------------------|-----------------|-------------------|
| 1 | PostgreSQL schema foundation: migrations `013`/`014`, registered and ordered. Migration/schema tests only — explicitly no `is_fatal`, no adapter code | PR 1 | `develop` | `cargo test -p ego-persistence migrations` | N/A — schema only, no adapter behavior to prove yet | Delete `013`/`014` and their registry entries; nothing else in the workspace references them |
| 2 | PostgreSQL durable adapters: `is_fatal` classifier, `OffsetStore`/`DedupStore` implementations, `mod.rs` re-exports, and the full real-PG conformance suite (RED before GREEN, per AD-12) | PR 2 | PR 1 | `cargo test -p ego-persistence postgres::` (unit `is_fatal`) + `cargo test -p ego-integration-tests --test read_side_progress_postgres` (conformance: restart-survival, isolation, offset upsert, dedup convergence, `is_durable()`) | Real PostgreSQL via `isolated_database()` (documented architectural exception, root-level `integration-tests/` workspace) | Delete `is_fatal`, both adapter files, their re-exports, and the conformance test file; PR 1's schema stays valid and unused |
| 3 | Production adoption: reference-app wiring under `Profile::Production`, adoption-constraint + operational documentation, final traceability verification | PR 3 | PR 2 | Re-run 3.7 (`is_durable()` + `Profile::Production` acceptance) against the wired `main.rs` path; `cargo test -p reference-app`; `cargo test --workspace` | `examples/reference-app` composing under `Profile::Production` against a real Postgres pool | Revert `ReadSideProgressStores::postgres`, restore `main.rs`'s `None` + the retired "PROD-014A F-1" comment; PR 1–2 remain valid for any other host |

## Phase 1: Migrations (Foundation) — PR 1

- [x] 1.1 Create `crates/persistence/src/postgres/migrations/013_create_projection_offsets.sql`: `projection_offsets(projection_id, tag, tenant, offset_value, updated_at)`, `tenant NOT NULL`, `PRIMARY KEY (projection_id, tag, tenant)` (AD-1). Traces: "Offset Survives a Process Restart", "Tenant Is a Required Part of Offset Identity".
- [x] 1.2 Create `crates/persistence/src/postgres/migrations/014_create_projection_dedup.sql`: `projection_dedup(projection_id, tag, event_id, created_at)`, `PRIMARY KEY (projection_id, tag, event_id)`, no `tenant` column (AD-1, AD-7). Traces: "Repeated Dedup Marks Converge to One Record", "Dedup Identity Is Tenant-Independent".
- [x] 1.3 Register both as `include_str!` constants + two ascending entries in `migrations.rs::migrations()` (AD-2). No new test needed — run `cargo test -p ego-persistence migrations` to confirm the existing `every_migration_file_is_registered_and_every_registration_has_a_file` and `registration_order_ascends_by_numeric_prefix` tests cover `013`/`014` (R-4).

## Phase 2: Shared Error Classification — `postgres/mod.rs` (Exports & Wiring, Part A) — PR 2

- [x] 2.1 RED: `#[cfg(test)]` unit tests in `crates/persistence/src/postgres/mod.rs` asserting `is_fatal` classifies constructed `sqlx::Error::Database` values with SQLSTATE `42P01`/`42703`/`22001`/`23514` and `ColumnDecode`/`Decode` variants as `true`, and pool-timeout/I-O/protocol errors as `false` — no pool constructed (AD-8, AD-12).
- [x] 2.2 GREEN: implement `pub(crate) fn is_fatal(err: &sqlx::Error) -> bool` with the exact SQLSTATE match from AD-8, including its rustdoc explaining why `Transient` is the default.

## Phase 3: Integration & Conformance Tests — RED (`integration-tests/tests/infrastructure/read_side_progress_postgres.rs`) — PR 2

Written entirely before either adapter body exists (AD-12); every case obtains its database
via `ego_integration_tests::isolated_database()` (D-8, SC-10). Compile failure against
not-yet-existing `PostgreSQLOffsetStore`/`PostgreSQLDedupStore` is the expected RED state.

- [x] 3.1 RED: restart-survival — write offset N for `(projection_id, tag, tenant)`, **drop the store and its pool**, open a *new* pool against the same database, rebuild the store, `read_offset` returns N (never in-process state). Traces: "Offset Survives a Process Restart" / scenario "Restart resumes from the last persisted offset"; SC-1, R-3.
- [x] 3.2 RED: absent-offset tenant isolation — offsets exist for tenant A on `(projection_id, tag)`, tenant B was never written; reading tenant B on the same `(projection_id, tag)` returns `None`, never tenant A's value. Traces: "Absent Offset Reads Are Tenant-Isolated"; SC-2, G-4.
- [x] 3.3 RED: offset last-write-wins — write offset N, then write offset M for the same `(projection_id, tag, tenant)` with no ordering coordination between the two writes; the stored value becomes M, with no error and no conflict signal to either write. Traces: "Offset Writes Are Last-Write-Wins"; SC-7 (every offset statement binds `tenant`, asserted as part of this test's setup).
- [x] 3.4 RED: dedup sequential double-mark — `mark_seen` called twice sequentially for the same `(projection_id, tag, event_id)`; both calls `Ok`, `SELECT COUNT(*)` is exactly 1, `seen()` returns `true`. Traces: "Repeated Dedup Marks Converge to One Record".
- [x] 3.5 RED: dedup concurrent double-mark — two `mark_seen` calls for the same identity run via `tokio::join!`; both `Ok`, `SELECT COUNT(*)` is exactly 1, `seen()` returns `true`. Test doc comment states explicitly this proves **storage-level convergence of two calls on one identity**, not execution exclusion, not exactly-once handling, and not multi-replica safety — the delivered guarantee is single-writer-per-`(projection_id, tag, tenant)` (AD-6; spec's "Durable Dedup Bookkeeping Does Not Imply Exactly-Once Handler Execution"). Traces: "Repeated Dedup Marks Converge to One Record".
- [x] 3.6 RED: dedup tenant-independence — `event_id` marked seen under tenant A; `seen()` under tenant B for the same `(projection_id, tag)` returns `true`. Traces: "Dedup Identity Is Tenant-Independent".
- [ ] 3.7 RED: durability + production-profile acceptance — both `is_durable()` return `true`; `build_runtime_with(…, Some(ReadSideProgressStores::postgres(pool)))` builds successfully under `Profile::Production`, with no change to the gate's own validation logic. This case stays RED until Phase 6 lands `ReadSideProgressStores::postgres`; re-run at 8.2 to confirm final GREEN. Traces: "Both Progress Stores Report Themselves As Durable", "The Reference Application's Production Path Uses the Durable Pair"; SC-5, SC-6.
- [x] 3.8 RED: unapplied-migration classification — `read_offset` against a database with no migration applied returns `OffsetStoreError::Fatal`, not `Transient` (AD-8's replacement for a `probe()` method, AD-9).

## Phase 4: Adapters — GREEN (`crates/persistence/src/postgres/`) — PR 2

- [x] 4.1 GREEN: create `read_side_offset.rs` — `PostgreSQLOffsetStore { pool: PgPool }`, `pub fn new(pool: PgPool)`, manual `Debug` (pool only), `is_durable() -> true`, `write_offset` as the AD-3 upsert (`ON CONFLICT (projection_id, tag, tenant) DO UPDATE SET offset_value = EXCLUDED.offset_value, updated_at = NOW()`), `read_offset` as the AD-4 `fetch_optional` scalar lookup; both map errors via `is_fatal` into `OffsetStoreError::{Fatal,Transient}`. Turns 3.1, 3.2, 3.3, 3.8 GREEN (3.7 partially — offset half only).
- [x] 4.2 GREEN: create `read_side_dedup.rs` — `PostgreSQLDedupStore { pool: PgPool }`, `pub fn new(pool: PgPool)`, manual `Debug`, `is_durable() -> true`, `mark_seen` as the AD-5 `INSERT … ON CONFLICT (projection_id, tag, event_id) DO NOTHING`, `seen` as the AD-5 primary-key point lookup; both map errors via `is_fatal` into `DedupStoreError::{Fatal,Transient}`. Turns 3.4, 3.5, 3.6 GREEN (3.7 partially — dedup half only).

## Phase 5: Adapter Exports — `postgres/mod.rs` (Exports & Wiring, Part B) — PR 2

- [x] 5.1 Add `pub use read_side_offset::PostgreSQLOffsetStore;` and `pub use read_side_dedup::PostgreSQLDedupStore;` to `crates/persistence/src/postgres/mod.rs` (IS-4). Confirms 3.1–3.6 and 3.8 compile and pass against the real adapter types.

## Phase 6: Reference-App Production Wiring (`examples/reference-app/`) — PR 3

- [ ] 6.1 Add `pub fn postgres(pool: PgPool) -> Self` to `ReadSideProgressStores` in `src/read_side/mod.rs`, alongside `in_memory()`/`fake_durable()`, wiring `Arc::new(PostgreSQLOffsetStore::new(pool.clone()))` / `Arc::new(PostgreSQLDedupStore::new(pool))` (AD-10). Rustdoc states the single-writer-per-`(projection_id, tag, tenant)` adoption constraint verbatim per AD-10's snippet. Traces: "The Single-Writer Adoption Constraint Is Documented at the Adapter Level"; IS-5, IS-8.
- [ ] 6.2 In `src/main.rs`: take `pool.clone()` **before** `EntityEventStores::open(pool)` (resolves EC-2), build `read_side_progress = ReadSideProgressStores::postgres(pool.clone())` immediately after `migrations::run(&pool)` (line 77), and pass `Some(read_side_progress)` into `build_runtime_with(...)`, deleting the `None` + retired "PROD-014A F-1" comment. Traces: "The Reference Application's Production Path Uses the Durable Pair"; IS-6, SC-6.

## Phase 7: Adoption-Constraint & Operational Documentation — PR 3

- [ ] 7.1 Rustdoc on `PostgreSQLOffsetStore`/`PostgreSQLDedupStore` (or a shared module doc in `read_side_offset.rs`/`read_side_dedup.rs`): state the single-writer-per-`(projection_id, tag, tenant)` adoption constraint and that no multi-replica projection configuration is officially supported (D-7, IS-8, SC-8, SC-12). Traces: "The Single-Writer Adoption Constraint Is Documented at the Adapter Level", "Prevention of Double Handler Execution Rests on an Explicit, Unenforced Single-Writer Adoption Constraint".
- [ ] 7.2 Rustdoc operational note on `PostgreSQLDedupStore`: `projection_dedup` grows unbounded and monotonically with unique events processed; no purge/TTL/eviction ships in this change; row count is a signal to observe, not a surprise (D-4, L-4, AD-11). Traces: "Dedup Storage Growth Is Unbounded In This Capability".
- [ ] 7.3 Create `crates/persistence/README.md` (does not exist today) documenting the durable read-side pair, and update `ARCHITECTURE.md`'s `Profile::Production` / Persistence Completeness Rule section (~line 197-210) with the same single-writer adoption constraint plus the named follow-up **PROD-014C — Atomic Read-Side Event Claiming** (D-7, F-1, SC-9). Traces: "Durable Dedup Bookkeeping Does Not Imply Exactly-Once Handler Execution", "Prevention of Double Handler Execution Rests on an Explicit, Unenforced Single-Writer Adoption Constraint", "The Concurrency Gap Has a Named, Distinct Follow-Up".

## Phase 8: Final Verification — PR 3

- [ ] 8.1 `cargo test --workspace` zero new failures; `cargo clippy --workspace -- -D warnings` clean; confirm no touched function exceeds cognitive-complexity 10.
- [ ] 8.2 Re-run the full conformance suite (`cargo test -p ego-integration-tests --test read_side_progress_postgres`); confirm 3.1–3.8 all GREEN, including 3.7 now that Phase 6 exists.
- [ ] 8.3 Diff-read confirmation (no code change): SC-7 — every `$N` bound, no interpolation anywhere; SC-11 — `crates/domain/src/read_side/`, `crates/service-sdk`'s gate/registration, and `crates/runtime/src/read_side/scheduler.rs` appear in no file list of this change.

## Traceability Audit

All 13 spec requirements mapped to at least one covering task:

| Requirement | Capability | Covering task(s) |
|---|---|---|
| Offset Survives a Process Restart | `read-side-durable-progress` | 1.1, 3.1, 4.1 |
| Absent Offset Reads Are Tenant-Isolated | `read-side-durable-progress` | 1.1, 3.2, 4.1 |
| Repeated Dedup Marks Converge to One Record | `read-side-durable-progress` | 1.2, 3.4, 3.5, 4.2 |
| Dedup Identity Is Tenant-Independent | `read-side-durable-progress` | 1.2, 3.6, 4.2 |
| Offset Writes Are Last-Write-Wins | `read-side-durable-progress` | 3.3, 4.1 |
| Both Progress Stores Report Themselves As Durable | `read-side-durable-progress` | 3.7, 4.1, 4.2 |
| Tenant Is a Required Part of Offset Identity | `read-side-durable-progress` | 1.1, 4.1 |
| Dedup Storage Growth Is Unbounded In This Capability | `read-side-durable-progress` | 7.2 (no cleanup code shipped anywhere in Phases 1–6, confirmed at 8.3) |
| The Reference Application's Production Path Uses the Durable Pair | `read-side-durable-progress` | 3.7, 6.1, 6.2 |
| The Single-Writer Adoption Constraint Is Documented at the Adapter Level | `read-side-durable-progress` | 6.1, 7.1 |
| Durable Dedup Bookkeeping Does Not Imply Exactly-Once Handler Execution | `read-side` (ADDED) | 3.5, 7.3 |
| Prevention of Double Handler Execution Rests on an Explicit, Unenforced Single-Writer Adoption Constraint | `read-side` (ADDED) | 6.1, 7.1, 7.3 |
| The Concurrency Gap Has a Named, Distinct Follow-Up | `read-side` (ADDED) | 7.3 |

**Scope-boundary cross-check against spec's Non-Goals and design's OOS references — zero
findings.** No task in this list touches: dedup retention/TTL/cleanup (explicitly ruled
out — Phase 7's docs only state the limitation, no code path exists in any phase); atomic
event claiming/reservation, leader election, locks, leases, or fencing (OOS-2 — no task
adds one; Phase 3's concurrent-mark_seen test (3.5) explicitly disclaims proving exclusion);
multi-replica detection of any kind (OOS-2/OOS-7 — no task adds one); or any backend other
than PostgreSQL (OOS-4 — every task targets `crates/persistence/src/postgres/`). All of
these are reserved for **PROD-014C — Atomic Read-Side Event Claiming** (named at 7.3, not
implemented here) or backlog (F-2 retention), per D-4, D-7, OOS-2, OOS-3, OOS-4.
