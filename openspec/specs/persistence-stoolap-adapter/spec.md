# Spec: `persistence-stoolap-adapter` (New Capability)

> Canonical / English. Spanish companion: `spec.es.md` (1:1 requirement IDs and scenarios).
> Source of truth for requirement content: STOOLAP-S1's `proposal.md` (D-1..D-12, IS-1..IS-6,
> R1..R14) and `design.md` (EC-1..EC-7, AD-1..AD-11, OQ-1..OQ-3). This capability describes the
> **observable** contract a caller of `Repository<A>` sees when the Stoolap-backed implementation
> is in use — what a save, load, or delete returns, not how it is computed. No requirement here
> names a storage mechanism, a query language, an index, or an internal encoding; those choices
> belong to `design.md` and may change without this spec changing, as long as every scenario below
> keeps holding.

## Purpose

The observable contract that a Stoolap-backed implementation of `ego_persistence_api::persistence::Repository<A>`
exists; that it scopes aggregates by tenant, with the systemwide (no-tenant) scope isolated from
every concrete tenant and equal to itself across calls; that it enforces optimistic concurrency,
reporting every lost race as the same conflict outcome; that a committed save survives an unclean
restart of the process that made it; that its error outcomes never exceed the four already
shipped by `Repository<A>`; and that its externally observable behavior is indistinguishable from
the in-memory and PostgreSQL-backed implementations for every scenario the shared conformance
suite covers. It also states, as an explicit boundary rather than a silent omission, that this
capability's guarantees hold only within one owning process and make no promise about two or more
separate operating-system processes sharing the same store.

This capability does not cover any other Stoolap-backed store (`EventStore`, `Snapshot`,
`OperationReservationStore`, `OffsetStore`, `DedupStore`), any generic storage-backend
abstraction, any backend beyond Memory/PostgreSQL/Stoolap, or any change to `Repository<A>`'s own
method set, bounds, or error type.

## Requirements

### Requirement: R1 — A Stoolap-Backed Repository Implementation Exists and Round-Trips an Aggregate

A Stoolap-backed implementation of `Repository<A>` MUST exist, and a caller MUST be able to save a
new aggregate, load it back with identical content, save an update to it with the version
advancing by exactly one on every successful save, and later delete it.

#### Scenario: A fresh aggregate save starts at version one

- GIVEN an aggregate that has never been saved
- WHEN a caller saves it with an expected version of zero
- THEN the save succeeds and reports version one

#### Scenario: Sequential successful saves advance the version by exactly one each time

- GIVEN an aggregate already saved at version one
- WHEN a caller saves it again with an expected version of one
- THEN the save succeeds and reports version two

#### Scenario: A loaded aggregate matches the most recently saved content

- GIVEN an aggregate saved, then saved again with updated content
- WHEN a caller loads it
- THEN the loaded content matches the most recent save, not an earlier one

### Requirement: R2 — Memory, PostgreSQL, and Stoolap Satisfy One Shared Behavioral Contract

The in-memory, PostgreSQL-backed, and Stoolap-backed implementations of `Repository<A>` MUST be
judged against one identical, shared set of conformance scenarios, not against three separate
readings of the contract. Passing that shared suite MUST be required of all three implementations
for every scenario it defines, with no per-implementation variant and no skipped scenario. (R6
below names the one scenario deliberately excluded from this suite, and states why.)

#### Scenario: The same conformance scenarios run against all three implementations

- GIVEN a shared conformance suite covering fresh-save versioning, sequential version
  advancement, stale-version conflict, load round-trip, not-found on absent load and delete, real
  deletion, missing-tenant rejection, and the three tenant-isolation scenarios of R3
- WHEN that suite is run against the in-memory, PostgreSQL-backed, and Stoolap-backed
  implementations
- THEN every scenario passes identically against all three, with no scenario skipped or varied
  per implementation

### Requirement: R3 — Tenant Scopes Are Isolated From Each Other, Including the Systemwide Scope

