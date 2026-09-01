# Real Infrastructure Verification Specification

## Purpose

Which invariants MUST be demonstrated against real PostgreSQL, the admission contract that
keeps the suite small, and the wall-clock budget. This capability governs the verification
*methodology*; the durable-adapter behavioral obligations themselves are stated in the
`event-store` and `idempotent-command-processing` specs.

## Requirements

### Requirement: Durable Adapters Are Proven Against the Existing Conformance Harnesses, Verbatim

The verification suite MUST run `assert_event_store_conformance`
(`crates/testkit/src/event_store.rs:69`) against `PostgreSQLEventStore` and
`assert_reservation_store_conformance` (`crates/testkit/src/reservation_conformance.rs:963`)
against `PostgresOperationReservationStore`, using the exact same `ego-testkit` assertion
definitions the in-memory callers use — never a parallel or re-derived copy. Unit-of-work
atomicity (drop-without-commit persists nothing; an open unit of work is invisible to a
concurrent reader, `crates/testkit/src/event_store.rs:328-375`) MUST be demonstrated through
this same conformance run against `PostgreSQLEventStore`'s distinct pooled connection, not
through a separate bespoke test, unless that distinct-connection property is shown false — in
which case a dedicated follow-up requirement is warranted, not assumed here.

#### Scenario: Durable adapters pass the identical assertions the in-memory adapters pass
- GIVEN `PostgreSQLEventStore` and `PostgresOperationReservationStore`
- WHEN each is driven through its respective conformance harness
- THEN every assertion the in-memory implementations already satisfy also passes against the
  durable adapter, with no re-derived or weakened assertion set

#### Scenario: Unit-of-work atomicity is demonstrated by the conformance run, not a new test
- GIVEN the conformance run against `PostgreSQLEventStore`
- WHEN a staged, uncommitted append is read from a second pooled connection
- THEN it is invisible, and a dropped-without-commit unit of work persists nothing — both
  observed as part of the conformance run itself, with no additional test file created for
  this invariant

### Requirement: Migration 007's Backfill Is Provably Transactional

`aggregate_type_backfill.rs` (`crates/persistence/src/postgres/aggregate_type_backfill.rs`)
MUST be exercised against a real, migrated PostgreSQL database and MUST demonstrate: an abort
before its first `UPDATE` leaves the table byte-identical; a run over zero eligible rows
commits without side effects; and a revert rejoins exactly the state that preceded the
backfill.

#### Scenario: Abort before the first UPDATE leaves the table untouched
- GIVEN a migrated database with rows eligible for backfill
- WHEN the backfill transaction aborts before its first `UPDATE` executes
- THEN the table is byte-identical to its pre-backfill state

#### Scenario: An explicit rollback after a completed UPDATE leaves the table untouched
- GIVEN a migrated database with rows eligible for backfill, and a backfill transaction that
  has executed at least one `UPDATE`
- WHEN the transaction is explicitly rolled back rather than committed
- THEN the table is byte-identical to its pre-backfill state, proving the rollback — not just
  statement ordering — is what guarantees no partial effect

#### Scenario: A zero-row run commits cleanly
- GIVEN a migrated database with no rows eligible for backfill
- WHEN the backfill runs to completion
- THEN the transaction commits with no rows changed and no error

#### Scenario: A revert rejoins exactly the prior state
- GIVEN a completed backfill
- WHEN its revert path runs
- THEN the database state is identical to the state immediately before the backfill ran

### Requirement: Every New Verification Test States Its Own Admission, or It Is Not Admitted

Each test file added under this verification effort MUST state, in its own doc comment, the
exact invariant it demonstrates and why that invariant cannot be demonstrated in-process, by
contract, by conformance, or at compile time. Each such test MUST be reflected consistently
across its module registration and its tracked ledger entry, with zero drift between the two
and the tree. Because the suite's end-to-end budget is already fully spent, no test added under
this capability MAY be filed as a new end-to-end scenario; each MUST file under a non-end-to-end
category and state its own infrastructure risk.

