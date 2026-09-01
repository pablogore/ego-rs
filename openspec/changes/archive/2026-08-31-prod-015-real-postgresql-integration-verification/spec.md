# Delta Specs: PROD-015 — Real PostgreSQL Integration Verification

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).
> Single file covering three capabilities, per this change's Capabilities section: one new
> (`real-infrastructure-verification`) and two modified (`event-store`,
> `idempotent-command-processing`).

## Capability: `real-infrastructure-verification` (NEW)

### Purpose

Which invariants MUST be demonstrated against real PostgreSQL, the admission contract that
keeps the suite small, and the wall-clock budget. This capability governs the verification
*methodology*; the durable-adapter behavioral obligations themselves are stated in the
`event-store` and `idempotent-command-processing` deltas below.

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
above and in the `event-store` and `idempotent-command-processing` deltas below) MUST continue
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

## Capability: `event-store` (MODIFIED)

## ADDED Requirements

### Requirement: Event Store Conformance Extends to Durable Adapters

`PostgreSQLEventStore` MUST satisfy the identical `assert_event_store_conformance` definitions
that govern the in-memory implementation. Passing the in-memory conformance suite alone MUST
NOT be treated as sufficient evidence that a durable implementation is compliant.

#### Scenario: A durable event store that fails conformance is non-compliant
- GIVEN `PostgreSQLEventStore` driven through `assert_event_store_conformance`
- WHEN any assertion in that harness fails
- THEN `PostgreSQLEventStore` is not compliant with this capability, regardless of the in-memory
  implementation's status

### Requirement: NULL-Tenant Stream Identity Honors SQL's Three-Valued Comparison Behaviorally

Stream-identity comparisons involving a `NULL`/systemwide tenant (`Option::None`) MUST be
verified behaviorally against real PostgreSQL, not only asserted from the schema/catalog.
Ordinary equality comparison under three-valued logic (`NULL = NULL` is not true) MUST NOT
cause a systemwide-tenant stream to be silently missed by, or silently merged with, another
systemwide-tenant stream during identity resolution.

#### Scenario: Two distinct systemwide-tenant streams resolve independently
- GIVEN two events stored under distinct aggregates, both with `Option::None` tenant
- WHEN each stream's identity is resolved
- THEN each resolves independently to its own stream, with no false collision or false miss
  caused by NULL's three-valued comparison behavior

## MODIFIED Requirements

### Requirement: Effective Uniqueness on the Event Stream Identity

The event store MUST reject a second event written for the same
`(tenant_id, aggregate_type, aggregate_id, version)` tuple — including when
`tenant_id` represents the NULL/systemwide tenant. A duplicate MUST be
rejected by the store itself, not merely by application-level discipline. Against a real,
concurrent writer population, a rejected duplicate MUST surface as a conflict reporting the
**real current version** of the stream, and an N-way concurrent append race targeting one
stream MUST leave exactly one winner.
(Previously: stated only the rejection outcome; did not state what the rejection reports under
real concurrent contention, nor the N-way race outcome.)

#### Scenario: Duplicate version for the same tenant-scoped aggregate is rejected
- GIVEN an event already stored for `(tenant-a, User, user-7, version=3)`
- WHEN a second event is appended for the identical tuple
- THEN the store rejects the second append as a uniqueness violation

#### Scenario: Duplicate version under the NULL-tenant systemwide mode is also rejected
- GIVEN an event already stored for `(NULL, TenantOrganization, org-1,
  version=1)` in the systemwide tenant-less mode
- WHEN a second event is appended for the identical systemwide tuple
- THEN the store rejects the second append — NULL tenant identity does not
  exempt the tuple from uniqueness enforcement

#### Scenario: An N-way concurrent append race leaves exactly one winner, each reporting the real version
- GIVEN N concurrent callers each appending the next event to the identical stream
- WHEN all N appends are attempted concurrently
- THEN exactly one append succeeds, the remaining N-1 are rejected as conflicts, and each of
  those N-1 conflicts reports the stream's real, winning current version — obtainable only
  under genuine concurrent contention, past the point where the store's own transaction has
  already aborted and must re-read the stream on another connection, not from the single-caller
  stale-expected-version pre-check the conformance harness already exercises

## Capability: `idempotent-command-processing` (MODIFIED)

## ADDED Requirements

### Requirement: Reservation Store Conformance Extends to the Durable Adapter

`PostgresOperationReservationStore` MUST satisfy the identical
`assert_reservation_store_conformance` definitions that govern the harness's existing callers.
Passing those assertions in a non-durable test context alone MUST NOT be treated as sufficient
evidence that the durable adapter is compliant.

#### Scenario: A durable reservation store that fails conformance is non-compliant
- GIVEN `PostgresOperationReservationStore` driven through
  `assert_reservation_store_conformance`
- WHEN any assertion in that harness fails
- THEN `PostgresOperationReservationStore` is not compliant with this capability

## MODIFIED Requirements

### Requirement: Lease With Owner, Expiry, and Verified Fencing

A reservation in progress MUST be governed by a lease carrying `owner_id`,
`lease_until`, and `fencing_token`. Every renewal, completion, or abandonment
of a reservation MUST perform a conditional update verifying
`operation_id + owner_id + fencing_token` together — storing a fencing token
without verifying it on every mutating call does NOT satisfy this
requirement. An update presented by an owner whose lease has expired MUST be
rejected with `StaleOwner`, and that owner MUST NOT be able to close or renew
the operation afterward. A later caller MUST be able to take over an expired
lease atomically, fencing out the prior owner. This MUST hold under real
multi-contender concurrency, not only the single-contender case: when multiple
contenders race one expired lease, exactly one MUST win, and the fencing
token MUST advance by exactly one — never by the number of contenders.
(Previously: stated the single-contender takeover guarantee only; did not state the
many-contender race outcome.)

#### Scenario: Conditional update rejects a stale owner
- GIVEN a reservation whose lease expired and was taken over by a new owner
- WHEN the original owner attempts to complete the reservation
- THEN the conditional update fails, `StaleOwner` is returned, and the
  reservation is not modified by the stale caller

#### Scenario: Atomic takeover fences out the prior owner
- GIVEN a reservation with an expired lease
- WHEN a new caller takes over the reservation
- THEN the takeover succeeds atomically with a new `fencing_token`, and any
  subsequent call from the prior owner's fencing token fails

#### Scenario: Storing a token without verifying it is insufficient
- GIVEN an implementation that persists `fencing_token` but does not compare
  it on renew/complete/abandon
- WHEN a stale owner issues a renew after takeover
- THEN this requirement is NOT satisfied — the conditional-update comparison
  is mandatory, not merely storing the value

#### Scenario: Six contenders racing one expired lease leave exactly one winner
- GIVEN a reservation with an expired lease and six concurrent contenders attempting takeover
- WHEN all six attempts race concurrently
- THEN exactly one contender wins the takeover, and the fencing token advances by exactly one —
  not by six

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
