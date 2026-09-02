# Spec: Read-Side Progress — Lifecycle Spawn/Stop & Durable Storage

## Capability: read-side

Purpose: `TagSchedulerImpl` gains one call, `spawn_projection`, that wires
the stop/join lifecycle plumbing every consuming application previously
hand-rolled around `run_until_stopped` — spawning the poll loop and returning
a handle whose `stop()` consumes itself, waits for an in-flight batch to
drain, and surfaces a failed drain instead of swallowing it.

Additionally, this capability documents the critical constraints on durable
progress store implementations when used in production: durable dedup
bookkeeping does not imply exactly-once handler execution, and prevention of
double handler execution rests on an explicit, unenforced single-writer
adoption constraint. This capability specifies only the spawn/stop lifecycle
wrapper's observable behavior and these adoption constraints; the underlying
scheduler engine itself (`TagSchedulerImpl`, `run_until_stopped`, and friends)
and the interface of the `OffsetStore` and `DedupStore` are out of scope and
unchanged.

**Explicitly not covered by this capability** (see Non-Goals): constructing
a dedup store, an offset store, a tag-discovery mechanism, a handler, or the
application's own queryable read model. All of these remain the calling
application's responsibility, exactly as before this capability exists — a
consuming application still assembles them itself (e.g. reference-app's own
`ReadSideHandles`, unchanged by this capability) and passes them to
`spawn_projection` as arguments.

### Requirement: Ownership Split — Application Owns the Read Model, the Constructor Owns Only the Spawn/Stop Lifecycle

The queryable read model (the application's own domain read-view, e.g. a
`UsersByTenantStore`-shaped type) is constructed and owned by the consuming
application, exactly as before this capability exists — it is application
domain logic, not framework plumbing, and this capability does not wrap,
replace, or return it. `spawn_projection`'s sole responsibility is the
background task's spawn/stop/drain lifecycle: starting the poll loop and
returning a handle that can later stop it and observe how it ended. The
dedup store, offset store, tag-discovery mechanism, handler, and poll
interval are supplied by the caller at the same call site, not constructed
internally.

#### Scenario: The call's result is a lifecycle handle, not a bundled read model

- GIVEN an application has already constructed its own queryable read model,
  dedup store, offset store, and tag-discovery closure
- WHEN it passes them all to `spawn_projection`
- THEN `spawn_projection` returns a poller handle only — the application's
  own read model reference is what it queries directly, not something
  returned or re-wrapped by the call

### Requirement: Spawn/Stop Lifecycle Convenience

`TagSchedulerImpl` MUST expose one call that, given a tag-discovery closure,
poll interval, handler, event store, dedup store, offset store, progress
reporter, and error callback, spawns the poll loop and returns a handle
covering its full stop/drain lifecycle — replacing the stop-signaling and
completion-tracking a caller previously had to hand-roll itself around
`run_until_stopped`. The poll interval MUST be an explicit, required
argument to that same call (not hardcoded, not defaulted, not configured
through a separate setter/builder step) — a caller with different interval
needs (e.g. a fast poll interval in tests) supplies its own value at the
same call site.

#### Scenario: One call yields a spawned poller with full lifecycle plumbing

- GIVEN an application has its dedup store, offset store, tag-discovery
  closure, handler, and event store already constructed
- WHEN it calls `spawn_projection` with those values and an explicit poll
  interval
- THEN the poll loop is spawned and the caller receives a single handle
  covering its stop/drain lifecycle, with no separate stop-signaling or
  completion-tracking left for the caller to hand-roll

#### Scenario: Poll interval is required, not defaulted

- GIVEN two applications with different poll-interval needs (e.g. production
  cadence vs. a fast interval for tests)
- WHEN each calls `spawn_projection`
- THEN each supplies its own interval value at the call site; neither gets a
  silently-hardcoded default it cannot override

### Requirement: Stop Consumes the Handle

The poller handle's stop operation MUST take ownership of the handle (not a
shared or exclusive reference) — once stopped, the handle cannot be reused
or stopped again, making a double-stop a compile-time error rather than a
runtime one.

#### Scenario: A stopped handle cannot be stopped again

- GIVEN a caller holds a poller handle
- WHEN it calls stop on that handle
- THEN the handle is consumed by that call, and no further operation on that
  same handle value is possible

