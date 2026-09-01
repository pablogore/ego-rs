# Design: PROD-015 — Real PostgreSQL Integration Verification

> Canonical / source of truth. Spanish review companion: `design.es.md` (1:1 identifiers).

## Technical Approach

Five new files under `integration-tests/tests/infrastructure/`, each with a module
registration and a README ledger row. No new harness, no new isolation mechanism, no new
runner: the suite already provisions one PostgreSQL per run, clones a migrated template into
a per-test database, and forbids sleeping. This change adds call sites and one shared
synchronisation helper, plus one bounded runner extension for the PG14 slice (IS-9).

Names corrected against HEAD: the durable event store is `PostgreSQLEventStore<E, F>`
(`crates/persistence/src/postgres/event_store.rs:53`), not `PostgresEventStore` as the
proposal writes it. `PostgresOperationReservationStore` is correct as written.

| File | Invariants | Category |
|------|-----------|----------|
| `durable_store_conformance_postgres.rs` | IS-1, IS-6 | Durable-adapter conformance |
| `events_identity_race_postgres.rs` | IS-2, IS-5 | PostgreSQL concurrency invariants |
| `lease_contention_postgres.rs` | IS-3 | PostgreSQL concurrency invariants |
| `aggregate_type_backfill_postgres.rs` | IS-4 | Migration transactional behaviour |
| `pg14_compatibility.rs` | IS-9 | Version-floor compatibility |

## Architecture Decisions

### AD-1 — Provisioning and sharing: one PG16 container owned by the runner, unchanged

**Choice.** `integration-tests/src/main.rs` already owns the entire lifecycle and stays the
owner. Ordering per run, unchanged for the five new files:

```
run-suite (cargo run --bin run-suite)
  1. cargo test --test ledger --no-run      hermetic; a build failure is Unavailable, not Diverged
  2. cargo test --test ledger               drift = exit 101, nothing provisioned
  3. Postgres::default().with_tag("16").start()
  4. CREATE DATABASE ego_template; migrations::run(&template); close both pools
  5. cargo test --test infrastructure       child, told only EGO_IT_PG_HOST / EGO_IT_PG_PORT
  6. container.rm().await                   inside a live runtime, on every path
  7. exit with exactly the suite's code
```

Every test binary that shares it is exactly one: `tests/infrastructure`. The five new files
are modules inside that single target (`tests/infrastructure.rs`), so they share the
container by construction, not by convention. `tests/ledger` is a second target and shares
nothing — it is hermetic on purpose.

**Alternatives considered.** A per-file container (what the suite was rebuilt away from); a
process-wide `OnceCell` holding the container inside the test binary.

**Rationale.** The `OnceCell` shape was measured and leaked three containers in three runs:
libtest has no suite-level teardown, so the async `Drop` runs at process exit with no runtime
to drive it. That is why the runner exists, and nothing here changes it.

### AD-2 — Isolation: database-per-test, cloned from the template. Unchanged.

**Choice.** Each `#[tokio::test]` calls `ego_integration_tests::isolated_database()`, gets
`ego_test_{n}` cloned from `ego_template` (migrated once, at step 4 above), and calls
`db.close()` at the end. Concurrency is bounded by a semaphore at `MAX_LIVE_DATABASES = 8`,
which bounds connections rather than serialising the suite.

**Alternatives considered.** Schema-per-test with a `search_path` discipline.

**Rationale.** Rejected already, for a reason PROD-015 depends on: several tests here scan
whole tables with no `WHERE`, and within their own database that query is correct. This
change adds three more such tests — `SELECT count(*) FROM events`, the backfill's own
whole-table digest, and the reservation harness's `purge_completed_before` — so schema
isolation would force every one of them to be rewritten to survive its harness. It also
resolves **R-3** directly: `assert_event_store_conformance` asserts exact
`list_aggregate_ids` listings, which is only meaningful in a database no neighbour can
reach.

Two sizing constraints this design pins, because the default is not enough:

- The conformance test's store pool MUST have `max_connections >= 2` (design uses 4). See
  AD-4.
- The IS-3 test's store pool MUST have `max_connections >= 6` (design uses 8) and the IS-2
  test's `>= 4` (design uses 6). Pools opened directly from `db.url()` rather than through
  `db.pool()` are closed by the test itself before `db.close()`, as `src/lib.rs` requires.

