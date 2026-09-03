# Tasks: STOOLAP-S1 — First-Class Stoolap `Repository` Adapter

> Canonical / source of truth. Spanish companion: `tasks.es.md` (1:1 identifiers).
> Strict TDD (`openspec/config.yaml` → `apply.tdd: true`): every slice's RED is either a
> compile failure naming a path that does not exist yet, or a new unit test naming a function
> that does not exist yet (design AD-11). No task in this change is a characterization test —
> every RED below asserts genuinely new behavior; nothing here re-documents already-passing code.
> Slice order is design AD-11's mandatory S1 (harness) → S2 (crate) → S3 (third subject), each
> independently compiling workspace-wide before the next starts (mid-flight rollback property,
> proposal Rollback Plan).
>
> **OQ-1 note**: this design implements the documented fresh-aggregate semantics (EC-1) and
> excludes that scenario from the shared harness (spec R6). No task reconciles
> `PostgreSQLRepository`; that is F-5, filed separately (NG-9/R11).
> **OQ-2 note**: resolved in `spec.md` R12 — single-owning-process guarantee only; no task claims
> multi-process safety.

## Review Workload Forecast

Measured baselines (not estimates): `crates/persistence/src/postgres/repository.rs` 214 lines ·
`integration-tests/tests/infrastructure/repository_tenant_scoping_postgres.rs` 213 lines (the
3 tests the harness generalizes to 11 scenarios) · `crates/testkit/src/event_store.rs` ~268
lines (the closest existing harness template, design AD-8 criterion 1).

