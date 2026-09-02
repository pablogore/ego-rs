# Delta Specs: PROD-014B — PostgreSQL Durable Read-Side Stores

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).
> Single file covering two capabilities, per this change's Capabilities section: one new
> (`read-side-durable-progress`) and one modified (`read-side`).

## Capability: `read-side-durable-progress` (NEW)

### Purpose

The observable durability contract for read-side progress state backed by PostgreSQL: what
survives a process restart, what identity each record has, what retention is promised, and
explicitly what concurrency guarantee is **not** offered. This capability governs the durable
`OffsetStore`/`DedupStore` pair's observable behavior only — it does not cover the underlying
interface those stores implement, atomic event claiming, or dedup retention/eviction (see
Non-Goals).

## Requirements

### Requirement: Offset Survives a Process Restart

The system MUST persist a projection's offset for a given `(projection_id, tag, tenant)` such
that after a process restart, reading that offset returns the last persisted value rather than
an absent value or a value requiring stream replay from the beginning.

#### Scenario: Restart resumes from the last persisted offset

- GIVEN a projection has written offset N for `(projection_id, tag, tenant)` through the
  durable pair
- WHEN the process restarts and the offset is read for the same
  `(projection_id, tag, tenant)`
- THEN it returns N — not absent, and not a replay from the beginning

### Requirement: Absent Offset Reads Are Tenant-Isolated

Reading an offset for a `(projection_id, tag, tenant)` that was never written MUST return an
absent value, and MUST NEVER return another tenant's offset for the same
`(projection_id, tag)`.

#### Scenario: An unwritten offset returns absent, never another tenant's value

- GIVEN offsets exist for tenant A on `(projection_id, tag)` but were never written for
  tenant B
- WHEN the offset is read for tenant B on the same `(projection_id, tag)`
- THEN it returns absent — never tenant A's offset

### Requirement: Repeated Dedup Marks Converge to One Record

Marking the same `(projection_id, tag, event_id)` as seen more than once — sequentially or
concurrently — MUST succeed on every call, MUST leave exactly one dedup record for that
identity, and MUST NOT raise an error on the repeat.

#### Scenario: A duplicate mark converges without error

- GIVEN `(projection_id, tag, event_id)` has already been marked seen
- WHEN it is marked seen again
- THEN the second call also succeeds, exactly one record exists for that identity, and a
  subsequent seen-check returns true

### Requirement: Dedup Identity Is Tenant-Independent

The same `event_id` marked under two different tenants for the same `(projection_id, tag)`
MUST be treated as one identity — tenant is NOT part of dedup identity.

#### Scenario: A second tenant's identical event_id is already seen

- GIVEN `event_id` has been marked seen for `(projection_id, tag)` under tenant A
- WHEN the same `event_id` is marked seen under tenant B for the same `(projection_id, tag)`
- THEN it is reported as already seen — dedup identity does not vary by tenant

### Requirement: Offset Writes Are Last-Write-Wins

A write to a projection's offset MUST overwrite the previously stored value with no
compare-and-swap, no expected-previous-offset check, and no detection of a concurrent
overwrite. This is a faithful implementation of the offset store's own existing write
contract, not an adapter-level shortcoming.

#### Scenario: A later write silently overwrites an earlier one

- GIVEN an offset already written for `(projection_id, tag, tenant)`
- WHEN a second write for the same identity is issued with a different value, with no
  ordering coordination between the two writers
- THEN the stored value becomes whatever was written last, with no error and no conflict
  signal raised to either writer

### Requirement: Both Progress Stores Report Themselves As Durable

The offset store and the dedup store MUST both report themselves as durable, and a
composition declaring the production durability profile that registers this pair through the
existing read-side progress registration point MUST build successfully with no change to that
profile's own validation logic.

#### Scenario: A production-profile composition passes on real durability

- GIVEN a composition declaring the production durability profile
- WHEN it registers this durable pair through the existing read-side progress registration
  point
- THEN composition succeeds because both stores report themselves durable, not because of a
  test-only substitute

### Requirement: Tenant Is a Required Part of Offset Identity

Every persisted offset record MUST carry a concrete tenant value. The persisted offset
storage MUST NOT accept, and MUST NOT ever contain, an absent/null tenant on an offset
record — unlike the nullable/systemwide-tenant handling used elsewhere in this framework for
write-side stores, which does not apply here.

#### Scenario: An offset record always carries a concrete tenant

- GIVEN a projection writing an offset
- WHEN the write is persisted
- THEN the stored record's tenant field holds the concrete tenant value the write was made
  for — never an absent/null value

### Requirement: Dedup Storage Growth Is Unbounded In This Capability