### AD-3 — Determinism: a shared `wait_until_blocked` helper, promoted into `src/lib.rs`

**Choice.** Extract `fencing_window_postgres.rs`'s private `wait_until_contender_is_blocked`
into `ego_integration_tests::wait_until_blocked(observer, statement_like, expected)`. It
polls with an explicit deadline and **fails the test** at the deadline; the sleep inside it
is a poll interval, never a timeout standing in for a condition.

```sql
SELECT count(DISTINCT pid) FROM pg_stat_activity
WHERE wait_event_type = 'Lock'
  AND state = 'active'
  AND datname = current_database()     -- NEW, see below
  AND query ILIKE $1
  AND pid <> pg_backend_pid()
```

Two corrections over the existing copy, both load-bearing:

1. **`datname = current_database()` is added.** `pg_stat_activity` is cluster-wide, and up to
   eight isolated databases are live at once. Today only one test blocks on
   `operation_reservations`, so the omission is latent; IS-3 adds a second, and without this
   predicate either test could be satisfied by the other's backend and pass having forced
   nothing open. This is a defect in the suite's own fixtures, not in production code, so
   D-8 does not apply.
2. **The statement fragment is narrowed from a table name to a statement.** IS-3 matches
   `'%UPDATE operation_reservations%'`, not `'%operation_reservations%'`. This is what makes
   the count *evidence*: a contender counted here has already passed its `INSERT … ON
   CONFLICT DO NOTHING` and its `SELECT`, so reaching `expected = 6` proves all six read the
   expired row before any of them wrote.

**IS-3, six contenders, orchestrated rather than raced.**

```
holder (test_pool tx):  SELECT owner_id … WHERE operation_key = $1 FOR UPDATE   -- row lock held
6 × contender (task):   store.reserve(owner-b_i, …)
                          INSERT … ON CONFLICT DO NOTHING   -> 0 rows, does not wait
                          SELECT … (plain, MVCC)            -> sees token T, lease expired
                          UPDATE … WHERE fencing_token = T
                                    AND lease_until <= now  -> BLOCKS on the row lock
test:                   wait_until_blocked(observer, '%UPDATE operation_reservations%', 6)
holder:                 COMMIT                              -- releases, changes nothing
6 × contender:          UPDATE proceeds; the CAS admits exactly one
```

Assertions: exactly one `TakenOver`; five `OtherInProgress`; `fencing_token = T + 1`, never
`T + 6`; `owner_id` is the single winner's. Without the poll the test would be probabilistic,
because six contenders could resolve serially — the first winning, the other five reading an
already-renewed lease and returning `OtherInProgress` without ever contending. The
assertions would still pass and would prove nothing. The deadline turns that into a loud
failure instead of a green run.

`INSERT … ON CONFLICT DO NOTHING` is specifically why the first statement does not block:
`DO NOTHING` takes no lock on a *committed* conflicting row and does not wait on it, unlike
`DO UPDATE`. The design does not rest on that reading being right — if it were wrong, the
narrowed statement fragment means the poll never reaches 6 and the deadline fails the test
by name, rather than passing on an unforced window.

**IS-2/IS-5, N-way append race**, the same shape with a table lock instead of a row lock:

```
holder:      BEGIN; LOCK TABLE events IN EXCLUSIVE MODE;   -- blocks INSERT, allows plain SELECT
4 × racer:   store.append(type, id, tenant, 0, [event])
               SELECT COALESCE(MAX(version),0)  -- ACCESS SHARE, not blocked, sees 0
               INSERT INTO events …             -- ROW EXCLUSIVE, BLOCKS
test:        wait_until_blocked(observer, '%INSERT INTO events%', 4)
holder:      COMMIT
```

`EXCLUSIVE` conflicts with `ROW EXCLUSIVE` and not with `ACCESS SHARE`, which is exactly the
window this needs. After release, one racer commits and the other three take `23505` on
`ux_events_identity_tenant`, drop their aborted transaction, re-read the stream **on a
different pooled connection**, and report `Conflict { expected: 0, actual: 1 }`
(`event_store.rs:198-247`). IS-5 is the identical race with `tenant = None`, which loads
`ux_events_identity_systemwide` — the partial index that exists only because
`NULLS NOT DISTINCT` is PostgreSQL 15+ — plus one direct-SQL duplicate insert under a NULL
tenant required to fail `23505`, and one insert of the same `(aggregate_type, aggregate_id,
version)` under a concrete tenant required to succeed.