An aggregate saved under one tenant scope MUST never be visible under, confused with, or
overwritten by a save under a different tenant scope or under the systemwide (no-tenant) scope,
even when both share the same aggregate identity. This MUST include the systemwide scope itself,
which round-trips through save, load, save-again, and delete exactly as a named tenant scope does.

#### Scenario: The systemwide scope round-trips through save, load, save, and delete

- GIVEN an aggregate identity saved with no tenant specified (systemwide scope)
- WHEN it is saved, loaded, saved again with an advanced version, and then deleted
- THEN each step succeeds exactly as it would for a named tenant scope, and the aggregate is gone
  after delete

#### Scenario: Two different tenants sharing one aggregate identity do not collide

- GIVEN the same aggregate identity saved independently under two different tenant scopes
- WHEN each is loaded
- THEN each returns only the content saved under its own tenant, and neither is visible under the
  systemwide scope

#### Scenario: A tenant scope and the systemwide scope sharing one aggregate identity do not collide

- GIVEN the same aggregate identity saved independently under a named tenant scope and under the
  systemwide scope
- WHEN the systemwide-scoped aggregate is deleted
- THEN the tenant-scoped aggregate remains intact and unaffected

### Requirement: R4 — An Empty Tenant Identifier Is Rejected, Never Coerced

A caller that passes an empty string as a tenant identifier MUST receive a
`PersistenceError::MissingTenant` rejection on save, load, and delete alike; it MUST never be
silently treated as the systemwide scope or as a valid tenant.

#### Scenario: An empty tenant identifier is rejected on every operation

- GIVEN an empty string passed as the tenant identifier
- WHEN a caller attempts to save, load, or delete an aggregate under it
- THEN each operation is rejected as `PersistenceError::MissingTenant`, and none succeeds

### Requirement: R5 — A Stale Expected Version Is Rejected as a Conflict, Truthfully

When a caller saves an existing aggregate with an expected version that no longer matches the
aggregate's current stored version, the save MUST be rejected as `PersistenceError::Conflict`
reporting both the version the caller expected and the version actually stored, rather than being
silently accepted or misreported.

#### Scenario: Saving with an outdated expected version is rejected

- GIVEN an aggregate currently stored at version two
- WHEN a caller saves it with an expected version of one
- THEN the save is rejected as a conflict reporting expected one and actual two, and the stored
  aggregate is unchanged

### Requirement: R6 — A Fresh Aggregate Rejects a Nonzero Expected Version, Matching the Documented Semantics

