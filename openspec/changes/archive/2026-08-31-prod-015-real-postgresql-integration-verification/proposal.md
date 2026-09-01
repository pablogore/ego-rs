# Proposal: PROD-015 — Real PostgreSQL Integration Verification

> Canonical / source of truth. Spanish review companion: `proposal.es.md` (1:1 identifiers).

## Objective

Close the invariants that only a real PostgreSQL — real migrations, real transactions, real
row locks, real concurrency — can demonstrate, and that the workspace currently asserts
nowhere. Every invariant in scope is a PostgreSQL SQL/transaction/concurrency guarantee.
Nothing else is admitted.

## Intent

`integration-tests/` already exists as an independent Cargo workspace with 16 admitted tests
(PROD-012 / PROD-012A / PROD-002-G11). PROD-015 **extends that existing structure** — it does
not create it, does not scaffold a workspace, and does not touch `cargo test --workspace`,
which stays Docker-free.

What remains open is narrow and verified against HEAD, not inherited from a stale backlog:

- **The conformance harnesses are still in-memory only.** `assert_event_store_conformance`
  (`crates/testkit/src/event_store.rs:69`) is driven only by
  `crates/infrastructure/tests/in_memory_event_store_conformance.rs` and
  `crates/persistent-entity/tests/default_store_conformance.rs`;
  `assert_reservation_store_conformance` (`crates/testkit/src/reservation_conformance.rs:963`)
  only by `crates/testkit/src/reservation.rs`. The durable adapters have never been put to the
  same definitions. `integration-tests/README.md` already states reusing them is a convention
  of this suite; it is a convention with no call site.
- **The `events` table's own optimistic concurrency is unguarded.**
  `conflict_from_postgres.rs` loads the *reservations* table's
  `(tenant_id, operation_key)` uniqueness. No test races appends on a stream.
- **The N-contender lease race is unguarded.** `fencing_window_postgres.rs` proves the
  single-contender row-lock re-check. `integration-tests/README.md` itself records the
  many-contender race as still missing.
- **Migration 007's backfill has zero real-PostgreSQL coverage.**
  `crates/persistence/src/postgres/aggregate_type_backfill.rs` is live; only its pure
  `split_aggregate_id` logic is unit-tested. Its transactional behavior is untested.
- **SQL `NULL` tenant semantics are asserted structurally, not behaviorally.**
  `schema_index_assertion.rs` reads the catalog; nothing exercises `Option::None` against
  three-valued logic on the `events` stream identity.

`docs/integration-test-backlog.md` and issue #275 both predate the delivered work and describe
tests that now exist. They are not ground truth for this change; the tree is.