### Requirement: Dynamic Per-Tenant Tag Discovery Preserved

`spawn_projection` MUST call the caller-supplied tag-discovery closure fresh
on each poll, rather than caching its result from the first call — preserving
CORE-018's per-tenant isolation guarantee (one tag stream per tenant) without
regression. This capability does not change what the closure discovers or how
tags are computed — only that `spawn_projection` continues to invoke it per
iteration rather than once at spawn time.

#### Scenario: A tenant's first event is picked up without reconfiguration

- GIVEN a poller handle already spawned, with no prior events for tenant `T`
- WHEN the first event for tenant `T` is written to the event store
- THEN a subsequent poll discovers and processes tenant `T`'s tag without the
  poller being respawned or explicitly told about the new tenant

### Requirement: Graceful Shutdown Preserved

Stopping the spawned poller MUST let any poll batch already in flight finish
draining before the stop call returns, and MUST surface a failed drain to
the caller as an error rather than discarding it silently.

#### Scenario: Stop waits for an in-flight batch to drain

- GIVEN the poller's poll loop is mid-batch when stop is requested
- WHEN the caller calls stop
- THEN stop does not return until that in-flight batch has finished draining

#### Scenario: A failed drain is reported, not swallowed

- GIVEN the spawned poll loop's background task terminates abnormally (panics
  or is aborted) instead of draining cleanly
- WHEN the caller calls stop
- THEN stop returns an error identifying the failure, instead of reporting
  success regardless

### Requirement: Usable By a Real Application Without Escape Hatches

`spawn_projection` MUST be sufficient for a real consuming application to
obtain the same spawned poller it would otherwise hand-wire around
`run_until_stopped`, without needing application-specific escape hatches
beyond supplying its dedup store, offset store, tag-discovery closure,
handler, event store, poll interval, progress reporter, and error callback.

#### Scenario: An application's hand-wired spawn/stop glue migrates to `spawn_projection`

- GIVEN an application previously hand-rolled its own stop-signaling and
  completion-tracking around the scheduler engine's `run_until_stopped` call
  (while still constructing its own dedup store, offset store, tag-discovery
  closure, and read model, as it does today)
- WHEN it calls `spawn_projection` instead, passing those same
  already-constructed values
- THEN it no longer hand-rolls the stop-signaling or completion-tracking
  itself, and per-tenant tag isolation continues to function unchanged; its
  dedup store, offset store, tag-discovery construction, and read model
  ownership are unaffected (see "Ownership Split" above)

### Requirement: Composition-Root Acceptance Of A Host-Constructed Durable Progress Pair Is In Scope; Framework Construction Remains Out Of Scope

A projection's durable progress pair — its `OffsetStore` and `DedupStore`
— MAY be composed at the composition root: accepted, classified by
durability, and refused there under `Profile::Production` when not
durable. This is orthogonal to, and does not reverse, CORE-026's existing
non-goal that the framework constructs or defaults these stores
internally — that non-goal remains fully in force. The composition root
never internally constructs an `OffsetStore`, `DedupStore`, or
tag-discovery mechanism on the application's behalf; it only accepts,
classifies, and validates a pair the application already built.

#### Scenario: The composition root classifies and validates without constructing

- GIVEN an application that has already constructed its own
  `OffsetStore`/`DedupStore` pair
- WHEN it registers that pair at the composition root
- THEN the composition root classifies and validates the pair's
  durability without itself constructing either store

#### Scenario: An application that registers nothing is unaffected

- GIVEN an application that never registers a durable progress pair at
  the composition root
- WHEN it composes its read-side wiring exactly as before this change
- THEN nothing about that wiring is required or performed by this
  capability, unchanged from before

#### Scenario: The refusal never reaches the scheduler engine

- GIVEN a registered pair refused under `Profile::Production`
- WHEN that refusal occurs
- THEN it occurs at the composition root, never inside
  `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()`, or the first
  poll batch

### ADDED Requirements (PROD-014B)

#### Requirement: Durable Dedup Bookkeeping Does Not Imply Exactly-Once Handler Execution

Persisting dedup bookkeeping durably MUST NOT be read, described, or documented as
exactly-once event handling anywhere in this system. This capability delivers at-least-once
handler execution with best-effort dedup bookkeeping; nothing in it prevents a handler from
running more than once for the same event under concurrent writers.