**Scope correction, and it is a real one.** SC-2's first clause — "a stale expected version
surfaces a conflict reporting the real current version" — is already satisfied by IS-1: the
shared harness asserts exactly that (`crates/testkit/src/event_store.rs:124-149`) and it
travels `append`'s *pre-check* branch, which needs no race. Re-asserting it here would
violate admission rule 3. What no existing test reaches is the **post-`23505` re-read**: the
branch where the transaction is already aborted and `actual` must come from another
connection. IS-2 is scoped to that branch only. Flagged for spec cross-review.

### AD-4 — IS-1 reuse, and the D-4 verdict: **confirmed, IS-6 needs no test of its own**

**Choice.** One file, two `#[tokio::test]` functions, both against the same shared
definitions:

```rust
// event store
let pool = connect(db.url(), 4).await;                       // >= 2 is mandatory, see below
let mut store = PostgreSQLEventStore::open(pool, deserialize).await?;
assert_event_store_conformance(&mut store, |kind| ConformanceEvent { … }).await;

// reservation store — the factory must be `Copy`, so it captures `&PgPool`, never a PgPool
let pool = &connect(db.url(), 4).await;
assert_reservation_store_conformance(|| async move {
    sqlx::query("TRUNCATE operation_reservations").execute(pool).await.unwrap();
    let clock = Arc::new(TestClock::new(epoch()));
    (PostgresOperationReservationStore::new(pool.clone(), clock.clone()), clock)
}).await;
```

Three concrete constraints the signatures impose, verified rather than assumed:

- `assert_reservation_store_conformance<S, F, Fut>(fresh: F) where F: Fn() -> Fut + Copy`
  (`reservation_conformance.rs:963`). `Copy` forbids capturing an owned `PgPool`; a shared
  reference is `Copy`, so the closure captures `&PgPool`. D-5's stated shape
  (`Fn() -> (S, Arc<TestClock>)`) is close but not exact — the factory is **async**.
- `fresh()` is called **21 times**, and the purge group asserts whole-table counts
  (`removed == 0` at `:937`), so a factory that did not reset would carry earlier scenarios'
  rows into later assertions. `TRUNCATE operation_reservations` is the reset: this store owns
  exactly one table, so truncating it is both sufficient and cheap. A fresh isolated database
  per call was rejected — 21 serialised `CREATE DATABASE`s against one template, for no extra
  isolation.
- Each new file defines its own local `ConformanceEvent`. That is a fixture, not a
  re-derived contract; the `crates/infrastructure` copy is private to another crate's test
  target and cannot be imported. The **harness** is shared, which is what IS-1 requires.

**D-4 verdict: the assumption holds.** Traced at HEAD:
`PostgreSQLEventStore::begin()` calls `self.pool.begin()` (`event_store.rs:414-424`), which
checks out its own pooled connection and holds it for the unit of work's life;
`PostgreSQLEventStore::load()` calls `.fetch_all(&self.pool)` (`event_store.rs:274`), which
checks out a **different** connection. The harness's "an uncommitted append must not be
visible to a reader" (`event_store.rs:356-363`) and the matching receipt assertion
(`:493-501`) therefore become genuine cross-connection `READ COMMITTED` isolation assertions
against real PostgreSQL, for free. **IS-6 gets no separate test and no separate ledger row.**

The assertion is two-sided by construction, so it cannot pass vacuously: if the reader
somehow shared the writer's connection it would *see* the staged rows and the pre-commit
assertion would fail; the post-commit assertion then requires the same rows to appear. One
mechanism, both directions.

**The `max_connections >= 2` constraint is load-bearing, not hygiene.** With a pool of one,
`load()` would wait for the connection the open unit of work is holding and fail as a pool
timeout — an isolation test that fails for a reason unrelated to isolation. Pinned at 4.

**D-8 note.** No path in this design pre-authorises a fix. The most likely defect locus is
`PostgresEventStoreUnitOfWork::confirm_receipt`'s conflicting-fingerprint branch, which the
harness requires to be `Conflict` and which has never been run against real PostgreSQL. A
defect there is found by IS-1, and D-8's small-fix exception covers only IS-2 and IS-4 — so
it becomes a named follow-up spec, not absorbed scope.

