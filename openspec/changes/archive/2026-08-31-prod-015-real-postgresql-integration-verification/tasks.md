# Tasks: PROD-015 — Real PostgreSQL Integration Verification

> Canonical / source of truth. Spanish review companion: `tasks.es.md` (1:1 task IDs, ordering,
> and evidence). Reads against `proposal.md`, `spec.md` and `design.md`, all final and
> cross-reviewed; no decision recorded there is re-litigated here.

## How to read this file

Every task states: what it touches, what it depends on, whether it can run in parallel with
other tasks, which spec requirement (`IS-#` / `SC-#`) or design decision (`AD-#`) it satisfies,
concrete steps under TDD (RED → GREEN, per `skills/testing-tdd/SKILL.md`), and **verifiable
evidence** — a command to run, an assertion that must hold, or a value to capture. "Test passes"
alone is never sufficient evidence.

Every new test file also carries, in its own doc comment, the invariant it proves and why
in-process cannot show it (`IS-7`, admission rule 4) — stated once per file below and expected
to land verbatim as the Rust doc comment, not paraphrased differently in code.

All new/modified files stay inside `integration-tests/` or the two files this change touches
directly (`crates/persistence/src/postgres/reservation.rs`,
`crates/persistence/src/postgres/aggregate_type_backfill.rs`,
`crates/persistence/src/postgres/event_store.rs`) — and only as **temporary, reverted-by-design**
mutations for IS-8 (Group 2b/4b/1, mutation tasks). No production file is left modified by this
change (D-8, Rollback Plan).

## Priority legend

- **P0** — many-contender fencing (IS-3) and the N-way append race (IS-2, post-`23505` re-read
  scope only), plus IS-3's mutation proof. Required by this phase's gate 5.
- **P1** — everything else in scope: IS-1 (foundational, cheapest, closes AC10, retires IS-6),
  IS-4, IS-5, IS-9, and the IS-8 mutation proofs for IS-4/IS-6 (M2, M3).
- IS-1 is listed under Group 1 ahead of the P0 groups because it is the dependency-free first
  slice (`design.md` "Approach": "IS-1 is deliberately first and cheapest") and a complete PR on
  its own; priority label and landing order are independent.

---

## Group 0 — Shared infrastructure (sequential prerequisite for IS-2, IS-3, IS-9)

### T-00.1 — Promote `wait_until_blocked` into `src/lib.rs`

- **Files:** `integration-tests/src/lib.rs` (new function), `integration-tests/tests/infrastructure/fencing_window_postgres.rs` (call-site only)
- **Depends on:** nothing
- **Blocks:** T-02.* (IS-3), T-03.* (IS-2/IS-5)
- **Parallel with:** T-00.2, T-01.*
- **Satisfies:** `design.md` AD-3 (prerequisite for IS-2/IS-3; not a spec `IS-#` on its own)
- **Steps:**
  1. RED: add `pub async fn wait_until_blocked(observer: &PgPool, statement_like: &str, expected: usize)` to `src/lib.rs`, extracted from `fencing_window_postgres.rs`'s private `wait_until_contender_is_blocked`, with the two load-bearing corrections AD-3 names: (a) add `AND datname = current_database()` to the `pg_stat_activity` predicate — cluster-wide visibility across up to 8 isolated databases makes this omission a false-pass risk once IS-3 adds a second blocking test; (b) narrow the statement match from a bare table-name fragment to a statement fragment (e.g. `'%UPDATE operation_reservations%'`), so a counted backend has provably already passed its pre-lock statements. It polls with an explicit deadline and fails the test at the deadline — the sleep inside is a poll interval, never a timeout standing in for a condition. `fencing_window_postgres.rs` does not yet call it (RED: compiles, but its own private helper is now dead code / duplicated — a deliberately temporary state).
  2. GREEN: replace `fencing_window_postgres.rs`'s private poll call site with `ego_integration_tests::wait_until_blocked(...)`; delete the now-dead private copy. Doc comment and every assertion in that file stay unchanged (design.md File Changes table).
- **Evidence:** `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` — `fencing_window_postgres` still passes with byte-identical assertions (`owner_id`, `StaleOwner`, fencing-token checks); `git diff integration-tests/tests/infrastructure/fencing_window_postgres.rs` shows only the poll call-site line changed, doc comment untouched.
- **Status:** [x] Done — `wait_until_blocked` lands in `src/lib.rs`; `fencing_window_postgres.rs` calls it, private copy deleted. Real-container run: 43 passed, 1 pre-existing ignored, 0 failed; `fencing_window_postgres`'s assertions unchanged.

### T-00.2 — PG14 second-container runner extension

- **Files:** `integration-tests/src/main.rs` (modified), `integration-tests/src/lib.rs` (modified — add `pg14_database()`)
- **Depends on:** nothing
- **Blocks:** T-05.* (IS-9)
- **Parallel with:** T-00.1, T-01.*
- **Satisfies:** `design.md` AD-6 (prerequisite for IS-9)
- **Steps:**
  1. RED: add `pub async fn pg14_database() -> IsolatedDatabase` to `src/lib.rs` — creates a database on the PG14 container and runs `ego_persistence::postgres::migrations::run()` against it **directly, in place, no template, no clone** (the migration run is itself the invariant IS-9 proves, per AD-6). Compiles; nothing calls it yet.
  2. GREEN: extend `main.rs` to start a second `Postgres::default().with_tag("14")` container, publish `EGO_IT_PG14_HOST` / `EGO_IT_PG14_PORT` to the child test process, and reclaim **both** containers (`.rm().await`) on every exit path — success, test failure, and the pre-existing panic/unwind-safe teardown — inside the still-live Tokio runtime, before returning the suite's exit code. Extend the existing timing report line with the PG14 provisioning and reclamation instants.
- **Evidence:** run the suite once clean and confirm via the container runtime's own listing (e.g. `docker ps -a` / the active `colima`/Docker context) that zero containers remain after exit; the runner's printed timing line names two provisioning instants (PG16, PG14) and both reclamation instants.
- **Status:** [x] Done — `pg14_database()` added to `src/lib.rs`; `main.rs` starts a second `Postgres::default().with_tag("14")` container and reclaims both independently. Real run confirmed `docker ps -a` shows zero containers after exit; timing line: `provisioned in 8.11s (PG16) / 31.67s (PG14) · template migrated at 31.80s · suite finished at 36.14s · reclaimed at 36.38s`. Minor evidence-wording gap: the timing line reports one combined `reclaimed_at` instant rather than two separate reclamation timestamps, though both `.rm().await` calls are independent and independently checked.

---

## Group 1 — IS-1 (foundational, closes AC10) + IS-6 retirement (P1, lands first)

### T-01.1 — RED: `durable_store_conformance_postgres.rs`, event-store half