When a caller saves an aggregate that has never been saved before (no prior save exists for that
scope and identity) using a nonzero expected version, the Stoolap-backed implementation MUST
report a version conflict rather than accepting the write — matching both the in-memory
implementation's behavior and `Repository<A>`'s own documented contract ("use 0 for new
aggregates"). This scenario MUST NOT appear in the shared conformance suite of R2: the two
previously-shipped implementations are already known to disagree on this exact case (the
PostgreSQL-backed implementation currently accepts the write instead of reporting a conflict),
and reconciling that disagreement is out of scope for this capability, tracked separately as its
own follow-up. Excluding the scenario from the shared suite is not a statement that the
Stoolap-backed implementation's behavior is wrong; it is a statement that the shared suite asserts
only what all three implementations are actually required to agree on today.

#### Scenario: A brand-new aggregate saved with a nonzero expected version is rejected as a conflict

- GIVEN no aggregate has ever been saved under a given tenant scope and aggregate identity
- WHEN a caller saves that aggregate with a nonzero expected version
- THEN the save is rejected as a conflict reporting an actual version of zero, not accepted

#### Scenario: This case is not exercised by the shared cross-backend conformance suite

- GIVEN the shared conformance suite defined in R2
- WHEN the suite's scenario list is inspected
- THEN it contains no scenario covering a fresh aggregate saved with a nonzero expected version,
  because the previously-shipped implementations are already known to disagree on this exact
  case, independently of this capability

### Requirement: R7 — Loading or Deleting an Absent Aggregate Reports Not Found

Loading or deleting an aggregate that was never saved, or that was already deleted, MUST be
reported as `PersistenceError::NotFound` rather than as an empty result, a default value, or an
error that misrepresents the cause.

#### Scenario: Loading an aggregate that was never saved reports not found

- GIVEN an aggregate identity that has never been saved
- WHEN a caller loads it
- THEN the load is rejected as not-found

#### Scenario: Deleting an aggregate that was never saved reports not found

- GIVEN an aggregate identity that has never been saved
- WHEN a caller deletes it
- THEN the delete is rejected as not-found

### Requirement: R8 — Delete Removes the Aggregate Permanently

Deleting an aggregate MUST make it genuinely absent: a subsequent load MUST report not-found, not
an empty or tombstoned value.

#### Scenario: A deleted aggregate is truly gone

- GIVEN an aggregate that has been saved
- WHEN a caller deletes it and then attempts to load it
- THEN the load is rejected as not-found

### Requirement: R9 — A Committed Save Survives an Unclean Process Restart

Once a save has completed successfully, its data MUST be present after the process that made it
stops running (including an unclean stop) and a new process reopens the same store at the same
location.

#### Scenario: Data from a completed save is present after the process restarts

- GIVEN a save that has completed successfully against a store at a given location
- WHEN the process is stopped and a new process reopens the store at that same location
- THEN loading the saved aggregate returns the content that was saved, unchanged

### Requirement: R10 — Every Write Conflict Is Reported Through the Same Existing Conflict Outcome

Whether a save is rejected because of a stale expected version or because it lost a race against
a concurrent write to the same aggregate, both cases MUST be reported through the identical
`PersistenceError::Conflict` outcome a caller already knows how to handle by reloading and
retrying. No new, additional, or differently-shaped error outcome MUST be introduced for either
case, and a caller MUST NOT be able to distinguish "stale version" from "lost a race" by outcome
shape alone.

#### Scenario: A concurrent write race is reported the same way as a stale version

- GIVEN two callers concurrently attempting to save the same existing aggregate
- WHEN both attempts are made at nearly the same time
- THEN exactly one save succeeds, and the other is rejected with the same conflict outcome R5
  describes for a stale expected version

### Requirement: R11 — No Internal Storage Detail Is Ever Visible to a Caller

Nothing about how a tenant scope or the systemwide scope is represented internally MUST ever be
exposed to a caller — not in a returned value, not in an error message, and not in any behavior
difference a caller could observe. A caller's view of tenant scoping MUST be identical across all
three implementations of `Repository<A>` (R2).

#### Scenario: No error or return value reveals internal scope representation

- GIVEN any save, load, or delete call across any tenant scope, including the systemwide scope
- WHEN the call's result or any error it produces is inspected
- THEN nothing in it reveals how the scope was represented internally, and the caller-visible
  behavior is indistinguishable from the other two implementations of `Repository<A>`

### Requirement: R12 — Concurrent Access Is Guaranteed Only Within a Single Owning Process

The Stoolap-backed implementation's guarantees — including tenant isolation (R3) and
optimistic-concurrency conflict detection (R5, R10) — MUST hold only among callers within one
owning process. This capability makes **no guarantee**, and it MUST NOT be relied upon, for
correct or safe behavior when two or more separate operating-system processes access the same
underlying store concurrently. A deployment that needs multiple processes to share one store's
data is out of scope for this capability.

#### Scenario: Concurrent callers within one process see correct conflict detection

- GIVEN two concurrent save attempts against the same aggregate, issued by two callers within one
  owning process
- WHEN both attempts race
- THEN exactly one succeeds and the other is reported as a conflict, per R10

#### Scenario: Multi-process concurrent access is an explicit non-guarantee

- GIVEN a deployment that runs two or more separate operating-system processes against the same
  underlying store
- WHEN those processes access the store concurrently
- THEN this capability documents no guarantee of correct or safe behavior for that scenario, and
  such a deployment is outside this capability's scope