### AD-5 — IS-4: the transactional guarantee is the rollback, not the abort

**Choice.** Four cases in one file, each comparing a whole-table digest taken before and
after:

```sql
SELECT md5(string_agg(events::text, '|' ORDER BY id)) FROM events
```

`events::text` renders the entire row as a composite literal, so every column is included and
a column added later is covered without editing the test — the same structural-exhaustiveness
idea the harness uses when it destructures `StoredEvent` without `..`.

| Case | Setup | Expected |
|---|---|---|
| C1 Aborted | one row whose `aggregate_id` matches no registered type | `Aborted(NoRegisteredTypeMatches)`, `rows_rewritten: 0`, digest unchanged |
| C2 RolledBack | a stream with versions 1 and 3 (a hole), splitting cleanly | `RolledBack(StreamVersionsAreNotConsecutiveFromOne)`, digest unchanged, `aggregate_type` **still nullable** |
| C3 Zero rows | empty `events` | `Committed`, `rows_scanned: 0`, and `information_schema.columns` now reports `is_nullable = 'NO'` |
| C4 Revert | seed → digest → backfill (`Committed`) → `revert_aggregate_type_column` | digest identical to the pre-backfill one, column gone |

**Rationale, and it changes what IS-4 must contain.** C1's guarantee comes from *ordering*,
not from the transaction: `backfill_aggregate_type`'s abort closure runs `drop(tx)` before any
`UPDATE` is issued (`aggregate_type_backfill.rs:269-288`), and the source says so. So a test
of C1 alone would be described as proving transactional atomicity while proving statement
ordering. C2 is the only case where rows are genuinely written and then discarded, which is
the property only a real transaction can have and only a real PostgreSQL can demonstrate.
C2 is therefore mandatory, and the proposal's IS-4 wording (abort / zero-row / revert) does
not name it. Flagged for spec cross-review.

C3's `is_nullable` assertion matters because `SET NOT NULL` is the last statement before the
commit; without it, "a zero-row run commits" is satisfied by a run that commits nothing.

### AD-6 — IS-9: a second, version-pinned container owned by the same runner

**Choice.** The runner starts a **second** container, `Postgres::default().with_tag("14")`,
publishes `EGO_IT_PG14_HOST` / `EGO_IT_PG14_PORT`, and reclaims **both** on every exit path
before returning the suite's code. `src/lib.rs` gains
`pg14_database() -> IsolatedDatabase`, which creates a database on that container and — this
is the point of the slice — runs `ego_persistence::postgres::migrations::run()` against it
directly. **No template, no clone**: the migration run *is* the invariant, so it cannot be
pre-applied once and inherited.

Exactly one test file uses it. Exactly four assertions run there:

| # | Assertion | Why it is version-sensitive |
|---|---|---|
| T0 | `current_setting('server_version_num')::int` is in `[140000, 150000)` | Anti-vacuity. Without it a tag typo runs the "PG14 slice" on PG16 and IS-9 proves nothing — the same three-empty-sets failure the ledger guard has its own control for |
| T1 | The full migration set 001–012 applies cleanly | 008, 011 and 012 exist *because* `NULLS NOT DISTINCT` is PostgreSQL 15+ and the floor is 14; 008's own comment records that it is a syntax error on the 14 image. 012 additionally uses `DELETE … USING (SELECT … ROW_NUMBER() OVER …)` |
| T2 | A duplicate `(aggregate_type, aggregate_id, version)` under `tenant_id IS NULL` is refused with `23505` | The behavioural consequence of that decision. The catalog assertion pins the index's shape; only an insert proves PG14 enforces it |
| T3 | Migration 007 + `backfill_aggregate_type` commit path + `revert_aggregate_type_column` round trip | `ALTER TABLE … SET NOT NULL` inside a transaction, then `DROP COLUMN`, on the floor version |

**Explicitly not on PG14:** IS-1, IS-2, IS-3, IS-6, IS-4's C1/C2 abort and rollback cases,
and all sixteen pre-existing tests. Those exercise transaction, lock and pool behaviour that
does not diverge across 14 and 16; running them twice is exactly the accretion **R-9** names.