| Field | Value |
|-------|-------|
| Estimated changed lines | ~815–995 total — S1 ~340–400 (harness + 2 call sites + doc comment), S2 ~430–530 (crate + schema + `save`/`load`/`delete` + 7 colocated unit tests — the largest slice), S3 ~45–65 (dev-deps + 1 test file) |
| 400-line budget risk | High for the combined total and for S2 alone; S1 borderline High; S3 Low |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (S1 — harness + Memory + PostgreSQL runs) → PR 2 (S2 — crate, schema, CAS) → PR 3 (S3 — Stoolap harness run) |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending — user decision needed (recommend stacked-to-main, matching AD-11's mandatory S1→S2→S3 order and per-slice mid-flight rollback) |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

**Named fallback seam (design AD-11 criterion 3)**: if S2 exceeds budget in practice, split it
into S2a (`new` + schema + `load`/`delete`) and S2b (`save`'s CAS algorithm), moving the R3
systemwide-duplicate proof to S2b. Recorded as a decision, not an improvisation.

### Suggested Work Units

| Unit | Goal | PR | Branches from | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----|----------------|----------------------|-----------------|-------------------|
| 1 | S1 — shared harness, proven green against Memory and PostgreSQL before any Stoolap code exists (RK-4) | PR 1 | `develop` | `cargo test -p ego-testkit --test repository_conformance_memory` | `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` (real PostgreSQL) | Drop `repository_conformance.rs`, its `mod`/`pub use` pair, and both call-site test files; `ego-testkit`/`integration-tests` still build |
| 2 | S2 — `ego-persistence-stoolap` crate: schema, `save`/`load`/`delete`, error mapping | PR 2 | PR 1 | `cargo test -p ego-persistence-stoolap` | `cargo test -p ego-persistence-stoolap -- race_between_two_transactions_is_a_conflict` (real two-transaction MVCC race against a temp Stoolap file) | Drop `crates/persistence-stoolap/`, remove the workspace member and `layers.toml` entry; PR 1 stays valid |
| 3 | S3 — Stoolap becomes the harness's third subject | PR 3 | PR 2 | `cargo test -p ego-persistence-stoolap --test repository_conformance` | Same command — real embedded Stoolap DB at a fresh `tempfile::TempDir` path | Drop the test file and the two dev-dependency lines; PR 1–2 remain valid |

## Phase 1: RED — Harness Call Sites Before the Harness Exists — S1 — PR 1

- [x] 1.1 Create `crates/testkit/tests/repository_conformance_memory.rs`: build `InMemoryRepository<ConformanceAggregate, _>`, call `ego_testkit::assert_repository_conformance`. Fails to compile — neither symbol exists yet (AD-11).
- [x] 1.2 Create `integration-tests/tests/infrastructure/repository_conformance_postgres.rs`: build `PostgreSQLRepository<ConformanceAggregate, _>` against `isolated_database()`, call the same harness function; register one `mod repository_conformance_postgres;` line in `integration-tests/tests/infrastructure.rs`. Fails to compile — same reason.

## Phase 2: GREEN — The Shared Harness and the Memory Run — S1 — PR 1

- [x] 2.1 Create `crates/testkit/src/repository_conformance.rs`: `ConformanceAggregate`, `conformance_aggregate(value: &str)`, and `assert_repository_conformance<R: Repository<ConformanceAggregate> + ?Sized>(repository: &mut R)` implementing the 11 scenarios (design AD-8 table) — spec R1, R2, R3, R4, R5, R7, R8. Doc comment names the 4 deliberate exclusions: fresh+nonzero `expected_version` (EC-1, spec R6), durability, concurrency, payload shape.
- [x] 2.2 Export from `crates/testkit/src/lib.rs`: `pub mod repository_conformance;` plus `pub use` for the three public items, alongside the three existing conformance harnesses.
- [x] 2.3 Confirm 1.1 now compiles and passes — all 11 scenarios green against `InMemoryRepository` (spec R2 subject 1).

## Phase 3: RED+GREEN — The PostgreSQL Run — S1 — PR 1

- [x] 3.1 Confirm 1.2 now compiles and passes against real PostgreSQL — all 11 scenarios green (spec R2 subject 2).
- [x] 3.2 Confirm `repository_tenant_scoping_postgres.rs` still passes unmodified — its 3 tests are now a subset of the shared harness (design AD-9 criterion 4).

## Phase 4: Verification — S1 — PR 1

- [x] 4.1 `cargo test -p ego-testkit` passes with the Memory conformance run green.
- [x] 4.2 `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` passes with the PostgreSQL conformance run green.
- [x] 4.3 Confirm zero new dependency edges: `crates/persistence-memory/**` untouched (EC-2); `crates/testkit/Cargo.toml` and `integration-tests/Cargo.toml` unchanged (AD-9 criteria 1–2).
- [x] 4.4 Confirm the harness's doc comment states all 4 exclusions in full (AD-8 criterion 5) and that the fresh+nonzero scenario appears nowhere in the scenario list (spec R6, second scenario).

## Phase 5: Foundation — Crate Skeleton & Layer Gate — S2 — PR 2

- [x] 5.1 Create `crates/persistence-stoolap/Cargo.toml` (package `ego-persistence-stoolap`): normal deps `ego-persistence-api` (path), `stoolap = "0.4"`, `serde = "1"`, `serde_json = "1"` — exactly the D-3/AD-1 set, no dev-dependencies yet (design AD-11 defers `ego-testkit`/`tempfile` to S3).
- [x] 5.2 Create `src/lib.rs`: crate doc, `pub mod persistence;`, `pub use persistence::repository::StoolapRepository;` (AD-2). No `#![deny(missing_docs)]`.
- [x] 5.3 Create `src/persistence/mod.rs`: `pub mod repository;`.
- [x] 5.4 Add `layers.toml` entry `"ego-persistence-stoolap" = "infrastructure"`. Do not open `xtask/src/layers.rs` (D-2, AD-1).
- [x] 5.5 Add `"crates/persistence-stoolap",` to the root `Cargo.toml` workspace members.

## Phase 6: RED — DSN & Tenant-Sentinel Unit Tests — S2 — PR 2

- [x] 6.1 Write failing unit test `dsn_carries_full_sync`: `dsn_for(Path::new("/tmp/x")) == "file:///tmp/x?sync=full"`. Fails — `dsn_for` does not exist yet (AD-4; threat matrix "Durability").
- [x] 6.2 Write failing unit test `encode_tenant_maps_only_the_absent_scope_to_the_sentinel`: `encode_tenant(None) == ""`, `encode_tenant(Some("t")) == "t"`. Fails — `encode_tenant`/`SYSTEMWIDE_SCOPE` do not exist yet (AD-3; threat matrix "Tenant isolation"/"Sentinel leakage").

## Phase 7: GREEN — Schema, DSN, Tenant Sentinel — S2 — PR 2

- [x] 7.1 Add the `CREATE_AGGREGATES_TABLE` DDL constant (Schema section): `tenant_id`, `aggregate_id`, `version`, `payload`, `UNIQUE (tenant_id, aggregate_id)`, no `PRIMARY KEY` anywhere (EC-3).
- [x] 7.2 Implement `SYSTEMWIDE_SCOPE` and `encode_tenant()` — turns 6.2 green (AD-3).
- [x] 7.3 Implement `dsn_for()` — turns 6.1 green (AD-4).
- [x] 7.4 Implement `struct StoolapRepository<A, F>`, its `Debug` impl (prints `db.dsn()`, not the handle), and `pub fn new(path: &Path, deserialize: F) -> Result<Self, PersistenceError>` opening via `dsn_for` and executing the DDL (AD-4; OQ-3's fallible-constructor divergence, recorded not hidden).

## Phase 8: RED — Durability & CAS Unit Tests — S2 — PR 2

- [x] 8.1 Write failing unit test `an_opened_repository_requested_full_sync`: a thin `dsn()` accessor over `Database::dsn()` equals `dsn_for(path)`. Fails — no accessor yet (EC-6, spec R5 first half).
- [x] 8.2 Write failing unit test `a_committed_save_survives_close_and_reopen` (path under `std::env::temp_dir()` with a per-test unique suffix — no `tempfile` dep until S3). Fails — `save`/`load` not implemented (spec R9).
- [x] 8.3 Write failing unit test `two_systemwide_saves_leave_exactly_one_row`: save the same `aggregate_id` twice under `None`, assert one row and `version == 2`. Fails — `save` not implemented (spec R3; the failure a nullable column would have permitted).
- [x] 8.4 Write failing unit test `a_stale_expected_version_is_a_conflict`. Fails — `save` not implemented (spec R5).
- [x] 8.5 Write failing unit test `race_between_two_transactions_is_a_conflict`: two real transactions racing on one row, asserting `Conflict` not `Internal`. Fails — `save`/`is_write_conflict` not implemented (spec R10, R12; AD-7's brittle arm).

## Phase 9: GREEN — `save`/`load`/`delete` and Error Mapping — S2 — PR 2

- [x] 9.1 Add the `dsn()` accessor — turns 8.1 green (EC-6).
- [x] 9.2 Add `SELECT_VERSION`/`INSERT_AGGREGATE`/`UPDATE_AGGREGATE` statement constants (`$n`-parameterized, tuple binding — threat matrix "SQL construction") and implement `save()`'s 7-step algorithm (AD-5): resolve+encode tenant, real transaction, CAS read, absent-row+nonzero-expected ⇒ `Conflict` (EC-1), version-guarded write, re-read on `affected == 0`, commit.
- [x] 9.3 Implement `is_write_conflict()` (AD-7): `UniqueConstraint`, `TransactionAborted`, `LockAcquisitionFailed`/`DatabaseLocked` ⇒ `Conflict`; the pinned `Internal` message-text arm for the MVCC write-claim (EC-7); fail-loud default (`Internal`) for everything else.
- [x] 9.4 Implement `LOAD_PAYLOAD`/`DELETE_AGGREGATE` and `load()`/`delete()` (AD-6): plain `=` predicates, `NotFound` on absent row / zero rows affected (spec R7, R8).
- [x] 9.5 Confirm 8.1–8.5 all pass green.

## Phase 10: Verification — S2 — PR 2

- [x] 10.1 `cargo build -p ego-persistence-stoolap` succeeds standalone.
- [x] 10.2 `cargo test -p ego-persistence-stoolap` passes — all 7 colocated unit tests green.
- [x] 10.3 `cargo run -p xtask -- verify-layers` passes: new crate mapped, `infrastructure → domain` edge, no matrix edit (R8, proposal).
- [x] 10.4 Grep gate: `rg '""' crates/persistence-stoolap/src` returns exactly one non-test line (AD-3 criterion 1, threat matrix "Sentinel leakage"); no `sqlx`/`PgPool`/`ego-persistence`/`postgres`/migration token anywhere under the crate (R7, D-11); no `async`/`tokio`/`block_in_place`/`spawn_blocking` token in the crate (D-4).
- [x] 10.5 Confirm exactly one `impl Repository<...> for StoolapRepository` and no trait of its own declared in the crate (R10, proposal).

## Phase 11: RED — Stoolap Becomes the Harness's Third Subject — S3 — PR 3

- [ ] 11.1 Add `[dev-dependencies]` to `crates/persistence-stoolap/Cargo.toml`: `ego-testkit = { path = "../testkit" }`, `tempfile = "3"` (AD-9, AD-11 S3).
- [ ] 11.2 Create `crates/persistence-stoolap/tests/repository_conformance.rs`: build `StoolapRepository<ConformanceAggregate, _>` at a fresh `tempfile::TempDir` path, call `ego_testkit::assert_repository_conformance`. Fails to compile until 11.1 lands (AD-11 S3 RED).

## Phase 12: GREEN + Whole-Change Verification — S3 — PR 3

- [ ] 12.1 `cargo test -p ego-persistence-stoolap` passes — all 11 harness scenarios green against `StoolapRepository` (spec R1, R2 subject 3).
- [ ] 12.2 `cargo test --workspace` passes with no container runtime available; confirm no Testcontainers/Docker dependency anywhere in the root workspace (spec R2 combined with proposal R9, NG-8).
- [ ] 12.3 Confirm `repository_tenant_scoping_postgres.rs` still passes unmodified (R11, proposal).
- [ ] 12.4 Diff-read: `crates/persistence-api/**`, `crates/persistence/**`, `crates/runtime/**`, `crates/effect-store/**` absent from the whole-change file list (R6, R7, proposal NG-4/NG-6/KD-2).
- [ ] 12.5 Record F-5 and F-6 in the PR description as named follow-ups (R14, proposal); confirm KD-1..KD-4 remain accurately stated and untouched.

## Deferred / Out of Scope (named debt, not tasks)

- **KD-1** — `Snapshot`/`OffsetStore`/`DedupStore` remain without a shared conformance harness. No task adds one (proposal NG-1, F-1).
- **KD-2** — The effect-store's Stoolap provider stays at the non-fsync default. Not changed here (proposal, observed only).
- **KD-3 → F-5** — `PostgreSQLRepository` ignores `expected_version` on a fresh aggregate (EC-1); `StoolapRepository` conflicts, matching the trait's documentation. Reconciliation is its own change with its own review (NG-9/R11).
- **KD-4** — `sync=full` is asserted at the DSN; genuine fsync is trusted to Stoolap, not fault-injection-tested (design AD-4). No task attempts a crash-recovery test.
- **F-2, F-3, F-4, F-6** — backend abstraction, CORE-PERSIST-A2 relocation, the persistence-crate rename, and a selectable sync mode all stay unscheduled (proposal NG-2/NG-4/NG-6, design AD-4 criterion 2).

## Traceability Audit

| Spec requirement | Covering task(s) |
|---|---|
| R1 — Adapter exists, round-trips | 2.1, 9.2, 9.4, 12.1 |
| R2 — One shared suite, three subjects | 1.1, 1.2, 2.3, 3.1, 12.1, 12.2 |
| R3 — Tenant isolation incl. systemwide | 2.1, 8.3, 9.2 |
| R4 — Empty tenant rejected, never coerced | 2.1 (scenario 8) |
| R5 — Stale version ⇒ truthful conflict | 2.1, 8.4, 9.2 |
| R6 — Fresh+nonzero excluded from shared suite, documented reason | 2.1, 4.4 |
| R7 — Absent load/delete ⇒ NotFound | 2.1, 9.4 |
| R8 — Delete is permanent | 2.1, 9.4 |
| R9 — Survives unclean restart | 8.2, 9.2 |
| R10 — One conflict outcome for both races | 8.5, 9.3 |
| R11 — No internal detail ever visible | 2.1, 10.4 |
| R12 — Single-process guarantee only | 8.5, 9.3, 12.2 |

**Scope-boundary cross-check against proposal NG-1..NG-9 — zero findings.** No task touches
`crates/persistence-api/`, `crates/persistence/`, `crates/runtime/`, or `crates/effect-store/`;
no task adds a second Stoolap-backed store, a `StorageEngine`/dialect abstraction, or a fourth
backend; no task fixes `PostgreSQLRepository`'s or `InMemoryRepository`'s existing behavior.