- **File:** new `integration-tests/tests/infrastructure/durable_store_conformance_postgres.rs`
- **Depends on:** nothing (independent of Group 0)
- **Parallel with:** T-00.*, T-02.*, T-03.*, T-04.*
- **Satisfies:** IS-1 (event store leg), IS-6 (retired into this same run per D-4/AD-4's confirmed verdict — no separate test or ledger row)
- **Steps:**
  1. RED: `db = isolated_database(); pool = connect(db.url(), 4)` (`max_connections >= 2` is load-bearing per AD-4 — with a pool of 1, `load()` would starve waiting on the open unit of work's held connection and fail as a pool timeout, not an isolation failure; pinned at 4). `let mut store = PostgreSQLEventStore::open(pool, deserialize).await?;` then `assert_event_store_conformance(&mut store, |kind| ConformanceEvent { .. }).await;` with a local `ConformanceEvent` fixture (not a re-derived contract — `crates/infrastructure`'s copy is private to another crate's test target, per AD-4). File not yet registered as a module: `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` fails, correctly, naming the unregistered file.
  2. Doc comment (verbatim, per admission rule 4): states the invariant — `PostgreSQLEventStore` satisfies the identical `assert_event_store_conformance` definitions the in-memory adapters satisfy, including that a staged, uncommitted append on `PostgreSQLEventStore`'s held connection is invisible to `store.load()` issued from a distinct pooled connection, and that a dropped-without-commit unit of work persists nothing (IS-6, demonstrated here per D-4, no separate test) — and why in-process cannot show it: no in-memory double has a real transaction, a real second pooled connection, or real `READ COMMITTED` cross-connection visibility.
- **Evidence:** deferred to T-01.3 (module not yet registered — ledger intentionally red here).
- **Status:** [x] Done — file created with the event-store test and verbatim doc comment (adapted for both halves, T-01.2 landed in the same file). RED confirmed: `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` failed, naming `durable_store_conformance_postgres` as unregistered and undocumented, before T-01.3.

### T-01.2 — GREEN: `durable_store_conformance_postgres.rs`, reservation-store half

- **File:** same file as T-01.1, second `#[tokio::test]`
- **Depends on:** T-01.1 (same file, single writer)
- **Satisfies:** IS-1 (reservation store leg)
- **Steps:**
  1. `let pool = &connect(db.url(), 4).await;` — the factory the harness requires must be `Copy`, forbidding a captured owned `PgPool`; a shared reference is `Copy` (AD-4 verified this against the harness signature, which is **async**, not the sync shape D-5 stated). `assert_reservation_store_conformance(|| async move { sqlx::query("TRUNCATE operation_reservations").execute(pool).await.unwrap(); let clock = Arc::new(TestClock::new(epoch())); (PostgresOperationReservationStore::new(pool.clone(), clock.clone()), clock) }).await;` `TRUNCATE` is the reset the harness's 21 `fresh()` calls need — the store owns exactly one table, so truncating it is sufficient and cheap (AD-4; a fresh isolated database per call was rejected there as 21 serialized `CREATE DATABASE`s for no added isolation).
  2. Doc comment addendum for this second function: invariant = durable fencing/lease conformance under real conditional `UPDATE`s; why-not-in-process = the harness's fencing/CAS assertions need a real row and a real conditional comparison, which a scripted store cannot misrepresent in the way this suite is built to catch.
- **Evidence:** deferred to T-01.3.
- **Status:** [x] Done — second `#[tokio::test]` added to the same file, `pool: &PgPool` closure (Copy, per AD-4), `TRUNCATE operation_reservations` as the reset.

### T-01.3 — GREEN: register module + README ledger row for IS-1/IS-6

- **Files:** `integration-tests/tests/infrastructure.rs` (add `mod durable_store_conformance_postgres;`), `integration-tests/README.md` (new "Durable-adapter conformance" category row, IS-1 citing both `#[tokio::test]` functions; a prose sentence next to it stating IS-6 is demonstrated by this same run per D-4, with no separate row)
- **Depends on:** T-01.1, T-01.2
- **Steps:** register the module; add the ledger row exactly as a table row with the path as a code span (the guard only counts rows, per `integration-tests/README.md`'s own documented parsing rule); update the `Total infrastructure tests` count.
- **Evidence:** `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` passes with zero drift; `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` shows both `postgres_event_store_conformance` and `postgres_reservation_store_conformance` passing, with an assertion count no smaller than the existing in-memory callers' (`crates/infrastructure/tests/in_memory_event_store_conformance.rs`, `crates/testkit/src/reservation.rs`) — same definitions, not a weakened set.
- **D-8 note:** any conformance failure surfacing here is presumptively located in `PostgresEventStoreUnitOfWork::confirm_receipt`'s conflicting-fingerprint branch (AD-4's own "D-8 note" — never run against real PostgreSQL before). **No fix is pre-authorized.** A discovered defect here becomes a named follow-up spec (D-8's small-fix exception explicitly covers only IS-2 and IS-4, never IS-1/IS-6).
- **Status:** [x] Done — module registered in `tests/infrastructure.rs`, new "Durable-adapter conformance" section + row added to `README.md` with the IS-6 prose note, `Total infrastructure tests` updated 16 → 17. `cargo test --test ledger`: 9/9 passed. Real-container run: 45 passed (43 pre-existing + 2 new), 1 pre-existing ignored, 0 failed — both `postgres_event_store_conformance` and `postgres_reservation_store_conformance` passed on the first real-container run; no conformance failure surfaced, so the D-8 note's presumptive location was never exercised and no follow-up spec is needed for T-01.3.

### T-01.4 — IS-8 mutation proof for IS-1/IS-6 (M3), P1

- **Files (temporarily mutated, then reverted):** `crates/persistence/src/postgres/event_store.rs`; **Ledger addition:** `integration-tests/README.md`'s existing mutation table
- **Depends on:** T-01.3 (test must exist and be green before it can be shown to fail)
- **Satisfies:** IS-8 ("neutralizing transactional/unit-of-work atomicity fails the corresponding test" scenario, spec.md), SC-8
- **Steps (AD-7's exact 7-step recipe):**
  1. `shasum -a 256 crates/persistence/src/postgres/event_store.rs` — record BEFORE.
  2. In `PostgresEventStoreUnitOfWork::append`, change `.execute(&mut *self.tx)` → `.execute(&self.pool)` (routes the write off the held transaction).
  3. `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
  4. Record: expected — `durable_store_conformance_postgres` fails on "an uncommitted append must not be visible to a reader"; the in-memory conformance callers (unaffected production code) stay green. Record the exact failing assertion message.
  5. `git checkout -- crates/persistence/src/postgres/event_store.rs`.
  6. `shasum -a 256` — MUST equal the value from step 1.
  7. Re-run step 3 as the negative control: all green.
- **Evidence:** one new row (`M3`) in `integration-tests/README.md`'s mutation table with the BEFORE/AFTER-restore SHA-256 pair (equal) and the exact failing test name + assertion message from step 4.
- **Status:** [x] Done — new "IS-8 mutation proofs" table added to `README.md` (none existed; the pre-existing "mutation table" proves `tests/ledger.rs` itself, a different obligation, so a new section was added rather than repurposing it). SHA-256 before = `9dc88a462…` = after-restore (confirmed equal). Mutated run: `durable_store_conformance_postgres::postgres_event_store_conformance` failed exactly as predicted ("an uncommitted append must not be visible to a reader, saw 1 event(s)"), every other test including `postgres_reservation_store_conformance` stayed green. Negative-control re-run: 45 passed, 1 pre-existing ignored, 0 failed.

---

## Group 2 — IS-3, P0: many-contender fencing race + its mutation proof

### T-02.1 — RED: `lease_contention_postgres.rs`, six-contender race

- **File:** new `integration-tests/tests/infrastructure/lease_contention_postgres.rs`
- **Depends on:** T-00.1 (`wait_until_blocked`)
- **Parallel with:** T-01.*, T-03.*, T-04.*
- **Satisfies:** IS-3, SC-3
- **Steps:**
  1. RED: orchestrate per AD-3's exact shape — a `holder` transaction runs `SELECT owner_id … WHERE operation_key = $1 FOR UPDATE` on the expired-lease row and holds the row lock; spawn six contender tasks each calling `store.reserve(owner_b_i, …)` against a store pool with `max_connections >= 6` (design pins 8); each contender's `INSERT … ON CONFLICT DO NOTHING` (0 rows, does not wait) and plain `SELECT` (MVCC, sees the expired lease) complete, then its `UPDATE … WHERE fencing_token = T AND lease_until <= now` blocks on the holder's row lock; the test calls `wait_until_blocked(observer, "%UPDATE operation_reservations%", 6)` with an explicit deadline before releasing the holder — without this poll the test would be probabilistic, since six contenders could resolve serially with the assertions still passing while proving nothing (AD-3's own stated rationale). `holder.commit()`. Await all six contender results.
  2. Doc comment (verbatim): invariant — six real contenders racing one expired lease leave exactly one `TakenOver` winner and the fencing token advances by exactly one, never by the contender count; why-not-in-process — forcing six real `UPDATE` statements to genuinely block on one real row lock, with a deterministic poll proving all six read the expired lease before any wrote, is not expressible without a real PostgreSQL row lock.
  3. GREEN: register the module; add the README row under "PostgreSQL concurrency invariants".
- **Evidence:** exactly one `TakenOver`; exactly five `OtherInProgress`; `fencing_token == T + 1` (never `T + 6`); the winning `owner_id` matches the single `TakenOver` result. `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` — slice time ≤1–2 min (SC-9).
- **Status:** [x] Done — ledger guard confirmed RED first (`every_test_on_disk_is_registered_as_a_module` and `..._is_accounted_for_in_the_ledger` both failed, naming `lease_contention_postgres`); module registered in `tests/infrastructure.rs`, row added to README's "PostgreSQL concurrency invariants" table, Budget block updated (2 → 3 concurrency invariants, 17 → 18 total). Real-container run: `lease_contention_postgres::six_contenders_racing_one_expired_lease_leave_exactly_one_winner` passed — exactly one `TakenOver`, exactly five `OtherInProgress`, `fencing_token == a_token + 1`, winning owner/token match the row read back directly. Full suite: 46 passed (43 pre-existing + 2 from Group 1 + 1 new from this task), 1 pre-existing ignored, 0 failed; suite finished at 13.62s — well inside the ≤1–2 min slice budget (SC-9).

### T-02.2 — IS-8 mutation proof for IS-3 (M1a / M1b / M1c), P0

- **Files (temporarily mutated, then reverted):** `crates/persistence/src/postgres/reservation.rs`; **Ledger addition:** `integration-tests/README.md`'s mutation table (three rows)
- **Depends on:** T-02.1 (test must exist and be green), T-00.1 (`fencing_window_postgres` already using the shared helper)
- **Satisfies:** IS-8 ("neutralizing the fencing mechanism fails the fencing test" scenario, spec.md), SC-8 — **as corrected by AD-7**, blast-radius not global greenness for M1c
- **Steps — three separate applications of AD-7's 7-step recipe, one per row, never combined into a single edit:**
  1. **M1a** — neutralize the lease-expiry predicate. Record BEFORE hash. Edit the takeover `UPDATE`'s `.bind(now)` → `.bind(now - chrono::Duration::days(3650))`, which neutralizes `lease_until <= $7`. Run the suite. Record: expected — `fencing_window_postgres` **fails**; `lease_contention_postgres` stays green (the CAS predicate alone is still sufficient). Restore; verify hash equality; negative-control re-run, all green.
  2. **M1b** — neutralize the fencing-token CAS. Record BEFORE hash. Edit `AND fencing_token = $6` → `AND $6 = $6`. Run the suite. Record: expected — the **whole suite stays green** (the lease-expiry predicate alone is still sufficient; `reservation.rs:300-309`'s own comment already claims this, and this row re-measures rather than inherits that claim). Restore; verify; negative-control re-run.
  3. **M1c** — neutralize both predicates together. Record BEFORE hash. Apply both M1a and M1b's edits simultaneously. Run the suite. Record: expected — `lease_contention_postgres` **fails**, observing six `TakenOver` results where exactly one is required; `fencing_window_postgres` **also fails**, because both tests load the identical predicate. This is the case where SC-8's literal "and the pre-existing suite stays green" clause does not hold; the demonstrable, narrower claim — recorded here verbatim — is: *the new test fails, and no test that does not exercise this predicate fails*. Restore; verify hash equality; negative-control re-run, all green.
- **Evidence:** three README mutation rows (`M1a`, `M1b`, `M1c`) each with its BEFORE/AFTER-restore SHA-256 pair (equal) and the exact failing test name(s) + assertion message(s) from its run. M1c's row explicitly states the blast-radius framing above, not "suite stays green."
- **Status:** [x] Done, with one corrected edit noted below — three rows added to README's new IS-8 mutation-proofs table (T-01.4's table), each following AD-7's full 7-step recipe once, applied separately. BEFORE hash for all three: `e69b9bf6cefb6a51fd60ad1af9e75c1fd5fe05a1466f7d4be41425a8221118f3` (== after-restore, confirmed equal each time). **M1a — sign corrected from this task's literal text**: `.bind(now)` → `.bind(now - chrono::Duration::days(3650))` (a past date) was tried first and empirically breaks the predicate the *opposite* way — it makes `lease_until <= $7` almost always **false**, refusing every legitimate takeover, and failed 5 tests suite-wide (`durable_store_conformance_postgres::postgres_reservation_store_conformance`, `dual_aggregate_crash_recovery_postgres`, `takeover_fencing_postgres`, `fencing_window_postgres`, `lease_contention_postgres`), contradicting this task's own predicted blast radius. Reverted, hash re-confirmed equal, then re-applied as `.bind(now + chrono::Duration::days(3650))` (a future date), which makes the predicate almost always **true** — the correct "neutralize" direction. That version matched the prediction exactly: only `fencing_window_postgres` failed ("Got TakenOver(…)" where a refusal was required), 45 passed/1 failed/1 ignored, `lease_contention_postgres` stayed green. Restored; hash equal; negative-control re-run all green. **M1b**: `AND fencing_token = $6` → `AND $6 = $6`; whole suite stayed green as predicted, 46 passed/0 failed/1 ignored, re-measuring `reservation.rs:300-309`'s own comment. Restored; hash equal; negative-control re-run all green. **M1c**: both M1a (corrected, `+`) and M1b applied together; both `fencing_window_postgres` and `lease_contention_postgres` failed as predicted — the latter observing all six contenders report `TakenOver` (`left: 6, right: 1`) — 44 passed/2 failed/1 ignored, matching the blast-radius framing verbatim (not global greenness, per SC-8/AD-7). Restored; hash equal; negative-control re-run all green (46/0/1). `git diff --stat` on `reservation.rs` is empty — no production file left modified.

---

## Group 3 — IS-2 (P0, post-`23505` scope only) + IS-5 (P1, same file)

### T-03.1 — RED: `events_identity_race_postgres.rs`, IS-2 N-way append race

- **File:** new `integration-tests/tests/infrastructure/events_identity_race_postgres.rs`
- **Depends on:** T-00.1 (`wait_until_blocked`)
- **Parallel with:** T-01.*, T-02.*, T-04.*
- **Satisfies:** IS-2 — **scoped exactly to the post-`23505` re-read branch** (spec.md MODIFIED requirement "Effective Uniqueness on the Event Stream Identity", third scenario), never the single-caller stale-expected-version pre-check, which IS-1's conformance run already exercises (`crates/testkit/src/event_store.rs:124-149`) — re-asserting it here would violate admission rule 3 (no duplication). SC-2's second clause.
- **Steps:**
  1. RED: `holder: BEGIN; LOCK TABLE events IN EXCLUSIVE MODE` (blocks `INSERT`, allows plain `SELECT`); spawn four racer tasks each calling `store.append(type, id, tenant, 0, [event])` against a store pool with `max_connections >= 4` (design pins 6); each racer's `SELECT COALESCE(MAX(version),0)` (ACCESS SHARE, not blocked, sees 0) completes, then its `INSERT INTO events …` (ROW EXCLUSIVE) blocks on the holder's table lock; the test calls `wait_until_blocked(observer, "%INSERT INTO events%", 4)` before releasing the holder; `holder.commit()`. After release, exactly one racer commits and the remaining three take `23505` on `ux_events_identity_tenant`, drop their aborted transaction, re-read the stream **on a different pooled connection**, and report `Conflict { expected: 0, actual: 1 }` (traceable at `event_store.rs:198-247`).
  2. Doc comment (verbatim): invariant — an N-way concurrent append race on one stream leaves exactly one winner, and each of the N-1 losers' conflicts reports the real, winning current version, obtained only after the store's own transaction has already aborted on the unique-constraint violation and must re-read the stream on a different connection; why-not-in-process — this requires forcing real concurrent transactions to genuinely collide on a real unique constraint, past the point of a real transaction abort — a scripted store has no unique-constraint abort at all, and the pre-check branch (already covered by IS-1) needs no race. Explicitly states, in the same comment, that the pre-check clause of SC-2 is out of this test's scope by design, not by omission.
  3. GREEN: register the module; add the README row.
- **Evidence:** exactly 1 racer succeeds; exactly 3 racers report `Conflict { expected: 0, actual: 1 }`, with `actual` sourced from the post-abort re-read path. `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` — slice ≤1–2 min.
- **D-8 note:** if this test exposes that the post-`23505` re-read reports a stale or wrong `actual` (a real defect in that branch), a small, localized fix is permitted **only** if strictly necessary to satisfy this exact invariant, and **must not** introduce a new API, new contractual behavior, an architectural change, or an additional migration. Otherwise: a named follow-up spec (D-8, OOS-7). No fix is pre-authorized by this task.
- **Status:** [x] Done — file created with the table-lock racer fixture (`RaceEvent`, `race()`, `wait_until_blocked(test_pool, "%INSERT INTO events%", 4)`), module registered in `tests/infrastructure.rs` with the IS-2/IS-5 scope doc comment, README row added to "The PostgreSQL concurrency invariants". No D-8 defect surfaced: the post-`23505` re-read reported the correct winning version (`actual: 1`) on the first real-container run, so no fix and no follow-up spec are needed. Real-container run: `events_identity_race_postgres::an_n_way_append_race_leaves_one_winner_and_reports_the_real_version_after_abort` passed — exactly 1 of 4 racers won, exactly 3 reported `Conflict { expected: 0, actual: 1 }`.

### T-03.2 — RED: IS-5, NULL-tenant three-valued-logic behavioral race (P1)

- **File:** same file as T-03.1, additional `#[tokio::test]`
- **Depends on:** T-03.1 (same file, single writer)
- **Satisfies:** IS-5, SC-5, spec.md "NULL-Tenant Stream Identity Honors SQL's Three-Valued Comparison Behaviorally"
- **Steps:**
  1. RED: run the identical race shape as T-03.1 with `tenant = None`, which loads `ux_events_identity_systemwide` (the partial index that exists only because `NULLS NOT DISTINCT` is PostgreSQL 15+, per AD-3). Add: one direct-SQL duplicate insert under a NULL tenant, expected to fail with `23505` (proving NULL-tenant identity is NOT exempt from uniqueness — spec.md's second MODIFIED scenario); one insert of the identical `(aggregate_type, aggregate_id, version)` under a concrete tenant, expected to **succeed** (proving the systemwide and tenant-scoped partial indexes do not collide with each other); and two events under two distinct `aggregate_id`s, both `tenant = None`, each asserted to resolve to its own independent stream with no false collision or false merge (spec.md's ADDED scenario, "Two distinct systemwide-tenant streams resolve independently").
  2. Doc comment (verbatim): invariant — `Option::None` tenant identity is verified behaviorally under SQL's three-valued comparison (`NULL = NULL` is not true), not only from the catalog; two systemwide streams never silently collide or merge, and NULL-tenant uniqueness is genuinely enforced; why-not-in-process — `schema_index_assertion.rs` already pins the catalog shape, but only a real insert against real three-valued NULL comparison proves the behavior it implies.
- **Evidence:** `23505` observed for the NULL-tenant duplicate; the concrete-tenant insert with the identical `(aggregate_type, aggregate_id, version)` succeeds; the two distinct NULL-tenant streams each list independently via `list_aggregate_ids` / `load()` with no cross-contamination.
- **Status:** [x] Done — second `#[tokio::test]` added to the same file (single writer, same README row as T-03.1, covering both guarantees). Real-container run: `events_identity_race_postgres::null_tenant_identity_is_genuinely_unique_not_exempt_and_does_not_collide_with_a_concrete_tenant` passed — exactly 1 of 4 racers won the systemwide race; the direct-SQL NULL-tenant duplicate failed with `23505` on `ux_events_identity_systemwide`; the identical `(aggregate_type, aggregate_id, version)` under a concrete tenant succeeded (no cross-index collision); alpha/beta's two distinct systemwide streams each loaded back exactly their own event with no contamination. `cargo test --test ledger`: 9/9 passed (registration + anchoring both satisfied). Full suite: 48 passed (46 pre-existing + 2 new), 1 pre-existing ignored, 0 failed; suite finished at 13.22s — well inside the ≤1–2 min slice budget (SC-9). Containers reclaimed (`docker ps -a` empty for postgres).

---

## Group 4 — IS-4, P1 (full weight per D-9): migration 007 backfill transactional behavior

### T-04.1 — RED: `aggregate_type_backfill_postgres.rs`, C1 Aborted

- **File:** new `integration-tests/tests/infrastructure/aggregate_type_backfill_postgres.rs`
- **Depends on:** nothing
- **Parallel with:** T-01.*, T-02.*, T-03.*
- **Satisfies:** IS-4 (case C1), SC-4
- **Steps:** seed one row whose `aggregate_id` matches no registered type; take the pre-digest (`SELECT md5(string_agg(events::text, '|' ORDER BY id)) FROM events` — `events::text` renders every column as a composite literal, so a column added later is covered without editing the test, per AD-5); run the backfill; assert `Aborted(NoRegisteredTypeMatches)` and `rows_rewritten: 0`; take the post-digest; assert digest equality. **Note, per AD-5's own rationale:** this case alone proves statement *ordering* (`drop(tx)` runs before any `UPDATE`, `aggregate_type_backfill.rs:269-288`), not transactional rollback — C2 below is what actually proves the transaction.
- **Doc comment (verbatim):** invariant — an abort before the backfill's first `UPDATE` leaves the table byte-identical; why-not-in-process — requires a real migrated table and the real abort closure's statement ordering.
- **Evidence:** digest-before == digest-after; `Aborted(NoRegisteredTypeMatches)` returned.
- **Status:** [x] Done — file created; `c1_an_abort_before_any_write_leaves_the_table_byte_identical` seeds one `"orphan-123"` row (no registered type is a prefix), asserts `Aborted(NoRegisteredTypeMatches)`, `rows_rewritten: 0`, and digest equality via `SELECT md5(string_agg(events::text, '|' ORDER BY id))` (AD-5). Real-container run: passed.

### T-04.2 — RED: C2 RolledBack

- **File:** same file
- **Depends on:** T-04.1 (same file, single writer)
- **Satisfies:** IS-4 (case C2) — required for IS-4 to demonstrate a genuine transaction, per AD-5's rationale
- **Steps:** seed a stream with versions 1 and 3 (a hole, splitting cleanly); run the backfill; assert `RolledBack(StreamVersionsAreNotConsecutiveFromOne)`; take the post-digest and assert equality with pre; assert `aggregate_type` is **still nullable** via `information_schema.columns` (not yet `SET NOT NULL`) — this is the only case where rows are genuinely written and then discarded, the property only a real transaction (not statement ordering) can have.
- **Doc comment (verbatim):** invariant — an explicit rollback after at least one completed `UPDATE` leaves the table byte-identical, proving the rollback — not merely statement ordering — is what guarantees no partial effect; why-not-in-process — only a real transaction rollback against a real migrated table can demonstrate discarded writes.
- **Evidence:** digest-before == digest-after; `information_schema.columns.is_nullable = 'YES'` for `aggregate_type` post-run.
- **D-8 note:** this is the case most likely to expose a real defect (AD-5 names it as the only genuine transactional-rollback proof PROD-015 adds). If the rollback does not hold, a small, localized fix is permitted **only** if strictly necessary to satisfy this exact IS-4 invariant, and must not introduce a new API, new contractual behavior, an architectural change, or an additional migration. Otherwise: a named follow-up spec.
- **Status:** [x] Done — `c2_a_rollback_after_a_completed_write_leaves_the_table_byte_identical` seeds versions 1 and 3 of the same stream (a hole), asserts `RolledBack(StreamVersionsAreNotConsecutiveFromOne)`, digest equality, and `aggregate_type` still `is_nullable = 'YES'` post-run. No defect surfaced: the rollback held on the first real-container run, so no D-8 fix and no follow-up spec are needed. Real-container run: passed.

### T-04.3 — RED: C3 Zero-row commit

- **File:** same file
- **Depends on:** T-04.2
- **Satisfies:** IS-4 (case C3), SC-4
- **Steps:** empty `events`; run the backfill; assert `Committed`, `rows_scanned: 0`; assert `information_schema.columns.is_nullable = 'NO'` for `aggregate_type` post-run — proving the run committed `SET NOT NULL` (the last statement before commit), not merely "committed nothing" (AD-5's own point: without this assertion, a run that commits nothing at all would still satisfy a looser reading of "commits cleanly").
- **Doc comment (verbatim):** invariant — a run over zero eligible rows commits without side effects, including the schema-level `SET NOT NULL`; why-not-in-process — requires a real migrated table and a real catalog read to distinguish "committed the intended statement" from "committed nothing."
- **Evidence:** `Committed`, `rows_scanned: 0`; `is_nullable = 'NO'` post-run.
- **Status:** [x] Done — `c3_a_zero_row_commit_still_commits_the_schema_level_not_null` runs the backfill against an empty table, asserts `Committed`, `rows_scanned: 0`, and `is_nullable = 'NO'` post-run. Real-container run: passed.

### T-04.4 — RED: C4 Revert round-trip

- **File:** same file
- **Depends on:** T-04.3
- **Satisfies:** IS-4 (case C4), SC-4
- **Steps:** seed eligible rows → take pre-digest → run backfill (assert `Committed`) → call `revert_aggregate_type_column` → take post-revert digest; assert post-revert digest == pre-digest; assert `aggregate_type` column no longer exists via `information_schema.columns`.
- **Doc comment (verbatim):** invariant — a revert rejoins exactly the state that preceded the backfill; why-not-in-process — requires the real forward and reverse migration paths against a real, migrated database.
- **Evidence:** digest equality; column absence confirmed.
- **Status:** [x] Done, with one digest-formula correction from this task's literal text — `c4_a_revert_rejoins_exactly_the_state_that_preceded_the_backfill` seeds two eligible rows, asserts `Committed`, reverts, and compares digests. **Correction:** the plain `events::text` digest (AD-5's formula, reused verbatim for C1–C3) cannot express "unchanged" here — `revert_aggregate_type_column` drops the `aggregate_type` column entirely, so a composite built from every column has one fewer field after the revert than it did before the backfill ran, and would never compare equal regardless of content fidelity. Used instead: an explicit-column digest naming every column except `aggregate_type` (`digest_excluding_aggregate_type`), which is the only formula under which "rejoins exactly the state that preceded the backfill" is expressible — the column's *existence*, not its content, is what this case is about, and is checked separately via `information_schema.columns`. Real-container run: passed, confirming the digest equality and column absence both hold.

### T-04.5 — GREEN: register module + README ledger row

- **Files:** `integration-tests/tests/infrastructure.rs`, `integration-tests/README.md` (new "Migration transactional behaviour" category)
- **Depends on:** T-04.1 through T-04.4
- **Evidence:** `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` passes; `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` shows all four C1–C4 cases passing.
- **Status:** [x] Done — module registered in `tests/infrastructure.rs`; new "Migration transactional behaviour" section + four rows (C1–C4) added to `README.md`, all citing the one file; Budget block updated (new `Migration transactional behaviour ... 1` category, `Total infrastructure tests` 19 → 20). Ledger guard confirmed RED first (both assertions failed, naming `aggregate_type_backfill_postgres`), then GREEN: `cargo test --test ledger`: 9/9 passed. Real-container run: 52 passed (48 pre-existing + 4 new), 1 pre-existing ignored, 0 failed — all four C1–C4 cases passed on the first real-container run; suite finished at 16.48s, well inside the ≤1–2 min slice budget (SC-9). Containers reclaimed.

### T-04.6 — IS-8 mutation proof for IS-4 (M2), P1

- **Files (temporarily mutated, then reverted):** `crates/persistence/src/postgres/aggregate_type_backfill.rs`; **Ledger addition:** `integration-tests/README.md`'s mutation table
- **Depends on:** T-04.5
- **Satisfies:** IS-8 ("neutralizing transactional/unit-of-work atomicity fails the corresponding test" scenario, spec.md), SC-8 — **satisfied exactly as written** for this mutation (AD-7: M2 and M3 satisfy SC-8's literal "pre-existing suite stays green" clause; only M1c does not)
- **Steps (AD-7's 7-step recipe):**
  1. `shasum -a 256 crates/persistence/src/postgres/aggregate_type_backfill.rs` — BEFORE.
  2. In the `StreamVersionsAreNotConsecutiveFromOne` branch, change `tx.rollback().await?` → `tx.commit().await?`.
  3. Run the suite.
  4. Record: expected — `aggregate_type_backfill_postgres` C2 **fails** on the digest comparison; every other test stays green.
  5. `git checkout -- crates/persistence/src/postgres/aggregate_type_backfill.rs`.
  6. `shasum -a 256` — MUST equal BEFORE.
  7. Re-run as negative control: all green.
- **Evidence:** README mutation row `M2` with the SHA-256 pair and the exact failing assertion message.
- **Status:** [x] Done — SHA-256 BEFORE = `3381e5cf4852c9d99e6a30f227c6d0c625efc6ef84e92abdf6ee32bbc7085fed` (== after-restore, confirmed equal). Mutated run: `aggregate_type_backfill_postgres::c2_a_rollback_after_a_completed_write_leaves_the_table_byte_identical` failed exactly as predicted, on the digest comparison; every other test, including `c1`/`c3`/`c4` in the same file, stayed green — 51 passed, 1 failed, 1 ignored. SC-8's literal "pre-existing suite stays green" clause is satisfied exactly as written for this mutation (per AD-7, unlike M1c). Restored via `git checkout --`; hash re-confirmed equal; negative-control re-run: 52 passed, 0 failed, 1 ignored. `git diff --stat` on `aggregate_type_backfill.rs` is empty — no production file left modified. README `M2` row added.

---

## Group 5 — IS-9, P1: narrow PG14 compatibility slice

### T-05.1 — RED: `pg14_compatibility.rs`, T0 anti-vacuity guard

- **File:** new `integration-tests/tests/infrastructure/pg14_compatibility.rs`
- **Depends on:** T-00.2 (`pg14_database()`, second container)
- **Parallel with:** T-01.* through T-04.*
- **Satisfies:** IS-9 (assertion T0), SC-13
- **Steps:** `db = pg14_database().await;` query `current_setting('server_version_num')::int` and assert it is in `[140000, 150000)`. This is the same "three-empty-sets" style anti-vacuity control the ledger guard already has: without it, a container-tag typo would silently run the "PG14 slice" against PG16 and prove nothing.
- **Doc comment (verbatim, covers the whole file):** invariant — PG14 remains a verified, real compatibility floor for exactly the version-sensitive invariants named below (T1–T3), never a second full run of the main suite; why-not-in-process — a compatibility floor can only be demonstrated against the real target engine version.
- **Evidence:** assertion holds. (This guard is a correctness control on the slice's own container, not one of the three committed IS-8 mutations — a deliberate container-tag mutation is not part of this change's committed mutation set.)
- **Status:** [x] Done — new file `integration-tests/tests/infrastructure/pg14_compatibility.rs`; `t0_the_pg14_container_genuinely_reports_a_pg14_server_version` queries `current_setting('server_version_num')::int` against `pg14_database()` and asserts it falls in `[140000, 150000)`. Real-container run: passed.

### T-05.2 — RED: T1, full migration set 001–012 applies cleanly on PG14

- **File:** same file
- **Depends on:** T-05.1 (same file, single writer)
- **Satisfies:** IS-9 (assertion T1)
- **Steps:** confirm `pg14_database()`'s in-place `migrations::run()` (already executed by the fixture) completed without error; assert the migration-tracking table records all migrations 001 through 012 applied. Cite in the doc comment why 008, 011 and 012 are the version-sensitive ones: `NULLS NOT DISTINCT` is PostgreSQL 15+ and the floor is 14 (008's own comment records the 14-image syntax error); 012 additionally uses `DELETE … USING (SELECT … ROW_NUMBER() OVER …)`.
- **Evidence:** no migration error; tracking table shows 12 applied migrations.
- **Status:** [x] Done, with one correction from this task's literal text — `crates/persistence/src/postgres/migrations.rs`'s `run()` carries no migration-tracking table at all: it re-applies every registered migration's idempotent (`IF NOT EXISTS`) SQL on every call, with nothing recording which names already ran, so there is no tracking table to query. `t1_the_full_migration_set_applies_cleanly_and_leaves_every_version_sensitive_artifact_present` asserts the positive, verifiable equivalent instead: every version-sensitive schema artifact — the eight partial unique indexes from migrations 008/010/011/012, `events.aggregate_type` and `events.operation_key` from 007/009, and the `operation_reservations`/`operation_receipts` tables from 010/011 — genuinely exists on the PG14 target via `pg_indexes`/`information_schema`, after `pg14_database()`'s in-place (non-templated) `migrations::run()` already succeeded without panicking. Real-container run: passed.

### T-05.3 — RED: T2, duplicate under `tenant_id IS NULL` refused with `23505` on PG14

- **File:** same file
- **Depends on:** T-05.2
- **Satisfies:** IS-9 (assertion T2)
- **Steps:** direct-SQL insert of a duplicate `(aggregate_type, aggregate_id, version)` under `tenant_id IS NULL`; assert PostgreSQL error code `23505`.
- **Evidence:** `23505` observed.
- **Status:** [x] Done — `t2_a_systemwide_duplicate_identity_is_refused_with_23505_on_pg14` inserts a `tenant_id IS NULL` row with `aggregate_type = 'order'`, `aggregate_id = 'id-1'`, `version = 1`, then repeats the identical insert; asserts the second returns `sqlx::Error::Database` with `.code() == Some("23505")`, exercising `ux_events_identity_systemwide` directly. Real-container run: passed.

### T-05.4 — RED: T3, migration 007 + backfill commit + revert round trip on PG14

- **File:** same file
- **Depends on:** T-05.3
- **Satisfies:** IS-9 (assertion T3)
- **Steps:** mirror T-04.4's shape against the PG14 database: seed → run `backfill_aggregate_type` (assert `Committed`) → run `revert_aggregate_type_column` → assert round trip (digest or column-presence check).
- **Evidence:** `Committed`; revert round trip confirmed.
- **Status:** [x] Done — `t3_the_backfill_and_its_revert_round_trip_cleanly_on_pg14` seeds one `"user-7"` row against the PG14 database, takes the explicit-column pre-digest (T-04.4's corrected formula, reused here since C4's column-drop shape mismatch applies identically on PG14), runs `backfill_aggregate_type` (asserts `Committed`), calls `revert_aggregate_type_column`, asserts post-digest equality and `aggregate_type` column absence via `information_schema.columns`. Real-container run: passed.

### T-05.5 — GREEN: register module + README ledger row for IS-9

- **Files:** `integration-tests/tests/infrastructure.rs`, `integration-tests/README.md` (new "Version-floor compatibility" category)
- **Depends on:** T-05.1 through T-05.4
- **Steps:** register the module; add the ledger row; add an explicit prose note (per AD-6's own "Explicitly not on PG14" list) that IS-1, IS-2, IS-3, IS-6, and IS-4's C1/C2 abort/rollback cases, plus all sixteen pre-existing tests, are **not** run on PG14 — this file targets exactly T0–T3, nothing else.
- **Evidence:** `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` green; a `grep` of the file shows exactly four `#[tokio::test]` functions (T0–T3), none named after contention, fencing, or unit-of-work.
- **Status:** [x] Done — module registered in `tests/infrastructure.rs`; new "Version-floor compatibility" section + four rows (T0–T3) added to `README.md`, all citing the one file, with the explicit AD-6 "not run on PG14" prose note; Budget block updated (new `Version-floor compatibility ... 1` category, `Total infrastructure tests` 20 → 21). Ledger guard confirmed RED first (`every_test_on_disk_is_accounted_for_in_the_ledger` failed, naming `pg14_compatibility`), then GREEN: `cargo test --test ledger`: 9/9 passed. `rg -c '#\[tokio::test\]' pg14_compatibility.rs` = 4, named `t0_`/`t1_`/`t2_`/`t3_` — none referencing contention, fencing, or unit-of-work. Real-container run: 56 passed (52 pre-existing + 4 new), 1 pre-existing ignored, 0 failed — all four T0–T3 cases passed on the first real-container run; suite finished at 13.52s total (both containers), well inside the ≤1–2 min slice budget (SC-9). Containers reclaimed.

---

## Group 6 — README/ledger finalization, budget verification, security check

### T-06.1 — Consolidated README ledger pass

- **File:** `integration-tests/README.md`
- **Depends on:** T-01.3, T-02.1, T-03.1/T-03.2, T-04.5, T-05.5 (all module registrations landed)
- **Steps:** update `Total infrastructure tests` to the new count (existing convention: count files/rows as already tracked, following the exact counting rule this document already uses — verify against the file before editing, do not assume the +1-per-file heuristic); confirm all five new category rows exist; confirm all five mutation rows (`M1a`, `M1b`, `M1c`, `M2`, `M3`) exist; confirm the IS-6-retired-into-IS-1 prose note is present.
- **Evidence:** `cargo test --manifest-path integration-tests/Cargo.toml --test ledger` green with zero drift in every direction (file↔module↔ledger).
- **Status:** [x] Done — verified against the file rather than assumed: the counting rule this document already uses is one increment per **file** on disk (`fd . integration-tests/tests/infrastructure -e rs | wc -l` = 21, matching `Total infrastructure tests ... 21` already recorded — no edit needed, the +1-per-file heuristic happened to hold across every group of this change since each landed exactly one new file). Confirmed present: all five new file citations (`durable_store_conformance_postgres.rs`, `lease_contention_postgres.rs`, `events_identity_race_postgres.rs`, `aggregate_type_backfill_postgres.rs` ×4 rows, `pg14_compatibility.rs` ×4 rows); exactly two brand-new category headers (`## Migration transactional behaviour`, `## Version-floor compatibility`) — `design.md`'s File Changes table's own "two new categories" phrasing, distinct from this task's "five new category rows" wording, which this Status line reads as five new row-citations, three landing in pre-existing categories (Durable-adapter conformance; PostgreSQL concurrency invariants ×2) and two founding new categories; all five mutation rows (`M1a`, `M1b`, `M1c`, `M2`, `M3`); the IS-6-retired-into-IS-1 prose note (`integration-tests/README.md:275`, "retired into this same run per D-4/AD-4"). `cargo test --test ledger`: 9/9 passed, zero drift.

### T-06.2 — Full-suite budget verification (SC-9)

- **Depends on:** all prior groups
- **Steps:** run `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` end to end; capture the runner's own timing line.
- **Evidence:** total wall-clock ≤5 minutes; no individual slice >1–2 minutes; compile time reported separately from execution time (the runner's existing behavior, confirmed still true with two containers and five new files).
- **Status:** [x] Done — full end-to-end run, `DOCKER_HOST` exported for colima. Runner's own timing line: `provisioned in 8.10s (PG16) / 8.69s (PG14) · template migrated at 8.80s · suite finished at 13.63s · reclaimed at 13.85s`. The one infrastructure slice (56 passed, 0 failed, 1 ignored) ran in 2.92s of test execution; the runner's whole lifecycle (both containers provisioned, template migrated, suite run, both containers reclaimed) finished at 13.85s — well inside the ≤1–2 min single-slice budget. Total wall clock for the outer `cargo run` invocation (including a from-scratch dependency rebuild after the new `pg14_compatibility.rs` file) was 19.27s, well inside the ≤5 min total budget. Compile time (dependency + target build, reported by cargo's own build lines) is visibly separate from the runner's own execution timing line — confirmed still true with two containers and five new files.

### T-06.3 — Security-skill compliance check (`skills/security/SKILL.md` Rules 1 and 2)

- **Depends on:** all five new test files landed
- **Steps:** scan every `sqlx::query` / `sqlx::query_scalar` call across the five new files and `src/lib.rs`'s two new functions; confirm zero string interpolation or concatenation reaches SQL text — including `wait_until_blocked`'s `statement_like` argument, which is always a hardcoded literal fragment passed as a bound `$1`, never formatted into the query string; confirm every tenant-scoped query (IS-2/IS-5's inserts, IS-1's conformance calls) binds `tenant_id` as a parameter, never derived unbound.
- **Evidence:** report PASS/BLOCK per the security skill's own output contract; zero BLOCK findings required before this change can be considered done.
- **Status:** [x] Done — WARN, zero BLOCK. Scanned every `sqlx::query`/`sqlx::query_scalar`/`sqlx::query_as` call across the five new files (`durable_store_conformance_postgres.rs`, `lease_contention_postgres.rs`, `events_identity_race_postgres.rs`, `aggregate_type_backfill_postgres.rs`, `pg14_compatibility.rs`) and `integration-tests/src/lib.rs`'s two new functions (`wait_until_blocked`, `pg14_database`). Rule 1 (no interpolation/concatenation of user input, variables, or external data into SQL text): zero violations found — every dynamic value (`joined_aggregate_id`, `tenant_id`, `version`, `index_name`, `table_name`, `column_name`, `KEY`, `statement_like`, `AGGREGATE_TYPE`, `occurred_at`, etc.) is passed via `.bind()`; `wait_until_blocked`'s `statement_like` is confirmed always a hardcoded literal fragment at its two call sites, bound as `$1`, never formatted into a query string. `pg14_database()`'s `format!("CREATE DATABASE {name}")` interpolates a database identifier, not data — `name` is built solely from a fixed literal prefix + an internal `AtomicU64` counter, never external/user input, matching the pre-existing `isolated_database()` pattern in the same file; DDL identifiers cannot be bound at all in Postgres's wire protocol, and the closed, code-only generation is the allowlist-equivalent Rule 1 requires for the unbindable case. WARN (advisory, not BLOCK) on Rule 2's letter: three literal SQL text blocks write `tenant_id` as a hardcoded `NULL` rather than binding it — `events_identity_race_postgres.rs:246` and `pg14_compatibility.rs:152,160` (each proving the NULL-tenant/systemwide-uniqueness partition, which is the deliberate fixed subject under test, not derived from any variable, user input, or external source); `pg14_compatibility.rs:190` similarly writes `'tenant-a'` as a literal. Zero actual risk (no variable ever reaches these query strings, so Rule 1 is not implicated), but the letter of Rule 2 ("MUST include tenant_id as a bound parameter") is not met by a hardcoded literal — recommended fix, not required before merge: switch these four literals to `.bind()` calls for consistency with the rest of the suite. Zero BLOCK findings.

### T-06.4 — PR slicing plan (R-7), documentation only

- **Depends on:** nothing (can be written any time, informs merge order)
- **Steps:** record the planned slice order so no single PR exceeds the ~400-line reviewer budget:
  - Slice 1 — T-00.1, T-01.1–T-01.4 (IS-1/IS-6, closes #275 AC10 alone; a complete PR on its own per `design.md`'s Migration/Rollout note).
  - Slice 2 — T-02.1–T-02.2 (IS-3 + its mutation proof).
  - Slice 3 — T-03.1–T-03.2 (IS-2/IS-5).
  - Slice 4 — T-04.1–T-04.6 (IS-4 + M2).
  - Slice 5 — T-00.2, T-05.1–T-05.5 (IS-9/PG14).
  - Slice 6 — T-06.1–T-06.4 (final ledger/budget/security pass, if not already folded into slice 5).
- **Evidence:** each slice's diff size estimated from `design.md`'s File Changes table before opening the PR; any slice trending over budget is split further, never merged to raise the budget (mirrors R-2's own rule for wall-clock).
- **Status:** [x] Done — the six-slice order above is the recorded plan (documentation only, no code change). Diff-size sanity check against `design.md`'s File Changes table: Slice 1 (T-00.1/T-01.1–T-01.4) touches the migration + `durable_store_conformance_postgres.rs` + `lib.rs` scaffolding; Slice 2 (T-02.1–T-02.2) adds one file (`lease_contention_postgres.rs`) + `wait_until_blocked`; Slice 3 (T-03.1–T-03.2) adds one file (`events_identity_race_postgres.rs`); Slice 4 (T-04.1–T-04.6) adds `aggregate_type_backfill_postgres.rs`, exercising the pre-existing `aggregate_type_backfill.rs` module (from PROD-012, unmodified except as M2's mutation-test target) (largest slice, still one new file); Slice 5 (T-00.2/T-05.1–T-05.5) adds `pg14_compatibility.rs` + `pg14_database`; Slice 6 (T-06.1–T-06.4) is README/ledger prose only. Each slice lands exactly one new test file (or zero, for Slice 6), keeping every slice well under the ~400-line budget — no slice needs further splitting.

---

## Group 7 — Issue #275 AC-by-AC mapping (D-7 obligation, documentation only)

**Reconciled against the verbatim issue text.** Issue #275's real body has been fetched and
carries exactly 13 formal acceptance criteria. The table below is the verified AC-by-AC mapping
— not a guess, not a paraphrase pending confirmation.

| AC | Disposition | Evidence |
|---|---|---|
| AC1 | Pre-existing scaffolding, not touched by this change | `integration-tests/Cargo.toml` |
| AC2 | Pre-existing, unaffected by this change (root workspace members list unchanged) | root `Cargo.toml` members list |
| AC3 | Pre-existing; extended (not established) by this change | `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` |
| AC4 | Believed resolved by PROD-012/PROD-012A (per `proposal.md`); guard-script work, out of this change's scope — not a PostgreSQL invariant | requires verification against `scripts/detect-integration-tests.sh` and PROD-012/PROD-012A's own closure evidence, not produced by this change |
| AC5 | Believed resolved by PROD-012/PROD-012A; out of this change's scope, same reasoning as AC4 | same as AC4 |
| AC6 | Pre-existing (single shared PG16 container in `main.rs`). This change's Group 0 (T-00.2) adds a second, distinct shared container for PG14 compatibility only — not a per-test container, and not a departure from "one shared PostgreSQL per run" for the main suite | `integration-tests/src/main.rs`; T-00.2's container-count evidence |
| AC7 | Directly satisfied by this change's own process requirement — every new test file carries the verbatim doc-comment invariant/justification `tasks.md` mandates | every task's "Doc comment (verbatim)" step |
| AC8 | Directly satisfied — T-03.1 explicitly scopes IS-2 to the post-`23505` re-read path only (the pre-check is already covered by IS-1's conformance run, per admission rule 3); IS-6 is retired into IS-1 (T-01.1/T-01.2) rather than duplicated | T-03.1's "Satisfies" field; T-01.1's IS-6 retirement note |
| AC9 | Directly satisfied — `wait_until_blocked` (T-00.1) is the shared poll-with-deadline primitive every contention test in this change uses | T-00.1; its use in T-02.1, T-03.1 |
| AC10 | Closed by this change | T-01.1, T-01.2 |
| AC11 | Already satisfied pre-PROD-015 by the existing single-contender `fencing_window_postgres.rs` (referenced, not created, by T-00.1). This change does not close AC11 — it extends the guarantee to six real contenders (IS-3/T-02.1), a stronger property than the literal AC requires | `integration-tests/tests/infrastructure/fencing_window_postgres.rs` (pre-existing); T-02.1 (extension) |
| AC12 | Pre-existing measurement mechanism; this change must not regress it | T-06.2 |
| AC13 | Process criterion, not a test — satisfied by this document's own invariant-first framing (no test-count target stated anywhere in `tasks.md`) | this document's own structure |

**Repo cross-references confirmed for the dispositions above:** `integration-tests/tests/infrastructure/fencing_window_postgres.rs` pre-dates this change (landed under PROD-012, commit `5f085d1`); `integration-tests/src/main.rs` provisions exactly one `Postgres::default().with_tag("16")` container prior to this change's T-00.2 (AC6, AC11).

**Recommended split (D-7, documentation only — executing it is explicitly outside this SDD change, per proposal and per this task):**
1. Check off AC7, AC8, AC9, AC10, and AC11 on #275 as satisfied by this change, with links to the delivering files (T-01.1/T-01.2 for AC10; T-00.1 for AC7/AC9; T-03.1 for AC8; T-02.1 for AC11's extension).
2. Verify AC4 and AC5 separately against PROD-012/PROD-012A's own closure evidence — this change does not produce that evidence and must not claim it.
3. A future spec named PROD-016 owns HTTP / socket / OTLP verification, if any remaining scope from #275's descriptive sections warrants it.

No issue is created, edited, or closed by this task or by this change.

---

## Seven-gate self-check

| # | Gate | Status | Where enforced |
|---|---|---|---|
| 1 | Atomic tasks, no HTTP/OTLP/other-transport mixing | **Satisfied.** Every task above maps to exactly one PostgreSQL invariant (IS-1 through IS-9, or a shared prerequisite for one). No task touches `crates/transport`, `crates/infrastructure`'s OTLP path, or `examples/reference-app`'s HTTP path; those are named only in Group 7's mapping table as explicitly out of scope | Every task's "Satisfies" field; Group 7 table |
| 2 | PG16 as the main suite | **Satisfied.** Groups 1–4 (IS-1, IS-2, IS-3, IS-4, IS-6) all target the existing shared-per-run PG16 container via `isolated_database()` — no task in those groups touches the PG14 container | T-01.*, T-02.*, T-03.*, T-04.* all use `db = isolated_database()` against the PG16 template |
| 3 | PG14 as a specific compatibility slice, never a second full run | **Satisfied.** Group 5 is exactly four assertions (T0–T3) in one file, scoped to migration 007 and the version-sensitive `NULLS NOT DISTINCT` catalog feature, via the second container T-00.2 provisions (AD-6). T-05.5 explicitly records what is *not* run on PG14 | T-05.1–T-05.5; the "Explicitly not on PG14" note in T-05.5 |
| 4 | Reuse `ego-testkit`, never duplicate conformance | **Satisfied.** T-01.1/T-01.2 call `assert_event_store_conformance` and `assert_reservation_store_conformance` directly against the durable adapters, with only a local fixture type (`ConformanceEvent`), never a re-derived assertion set | T-01.1, T-01.2 |
| 5 | Real contention/fencing as P0 | **Satisfied.** Group 2 (IS-3 + M1a/M1b/M1c) and Group 3's T-03.1 (IS-2, post-`23505` scope only) are both marked P0 explicitly | Priority legend; Group 2/Group 3 headers |
| 6 | D-8 respected on any exposed defect | **Satisfied.** Every task touching `aggregate_type_backfill.rs` (T-04.2 specifically) or the `events` table's conflict-handling path (T-03.1) carries an explicit D-8 note: fix only if strictly necessary for the exact invariant, no new API/contractual behavior/architecture/migration, else a named follow-up spec. No task pre-authorizes a fix | T-01.3, T-03.1, T-04.2 "D-8 note" fields |
| 7 | Verifiable evidence per task, ES/EN parity | **Satisfied.** Every task above carries a concrete "Evidence" field — a command, an assertion, or a value to capture, never bare "test passes." `tasks.es.md` mirrors this file 1:1: same task IDs, same ordering, same evidence, faithfully translated | Every task's "Evidence" field; `tasks.es.md` |

**Exception, stated rather than hidden:** the #275 AC-by-AC mapping in Group 7 has been fully
reconciled against the verbatim issue text — it is no longer a gap. Gates 1–6 remain
self-contained within this change's own files and were unaffected by the reconciliation.