## Active Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D-1 | Identifier is **PROD-015**, not PROD-014 | `PROD-014 — Read-Side Persistence Composition & Durable Store` is already reserved in ROADMAP.md §7.13 and bound by PROD-013 D-5 |
| D-2 | Scope is **PostgreSQL-only**. Transport, HTTP, socket and OTLP verification are excluded entirely (OOS-1) | #275's 13 formal acceptance criteria are PostgreSQL / guard-script / testkit-harness scoped. Transport appears only in the descriptive "Scope by category §7" and in "Subsequent partitions", which the issue itself admits "only if it genuinely requires real infrastructure". Those items are also *hermetic loopback* by this repo's own classification (`skills/testing/SKILL.md` Rule 3, `skills/testing-strategy/SKILL.md` "Self-hosted loopback is not external infrastructure") — they need no container at all, so bundling them here would be wrong on placement as well as on atomicity |
| D-3 | **The `i64::MAX` fencing-exhaustion boundary is resolved OUT of scope — already covered, no new test.** | Checked rather than assumed: `token_for_storage`/`token_from_storage` (`crates/persistence/src/postgres/reservation.rs:107,124`) already carry in-process unit tests at `:626-665` asserting `i64::MAX` is storable and `i64::MAX + 1` is refused. The `BIGINT` + `CHECK (fencing_token > 0)` column shape is covered by the existing schema/catalog category. Real PostgreSQL adds nothing |
| D-4 | **Unit-of-work atomicity is expected to be satisfied by IS-1, not by a bespoke test.** | `assert_event_store_conformance` already asserts drop-without-commit discards, and that a staged append is invisible to a `store.load()` issued outside the unit of work (`crates/testkit/src/event_store.rs:328-375`). Against `PostgreSQLEventStore` that read travels a *different pooled connection*, so it becomes a genuine cross-connection isolation assertion for free. Design confirms the distinct-connection property; only if it does not hold does a separate test become justified |
| D-5 | The reservation harness needs **no testkit change** to run against PostgreSQL | Verified: `assert_reservation_store_conformance` takes a `Fn() -> (S, Arc<TestClock>)` factory, and `PostgresOperationReservationStore::new(pool, clock)` (`reservation.rs:85`) already accepts an injected `Arc<dyn Clock>`. The shapes already fit |
| D-6 | **No test-count target.** Roughly five to six new test files, each earning its slot | #275's own framing ("52 is not a target") and `integration-tests/README.md`'s admission rules. Coverage breadth is explicitly not the goal |
| D-7 | Issue #275 is **not closed by this change alone**; recommend a split | See "#275 handling" below. Executing the split is a separate action, outside this SDD change |
| D-8 | This change is **verification-first**. A defect a new test exposes becomes a named follow-up, not silently absorbed scope — **except** a small, localized fix, and only when it is necessary to satisfy the exact invariant PROD-015 is verifying (IS-4 or IS-2) | A verification spec that always defers fixes can end up demonstrating a known bug without ever closing the guarantee it exists to verify. `design.md` MUST explicitly name the defect, justify that the fix introduces no new capability, and bound the change. Anything needing a new API, new contractual behavior, an architectural change, an additional migration, or a non-trivial solution MUST go to a separate atomic follow-up spec instead — no exception for that (user decision, proposal question round) |
| D-9 | **IS-4 stays in scope at full weight, unconditionally.** Migration 007 / `aggregate_type_backfill.rs` is treated as a supported upgrade path for as long as the project allows migrating a database from an earlier state through it — not conditioned on whether any currently-known deployment has already migrated | A distributed migration's correctness must hold for any installation that crosses it, regardless of today's known deployment state. This forecloses `design.md` investigating "real deployment state" as a gate on IS-4's scope — that state does not determine the migration's correctness (user decision, proposal question round) |
| D-10 | **PG14 stays the real, supported compatibility floor.** The main suite (contention, fencing, UoW, concurrency — IS-1 through IS-6) keeps running on PG16 for speed. A separate, narrow PG14 slice covers only version-sensitive invariants (migrations, SQL/catalog features that could genuinely diverge across versions) — not a full duplication of the suite | The runner provisioning PG16 today is an implementation fact, not a redefinition of the declared minimum. If PG14 is the declared floor, an unverified floor is a verification debt, not an automatic new minimum. Raising the effective minimum to PG16 must be an explicit, separate product/versioning decision, never an accidental consequence of Testcontainers (user decision, proposal question round) |

## Atomicity Gate