This capability MUST NOT ship any purge, time-to-live, or eviction mechanism for dedup
records. Dedup storage grows monotonically with the number of unique events processed by a
projection, without an upper bound, for as long as this capability alone governs it. This is
an explicit, named limitation, not an omission.

#### Scenario: Dedup records accumulate with no automatic removal

- GIVEN a projection has processed many unique events over time
- WHEN the dedup records for those events are inspected
- THEN every one of them is still present — nothing in this capability has purged, expired,
  or evicted any of them

### Requirement: The Reference Application's Production Path Uses the Durable Pair

The reference application's production composition path MUST register a durable, real
progress pair for its read-side projection rather than omitting read-side progress or
substituting a non-durable placeholder.

#### Scenario: Production composition no longer omits read-side progress

- GIVEN the reference application composing itself under the production durability profile
- WHEN its read-side progress registration point is reached
- THEN it registers a real durable pair — not an absent value, and not a non-durable
  placeholder

### Requirement: The Single-Writer Adoption Constraint Is Documented at the Adapter Level

The public documentation for this capability's adapters MUST state, in words an operator can
read, that safe operation depends on single-writer-per-`(projection_id, tag, tenant)`, and
MUST NOT present a multi-replica projection configuration as officially supported.

#### Scenario: Adapter documentation states the adoption constraint

- GIVEN an operator reading this capability's adapter-level public documentation
- WHEN they look for guidance on running concurrent replicas of the same projection
- THEN the documentation states the single-writer-per-`(projection_id, tag, tenant)` adoption
  constraint explicitly, and does not describe a multi-replica configuration as supported

## Capability: `read-side` (MODIFIED)

## ADDED Requirements

### Requirement: Durable Dedup Bookkeeping Does Not Imply Exactly-Once Handler Execution

Persisting dedup bookkeeping durably MUST NOT be read, described, or documented as
exactly-once event handling anywhere in this system. This capability delivers at-least-once
handler execution with best-effort dedup bookkeeping; nothing in it prevents a handler from
running more than once for the same event under concurrent writers.

#### Scenario: Two concurrent writers may each run the handler once

- GIVEN two writers processing the same `(projection_id, tag, tenant)` concurrently, both
  checking whether an event has already been seen before either records it as seen
- WHEN both observe the event as not-yet-seen before either records it
- THEN the handler MAY run for both writers; this capability does not prevent that outcome,
  and no documentation may describe it as prevented

### Requirement: Prevention of Double Handler Execution Rests on an Explicit, Unenforced Single-Writer Adoption Constraint

Prevention of double handler execution for the same event MUST be stated as depending on an
external, unenforced adoption constraint — **single-writer-per-`(projection_id, tag,
tenant)`** — never as a guarantee this capability itself enforces. No leader election, lock,
lease, or fencing mechanism exists in this capability to enforce that constraint across
multiple replicas of the same projection. This is the change's binding adoption constraint:
adopting this durable pair in production is conditioned on it holding.

#### Scenario: A two-replica deployment is outside the guarantee, and undetected

- GIVEN two replicas of the same projection process running concurrently against the same
  `(projection_id, tag, tenant)`
- WHEN this configuration is evaluated against this capability's guarantees
- THEN it is outside the guarantee this capability provides, and nothing in this capability
  detects or refuses that configuration

### Requirement: The Concurrency Gap Has a Named, Distinct Follow-Up

The gap between durable dedup bookkeeping and prevention of double handler execution MUST be
recorded as a distinct, named follow-up — **PROD-014C — Atomic Read-Side Event Claiming** —
rather than folded into this capability's scope or silently left unowned.

#### Scenario: The follow-up is named, not implied

- GIVEN a reader of this capability's documentation looking for how double handler execution
  will eventually be prevented
- WHEN they look for the owning follow-up
- THEN they find it named as PROD-014C — Atomic Read-Side Event Claiming, distinct from and
  not part of this capability

## Non-Goals

- No change to the `OffsetStore`/`DedupStore` interface, the production durability profile's
  gate logic, or the existing read-side progress registration mechanism.
- No atomic event-claiming, reservation, leader election, lock, lease, or fencing token — all
  are PROD-014C — Atomic Read-Side Event Claiming's scope, not this capability's.
- No peer/replica detection of any kind.
- No dedup retention, TTL, or eviction mechanism.
- No backend other than PostgreSQL.
- No removal, deprecation, or hiding of the existing in-memory or fake-durable progress pairs
  — they remain valid for Dev and tests.
- No multi-worker ownership, partition leasing, high availability, exactly-once delivery, or
  projection-rebuild orchestration.
- No change to a projection spawned outside the composition root.
- No durable `ReadSideStore` (the event view a projection polls) — a separate, still-open
  item.