#### Scenario: A test with no stated justification is not admitted
- GIVEN a candidate test file with no doc comment stating its invariant and why-not-in-process
  justification
- WHEN the suite's admission check runs
- THEN the test is not admitted

#### Scenario: Registration and ledger tracking never drift from the tree
- GIVEN a new test file added to the suite
- WHEN the suite's consistency check runs
- THEN the file's module registration and its ledger entry both exist and agree with what is
  on disk — a mismatch in either direction fails the check

#### Scenario: No new end-to-end scenario is created
- GIVEN the end-to-end budget is already fully spent
- WHEN a new test is added under this verification effort
- THEN it is filed under a non-end-to-end category with its own stated infrastructure risk,
  never as a fifth end-to-end scenario

### Requirement: The Highest-Criticality Invariants Are Proven by Mutation, Not Only by a Passing Test

For the two highest-criticality invariants — the many-contender fencing race and migration
007's transactional/unit-of-work atomicity — the verification suite MUST demonstrate that
neutralizing the mechanism under test causes the corresponding new test to fail, while the
pre-existing suite remains green.

#### Scenario: Neutralizing the fencing mechanism fails the fencing test
- GIVEN the many-contender fencing test and the pre-existing single-contender fencing test
  (`fencing_window_postgres.rs`), both passing against the real mechanism
- WHEN the fencing mechanism is neutralized
- THEN both the many-contender fencing test and the pre-existing single-contender fencing test
  fail, because they share the same load-bearing predicate, while every test that does not
  exercise that predicate remains unaffected

#### Scenario: Neutralizing transactional/unit-of-work atomicity fails the corresponding test
- GIVEN the migration 007 and unit-of-work atomicity tests passing against the real mechanism
- WHEN that atomicity mechanism is neutralized
- THEN the corresponding test fails, and the rest of the pre-existing suite stays green

### Requirement: PostgreSQL Version-Compatibility Verification Is a Narrow Slice, Never a Second Full Run

PG14 MUST remain a verified, supported compatibility floor. Only the version-sensitive
invariants — migration 007's backfill and any SQL/catalog feature genuinely capable of
diverging across PostgreSQL versions — MUST be proven against PG14, through a separate, narrow
slice. The main suite's contention, fencing, unit-of-work and concurrency invariants (covered
above and in the `event-store` and `idempotent-command-processing` specs) MUST continue
running against PG16 only. This capability MUST NOT be satisfied by re-running the main suite a
second time against PG14.

#### Scenario: The PG14 slice covers only version-sensitive invariants
- GIVEN the PG14 compatibility slice
- WHEN its test set is enumerated
- THEN every test in it targets a named version-sensitive invariant (migration 007, or a named
  SQL/catalog feature that could genuinely diverge) — no contention, fencing, or unit-of-work
  test appears in it

#### Scenario: The main suite is never duplicated against PG14
- GIVEN the full main suite already passing against PG16
- WHEN the PG14 slice is evaluated for completeness
- THEN it is a small, distinct test set, never a second execution of the main suite's
  contention, fencing or unit-of-work tests against PG14

## Non-Goals

- Readiness-probe down/up transition testing (OOS-2) — connection-pool resilience under real
  network conditions, not a PostgreSQL SQL/transaction/fencing guarantee; unit-level coverage
  already exists.
- The `i64::MAX` fencing-exhaustion boundary (OOS-3) — already covered by existing in-process
  unit tests; real PostgreSQL adds nothing to that boundary.
- Creating the `integration-tests/` workspace, runner, or ledger guard (OOS-4) — all already
  exist and are extended, not built.
- HTTP, socket, OTLP, or CORE-018 real-HTTP end-to-end verification (OOS-1) — non-PostgreSQL,
  hermetic-loopback-classified, reserved for a future PROD-016 at naming level only.
- Fixing production defects a new test exposes, beyond a small localized fix the design phase
  explicitly accepts for IS-4 or IS-2 (OOS-7).
- Docker Compose, anywhere, for anything (OOS-8).