**Run, and it cut scope twice.** Readiness probe down/up transitions (#275 §5) were considered
and dropped (OOS-2): that is connection-pool resilience under real network conditions, not a
SQL/transaction/fencing guarantee, and B3.7's unit-level coverage already exists. The
`i64::MAX` boundary was checked rather than deferred and removed as already covered (D-3).
Re-checked after drafting: every remaining in-scope item names a PostgreSQL SQL, transaction,
row-lock or migration behavior. No HTTP, socket, OTLP, readiness, second-broker, general
performance or CI concern survives in scope.

## Scope

### In Scope

- **IS-1** — Drive `assert_event_store_conformance` against `PostgreSQLEventStore` and
  `assert_reservation_store_conformance` against `PostgresOperationReservationStore`, reusing
  the **exact same `ego-testkit` definitions** — never a re-derived or parallel copy. Closes
  #275 AC10.
- **IS-2** — `events`-table optimistic concurrency: a unique violation on stream identity
  surfaces as a conflict reporting the **real** current version, and an N-way concurrent
  append race on one stream leaves exactly one winner.
- **IS-3** — Six contenders racing one expired lease: exactly one wins, and the fencing token
  advances by **exactly one**, not by the number of contenders.
- **IS-4** — Migration 007 / `aggregate_type_backfill.rs` transactional behavior: abort before
  the first `UPDATE` leaves the table byte-identical; a zero-row run commits; a revert rejoins
  exactly the prior state.
- **IS-5** — SQL `NULL` tenant semantics at the `events` stream-identity level: `Option::None`
  under three-valued logic (`NULL = NULL` is not true), behaviorally, not from the catalog.
- **IS-6** — Unit-of-work atomicity — drop-without-commit persists nothing, an open unit of
  work is invisible to a concurrent reader — satisfied through IS-1 unless design proves
  otherwise (D-4).
- **IS-7** — Each new test is admitted under `integration-tests/README.md`'s four admission
  rules, states in its own doc comment the invariant it proves and **why in-process cannot
  show it**, and lands with its ledger row, module registration and category. The end-to-end
  budget is spent (4/4); every test here files under a non-end-to-end category with its own
  stated infrastructure risk.
- **IS-8** — Mutation/adversarial validation for the two highest-criticality invariants
  (IS-3 fencing, IS-4/IS-6 transaction and unit-of-work atomicity): neutralize the mechanism,
  confirm the new test fails and the existing suite stays green. Method is a `design.md`
  decision; this proposal only requires that it happens.
- **IS-9** — A narrow PG14 compatibility slice (D-10): only the version-sensitive
  invariants — migration 007 and any SQL/catalog feature that could genuinely diverge across
  PostgreSQL versions — run against PG14. The main suite (IS-1 through IS-6) stays on PG16.
  `design.md` picks the mechanism (a small separate matrix/slice, not a second full run of the
  suite) and names exactly which tests it covers.

### Out of Scope

- **OOS-1** — Real socket bind and graceful shutdown (`crates/transport`), OTLP wire
  round-trip (`crates/infrastructure`), and CORE-018 real-HTTP end-to-end
  (`examples/reference-app`). Confirmed genuinely missing at HEAD — none of
  `crates/transport/tests/server.rs`, `crates/infrastructure/tests/otlp_export_roundtrip.rs`
  or `examples/reference-app/tests/e2e_register.rs` exists, and
  `examples/reference-app/tests/http_route.rs` uses `tower::oneshot`, no real socket. They are
  non-PostgreSQL and hermetic-loopback-classified, so they need neither this suite nor
  Testcontainers. **Future spec, PROD-016 at naming level only** (D-2).
- **OOS-2** — Readiness probe down/up transition testing (#275 §5, TCP-forwarder mechanism).
  Explicitly decided, not silently dropped: it is connection-pool resilience under real
  network conditions rather than a PostgreSQL guarantee, B3.7 already covers the unit level,
  and the suite's wall-clock budget is better spent on IS-1 through IS-5.
- **OOS-3** — The `i64::MAX` fencing-exhaustion boundary (D-3). Already covered in-process.
- **OOS-4** — Creating the `integration-tests/` workspace, the runner, or the ledger guard.
  All already exist and are extended, not built.
- **OOS-5** — Re-delivering `scripts/detect-integration-tests.sh`'s dead-pathspec fix or its
  six-mutation self-test, `fencing_window_postgres.rs`, or `schema_index_assertion.rs`. Prior
  art this change builds on.
- **OOS-6** — ~~Superseded by IS-9/D-10.~~ Originally recorded as a follow-up; the user decided
  the PG14 floor is real and stays in scope, narrowly, as IS-9. Full duplication of the main
  suite against PG14 is still out of scope — only version-sensitive invariants get the second
  slice.
- **OOS-7** — Fixing production defects the new tests expose, beyond what `design.md` accepts
  explicitly (D-8).
- **OOS-8** — Docker Compose, anywhere, for anything.

## Capabilities

### New Capabilities

- `real-infrastructure-verification`: which invariants MUST be demonstrated against real
  PostgreSQL, the admission contract that keeps the suite small, and the wall-clock budget.

### Modified Capabilities

- `event-store`: the event-store conformance contract's obligation extends to **durable**
  implementations, not only in-memory ones; plus the `events`-table optimistic-concurrency and
  `NULL`-tenant stream-identity requirements (IS-2, IS-5).
- `idempotent-command-processing`: the reservation-store conformance obligation extends to the
  durable adapter, and the many-contender fencing-advance requirement is stated (IS-1, IS-3).

If the spec phase finds an existing requirement already implies one of these, it folds into
`real-infrastructure-verification` rather than manufacturing a delta.

## Approach

Add test files to the existing `integration-tests/tests/infrastructure/` tree, register each
module, and give each its ledger row — the `tests/ledger.rs` guard fails the run otherwise,
in milliseconds and before any container is provisioned.

IS-1 is deliberately first and cheapest: both harnesses already fit their durable adapters
(D-5), so it is a call site plus per-test database isolation, and it retires IS-6 as a side
effect (D-4). IS-2, IS-3 and IS-5 are store-level rather than end-to-end for the same reason
`fencing_window_postgres.rs` is: the evidence *is* precise control of concurrent transactions,
which HTTP cannot express. IS-4 drives `aggregate_type_backfill.rs` directly against a
migrated database.

Existing suite conventions are constraints, not suggestions: one shared PostgreSQL per run,
isolation by schema or database per test, migrations once per run, **no arbitrary sleeps** —
synchronize on a signal or poll `pg_locks` / `pg_stat_activity` with an explicit deadline.

Governing principle for spec and tasks: **every new test must name the exact invariant it
proves and justify why it is undemonstrable in-process, by contract, by conformance, or at
compile time.** A test that cannot answer that is not admitted.

## #275 Handling

**AC-by-AC mapping is a tasks-phase obligation, deliberately not paraphrased here.** The
exploration confirmed #275 has 13 formal acceptance criteria and that AC10 (conformance
harnesses reused against real PostgreSQL) is the substantive remaining gap, closed by IS-1.
The remaining criteria are believed stale-resolved by PROD-012 / PROD-012A / PROD-002-G11.
This proposal does not restate criteria it cannot quote verbatim; `tasks.md` MUST perform the
checkoff against the live issue text, one criterion per row, each with the file that satisfies
it.

Recommended split, as a documented recommendation only:

1. Check off the satisfied criteria on #275 with links to the delivering files.
2. Narrow #275 to what PROD-015 closes, or fork the confirmed-remaining transport items
   (OOS-1) into a new issue.
3. A future spec named PROD-016 owns HTTP / socket / OTLP verification.

**Executing this split is outside this SDD change.** No issue is created, edited or closed by
PROD-015.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `integration-tests/tests/infrastructure/` | New | ~5–6 test files (IS-1 through IS-5) |
| `integration-tests/tests/infrastructure.rs` | Modified | Module registration for each new file — the ledger guard fails without it |
| `integration-tests/README.md` | Modified | One Status row per new test, in a table, path as a code span, under a stated category with its own infrastructure risk; ledger counts updated |
| `crates/testkit/src/{event_store.rs,reservation_conformance.rs}` | Unchanged (expected) | Reused verbatim (D-5). Any change here is a design escalation, not a silent edit |
| `crates/persistence/src/postgres/aggregate_type_backfill.rs`, `migrations/007_*.sql` | Unchanged | Exercised, not modified (D-8) |
| `integration-tests/src/lib.rs` | Modified | Runner extended to provision and own a second PG14 container for the IS-9 compatibility slice |
| `integration-tests/src/main.rs` | Modified | Runner extended to provision and own a second PG14 container for the IS-9 compatibility slice |
| `crates/transport`, `crates/infrastructure`, `examples/reference-app` | Untouched | OOS-1 |
| Root `Cargo.toml`, `cargo test --workspace` | Untouched | The suite stays an independent workspace; the root stays Docker-free |

## Risks

| ID | Risk | Likelihood | Mitigation |
|----|------|------------|------------|
| R-1 | The AC mapping is deferred to tasks, so a criterion could be miscounted as stale-resolved | Med | `tasks.md` MUST check off against the verbatim issue text with a file per criterion. #275 is not closed by this change (D-7), so a miscount cannot silently close a real gap |
| R-2 | Wall-clock budget: ≤5 min for the suite, ≤1–2 min per slice. IS-1 adds two full conformance runs and IS-3 adds six contenders | Med | Compile and execution time reported separately, as the runner already does. If a slice exceeds its budget, the fix is a smaller test, never a raised budget |
| R-3 | The conformance harnesses assert exact stream listings, so a shared database would make them order- and neighbor-dependent | Med | Per-test isolated database, which the suite already provides. Confirm in `design.md` before the first RED |
| R-4 | D-4 assumes `store.load()` on `PostgreSQLEventStore` uses a different pooled connection than the open unit of work | Med | Verified in `design.md` before IS-6 is retired. If it does not hold, IS-6 becomes its own test with its own ledger row |
| R-5 | Suite growth by accretion — the failure mode `integration-tests/README.md` was written to prevent | Med | D-6 (no count target), IS-7 (admission rules per test), and the spent 4/4 end-to-end budget. Categories are not a loophole |
| R-6 | IS-4 or IS-2 exposes a real production defect, and the change absorbs the fix | Med | D-8: verification-first. A defect becomes a named follow-up unless `design.md` accepts a small fix explicitly |
| R-7 | Reviewer load: ~5–6 test files plus README ledger prose can exceed the 400-line budget | Med | `tasks.md` forecasts it and slices into chained PRs — IS-1 is a natural first slice that closes AC10 on its own |
| R-8 | `docs/integration-test-backlog.md` and `skills/testing-strategy/SKILL.md` are both stale at HEAD (the latter claims three transport test files exist; none do) | Low | Recorded here so no later phase treats either as ground truth. Correcting them is not in scope |
| R-9 | IS-9 (PG14 slice) adds a second PostgreSQL version target to the suite, which is exactly the accretion pattern R-5 guards against if scoped loosely | Med | Bounded explicitly by D-10/IS-9 to version-sensitive invariants only — migration 007 and catalog/SQL features that could genuinely diverge. `design.md` names the exact test set; it is never "run everything twice" |

## Rollback Plan

Test-only and additive. Reverting is deleting the new files under
`integration-tests/tests/infrastructure/`, their module registrations, and their README ledger
rows — the ledger guard verifies the three stay consistent in both directions, so a partial
revert fails loudly rather than silently. No production code, no schema, no migration, no data
and no public API is touched, so nothing outside `integration-tests/` can regress. If a
`crates/testkit` change proves unavoidable (D-5 says it should not), it is additive and
reverts with the call site.

## Dependencies

- Existing `integration-tests/` workspace, its runner
  (`cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`), its shared
  container and its `tests/ledger.rs` guard.
- Existing `ego-testkit` conformance harnesses — reused, not modified.
- Existing durable adapters: `PostgreSQLEventStore`, `PostgresOperationReservationStore`,
  `aggregate_type_backfill.rs` and migration 007.
- A reachable Docker daemon for the suite. No new crate, service or external dependency.

## Success Criteria

- [ ] **SC-1** — `assert_event_store_conformance` and `assert_reservation_store_conformance`
      both run against their durable PostgreSQL adapters, using the same `ego-testkit`
      definitions the in-memory callers use. #275 AC10 is closed.
- [ ] **SC-2** — A stale expected version on the `events` table surfaces a conflict reporting
      the real current version, and an N-way append race on one stream leaves exactly one
      winner.
- [ ] **SC-3** — Six contenders racing one expired lease produce exactly one winner and a
      fencing token that advanced by exactly one.
- [ ] **SC-4** — Migration 007's backfill is proven transactional: abort before the first
      `UPDATE` leaves the table byte-identical, a zero-row run commits, and a revert rejoins
      exactly the prior state.
- [ ] **SC-5** — `NULL` tenant semantics on `events` stream identity are asserted
      behaviorally, not from the catalog.
- [ ] **SC-6** — Unit-of-work atomicity holds against real PostgreSQL, whether through IS-1
      (D-4) or through its own test if design requires one.
- [ ] **SC-7** — Every new test states its invariant and its why-not-in-process justification
      in its own doc comment, and `tests/ledger.rs` passes with no drift.
- [ ] **SC-8** — The mutation check for IS-3 and IS-4/IS-6 is recorded: with the mechanism
      neutralized the new test fails. For IS-4/IS-6 (migration 007 / unit-of-work atomicity),
      the rest of the pre-existing suite stays green. For IS-3 (fencing), because the new
      many-contender fencing test shares its load-bearing predicate with the pre-existing
      single-contender fencing test (`fencing_window_postgres.rs`), neutralizing that predicate
      fails both the new test and that pre-existing test — global suite greenness is not the
      claim; every test that does not exercise the shared predicate remains unaffected.
- [ ] **SC-9** — The whole suite finishes within ≤5 minutes, no slice exceeds 1–2 minutes, and
      compile time is reported separately from execution time.
- [ ] **SC-10** — `cargo test --workspace` remains Docker-free and unchanged; the root
      workspace is untouched.
- [ ] **SC-11** — No HTTP, socket, OTLP, readiness-probe or CI work appears anywhere in the
      delivered change, and the PROD-016 recommendation is recorded for the split.
- [ ] **SC-12** — `tasks.md` contains the AC-by-AC checkoff against #275's verbatim text, one
      criterion per row with the satisfying file.
- [ ] **SC-13** — The version-sensitive invariants (migration 007, catalog/SQL features that
      could diverge) are proven against PG14 through a narrow, separate slice — not a second
      full run of the suite — while the main suite (IS-1 through IS-6) stays on PG16.