**Alternatives considered.** (a) A separate `pg14` test target with its own runner step —
rejected: a second lifecycle for one file, and the ledger guard would need a second source.
(b) Gating the slice behind an environment variable — rejected on D-10's own argument: an
opt-in verification of the declared floor is verification debt wearing a flag.
(c) The test starting its own container — rejected: it is the one thing this suite's
conventions forbid outright.

**Cost.** One extra container start (~1.8s measured for the PG16 one) plus one migration run
(~0.5s). Reported on the existing timing line, which gains the PG14 provisioning and
reclamation instants.

### AD-7 — IS-8: the mutation procedure, as a repeatable recipe

Per mutation, exactly these steps, recorded as a row in `integration-tests/README.md`'s
existing mutation table:

```
1. shasum -a 256 <file>                                     record BEFORE
2. apply the exact named edit
3. cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite
4. record: which tests failed, by name, with the assertion message
5. git checkout -- <file>
6. shasum -a 256 <file>                                     MUST equal step 1
7. re-run step 3                                            negative control: all green
```

| ID | Target | Exact edit | Expected |
|---|---|---|---|
| M1a | `reservation.rs` takeover `UPDATE` | `.bind(now)` → `.bind(now - chrono::Duration::days(3650))`, neutralising `lease_until <= $7` | `fencing_window_postgres` **fails**; `lease_contention_postgres` stays green |
| M1b | same `UPDATE` | `AND fencing_token = $6` → `AND $6 = $6` | whole suite green |
| M1c | same `UPDATE` | both of the above together | `lease_contention_postgres` **fails** with six `TakenOver` where one is required; `fencing_window_postgres` also fails |
| M2 | `aggregate_type_backfill.rs`, the `StreamVersionsAreNotConsecutiveFromOne` branch | `tx.rollback().await?` → `tx.commit().await?` | `aggregate_type_backfill_postgres` C2 **fails** on the digest; every other test green |
| M3 | `event_store.rs`, `PostgresEventStoreUnitOfWork::append` | `.execute(&mut *self.tx)` → `.execute(&self.pool)` | `durable_store_conformance_postgres` **fails** on "an uncommitted append must not be visible to a reader"; the in-memory conformance callers stay green |

**M1a/M1b/M1c are three rows, not one, and that is the finding.** IS-3's "exactly one winner"
rests on the **conjunction** of two predicates that are each independently sufficient here:
neutralise the lease check and the compare-and-swap still admits one winner; neutralise the
CAS and the lease check still does. Only neutralising both breaks it. `reservation.rs:300-309`
already records the CAS as redundant given the lease predicate and says the whole suite stays
green without it — M1b re-measures that claim rather than inheriting it.

**This refutes SC-8 as written.** SC-8 requires that with the mechanism neutralised "the new
test fails, **and the pre-existing suite stays green**". For M1c that second clause is false:
the only mutation that breaks IS-3 also breaks `fencing_window_postgres`, because both tests
load the same predicate. The demonstrable statement is *"the new test fails, and no test that
does not name that predicate fails"* — blast radius, not global greenness. Flagged for spec
cross-review; M2 and M3 satisfy SC-8 exactly as written.

## Data Flow