##### Scenario: Two concurrent writers may each run the handler once

- GIVEN two writers processing the same `(projection_id, tag, tenant)` concurrently, both
  checking whether an event has already been seen before either records it as seen
- WHEN both observe the event as not-yet-seen before either records it
- THEN the handler MAY run for both writers; this capability does not prevent that outcome,
  and no documentation may describe it as prevented

#### Requirement: Prevention of Double Handler Execution Rests on an Explicit, Unenforced Single-Writer Adoption Constraint

Prevention of double handler execution for the same event MUST be stated as depending on an
external, unenforced adoption constraint — **single-writer-per-`(projection_id, tag,
tenant)`** — never as a guarantee this capability itself enforces. No leader election, lock,
lease, or fencing mechanism exists in this capability to enforce that constraint across
multiple replicas of the same projection. This is the change's binding adoption constraint:
adopting this durable pair in production is conditioned on it holding.

##### Scenario: A two-replica deployment is outside the guarantee, and undetected

- GIVEN two replicas of the same projection process running concurrently against the same
  `(projection_id, tag, tenant)`
- WHEN this configuration is evaluated against this capability's guarantees
- THEN it is outside the guarantee this capability provides, and nothing in this capability
  detects or refuses that configuration

#### Requirement: The Concurrency Gap Has a Named, Distinct Follow-Up

The gap between durable dedup bookkeeping and prevention of double handler execution MUST be
recorded as a distinct, named follow-up — **PROD-014C — Atomic Read-Side Event Claiming** —
rather than folded into this capability's scope or silently left unowned.

##### Scenario: The follow-up is named, not implied

- GIVEN a reader of this capability's documentation looking for how double handler execution
  will eventually be prevented
- WHEN they look for the owning follow-up
- THEN they find it named as PROD-014C — Atomic Read-Side Event Claiming, distinct from and
  not part of this capability

### Non-Goals

- No change to `TagSchedulerImpl` or the underlying CORE-005 scheduler/store
  engine's own contract — this capability specifies only the spawn/stop
  wrapper's observable behavior built on top of it. Explicitly unchanged:
  polling semantics (how/when a poll fires), dedup semantics (what counts as
  a duplicate), offset semantics (how progress is tracked and resumed), and
  ordering guarantees (per-tag delivery order) — this capability wraps that
  engine's existing contract, it does not renegotiate any part of it.
- No new persistence format or read-model query capability beyond what
  already exists.
- No change to which type owns the queryable read model — it remains
  entirely application-owned (see "Ownership Split" above); this capability
  does not introduce a framework-owned read-model type.
- **Constructing a dedup store, an offset store, or a tag-discovery
  mechanism is out of scope.** `spawn_projection` takes these as required
  arguments; it does not provide a default or internally construct them.
  An application obtains them exactly as it does today (e.g.
  reference-app's own `ReadSideHandles::new`, itself unchanged by this
  capability) and passes them to `spawn_projection` to spawn the poller.
  A framework-level convenience that also constructs these internally
  (e.g. defaulting to in-memory implementations) was considered and
  rejected — see design.md AD-1, alternative (b) — because the handler and
  tag-discovery closure are irreducibly application-specific, and bundling
  the dedup/offset stores' construction with them would only cover half the
  boilerplate while suggesting the other half was solved too.
- No separate non-spawning "construct" step exists at this capability's
  level — `spawn_projection` always spawns immediately when called. An
  application that needs to construct its read-side wiring without a
  running async runtime (e.g. to assert on its own read model in a
  synchronous test) does so through its own pre-existing constructor (e.g.
  `ReadSideHandles::new`), which this capability does not change or replace.

## Capability: read-side-durable-progress (NEW)

Purpose: The observable durability contract for read-side progress state backed by PostgreSQL: what
survives a process restart, what identity each record has, what retention is promised, and
explicitly what concurrency guarantee is **not** offered. This capability governs the durable
`OffsetStore`/`DedupStore` pair's observable behavior only — it does not cover the underlying
interface those stores implement, atomic event claiming, or dedup retention/eviction (see
Non-Goals).

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

### Non-Goals

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