```
run-suite ──┬─→ ledger guard (hermetic, no container)          fails in ms on drift
            ├─→ PG16 container ─→ ego_template (migrated once)
            │                        │
            │                        └─→ ego_test_{n} (clone, per test)
            │                              ├─ IS-1/IS-6  conformance harnesses (shared defs)
            │                              ├─ IS-2/IS-5  LOCK TABLE + wait_until_blocked(4)
            │                              ├─ IS-3       FOR UPDATE  + wait_until_blocked(6)
            │                              └─ IS-4       backfill, digest before/after
            ├─→ PG14 container ─→ pg14_test_{n} (migrations run HERE, not cloned)
            │                              └─ IS-9       T0 guard, T1, T2, T3
            └─→ rm() both, inside a live runtime, on every path
```

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `integration-tests/tests/infrastructure/durable_store_conformance_postgres.rs` | Create | IS-1 + IS-6 |
| `integration-tests/tests/infrastructure/events_identity_race_postgres.rs` | Create | IS-2 + IS-5 |
| `integration-tests/tests/infrastructure/lease_contention_postgres.rs` | Create | IS-3 |
| `integration-tests/tests/infrastructure/aggregate_type_backfill_postgres.rs` | Create | IS-4, cases C1–C4 |
| `integration-tests/tests/infrastructure/pg14_compatibility.rs` | Create | IS-9, T0–T3 |
| `integration-tests/tests/infrastructure.rs` | Modify | Five `mod` registrations; the ledger guard fails without them |
| `integration-tests/README.md` | Modify | Five ledger rows, two new categories, updated counts, five mutation rows |
| `integration-tests/src/lib.rs` | Modify | `wait_until_blocked`, `pg14_database` |
| `integration-tests/src/main.rs` | Modify | Second container: start, publish, reclaim on every path, report timings |
| `integration-tests/tests/infrastructure/fencing_window_postgres.rs` | Modify | Its private poll becomes a call to `wait_until_blocked`; doc comment and every assertion unchanged |
| `crates/testkit/src/{event_store.rs,reservation_conformance.rs}` | Unchanged | Reused verbatim (D-5) |
| `crates/persistence/**`, `migrations/**` | Unchanged | Exercised, not modified (D-8) |
| Root `Cargo.toml`, `cargo test --workspace` | Untouched | Root stays Docker-free |

Two of these rows are **not** in the proposal's Affected Areas table: `src/lib.rs` and
`src/main.rs`. IS-9 and the shared poll cannot be delivered without them. Flagged for spec
cross-review; both are inside `integration-tests/`, so the rollback plan is unaffected.

## Interfaces / Contracts

```rust
// integration-tests/src/lib.rs — new, the only two additions
pub async fn wait_until_blocked(observer: &PgPool, statement_like: &str, expected: usize);
pub async fn pg14_database() -> IsolatedDatabase;   // migrated in place, not cloned
```

No production interface changes. No `ego-testkit` change (D-5 holds).

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit | Nothing new | Every in-process property in scope is already covered; D-3 removed the one boundary that looked open |
| Integration (PG16) | IS-1..IS-6 | Five test functions across four files, on the existing shared container and per-test database |
| Integration (PG14) | IS-9 | One file, one container, four assertions, migrations applied in place |
| Ledger | Registration/row/file agreement | `tests/ledger.rs`, hermetic, run before provisioning |
| Adversarial | IS-8 | Five recorded mutations, AD-7's procedure, SHA-256 restore proof |

Budget forecast: IS-1 ≈ 21 reservation resets plus ~40 event-store round trips on localhost;
IS-2/IS-3 are dominated by their poll deadlines, which are not reached on a passing run;
IS-9 adds one container start plus one migration run. Estimated added wall clock ~4–6s
against a ~16s baseline, well inside the ≤5 min budget. Compile and execution stay reported
separately, as the runner already does.

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file classification, or
process-integration boundary is added. The runner's existing `Command::new(env!("CARGO"))`
spawns are unchanged, and no new argument is derived from test data. The one new resource
boundary is the second container, which is covered by an explicit design requirement:
**both containers are reclaimed on every exit path, awaited inside a live runtime, before the
suite's exit code is returned** — the exact failure the runner was written to close, and a
regression here would leak silently behind a green suite.

## Migration / Rollout

No migration. Test-only and additive. Reverting is deleting the five files, their five module
registrations and their five ledger rows, and reverting the four modified files; the ledger
guard verifies the three sources agree in both directions, so a partial revert fails loudly.
Delivery slices naturally for the 400-line review budget: IS-1 alone closes #275 AC10 and is
a complete first PR.

## Open Questions

- [ ] **Q1 — IS-2 scope.** SC-2's first clause is already met by IS-1 (AD-3). Confirm with
      `spec.md` that IS-2's requirement text is the post-`23505` re-read branch, not the
      pre-check, before the first RED.
- [ ] **Q2 — IS-4 completeness.** C2 (RolledBack) is required for IS-4 to be about
      transactions at all (AD-5), and the proposal's wording does not name it. Confirm the
      spec admits it.
- [ ] **Q3 — SC-8 wording.** Global greenness under M1c is not achievable (AD-7). Confirm the
      spec states blast radius rather than global greenness.
- [ ] **Q4 — Affected Areas.** `integration-tests/src/{lib,main}.rs` need rows in the
      proposal's table.
