# Delta for read-side-event-claiming

> Canonical / source of truth. Spanish review companion: `spec.es.md` (1:1
> identifiers). This is a brand-new capability — no prior
> `openspec/specs/read-side-event-claiming/spec.md` exists — but follows
> this project's convention (PROD-014A's `production-composition-hardening`
> delta) of expressing even a first-time capability as an `ADDED
> Requirements` delta, so `sdd-archive` merges it identically to any other.

Scope: PROD-014C. Defines the observable exclusion contract only: claim
identity, acquisition refusal under a live claim, lease renewal, expiry-based
takeover, stale-owner rejection via fencing, immediate release, ordering
preservation, and the Production fail-closed gate. The exact port shape
(`try_claim`/`renew`/`complete`/`release` or otherwise), storage mechanism
(claim table vs. advisory lock), and migration number are `design.md`
decisions (see Non-Goals).

## ADDED Requirements

### Requirement: Claim Identity Is `(projection_id, tag, tenant)`

A claim MUST be identified by exactly the triple `(projection_id, tag,
tenant)` — the same triple that already owns one monotonically advancing
offset. A claim on one `(projection_id, tag, tenant)` MUST NOT affect
acquisition, renewal, or release of a claim on any other `(projection_id,
tag, tenant)`, including a different tag or a different tenant of the same
`projection_id`.

#### Scenario: A claim on one tenant does not block another tenant's claim

- GIVEN two different tenants of the same `projection_id` and `tag`
- WHEN one tenant's stream is validly claimed
- THEN the other tenant's stream remains independently claimable, unaffected
  by the first claim

### Requirement: Acquisition Excludes A Concurrent Second Claimant

For one `(projection_id, tag, tenant)`, at most one worker MUST hold a valid
claim at a time. When two or more workers attempt to acquire a claim for the
same identity at the same time, exactly one MUST succeed; every other MUST be
refused. A refused worker MUST NOT call `fetch` or invoke the handler for
that stream on that tick.

#### Scenario: One of two concurrent acquirers wins, the other is refused

- GIVEN two workers polling the same `(projection_id, tag, tenant)`
- WHEN both attempt to acquire the claim at the same time
- THEN exactly one obtains it; the other is refused and does not call `fetch`
  or invoke the handler for that stream on that tick

### Requirement: A Valid Claim May Be Renewed To Extend Processing

A worker holding a valid claim MUST be able to extend its lease before it
expires, without losing the claim or interrupting an in-progress batch. While
a lease remains valid — whether original or renewed — no other worker MAY
take the stream over.

#### Scenario: Renewal during a long batch prevents takeover

- GIVEN a worker holding a valid claim on a stream, still processing a long
  batch as its lease approaches expiry
- WHEN it renews the lease
- THEN it continues holding the claim, and no other worker takes the stream
  over while the renewed lease remains valid

### Requirement: An Expired Lease Enables Takeover Without Operator Action

When a worker holding a claim stops — crashes, is killed, or pauses
indefinitely — without releasing it, the claim's lease MUST eventually
expire. Once expired, another worker MUST be able to take the stream over
without operator intervention and without waiting indefinitely, so a dead
worker cannot block a stream forever.

#### Scenario: A dead worker's claim is taken over automatically

- GIVEN a worker that acquired a claim and then stopped without releasing it
- WHEN its lease expires
- THEN another worker takes the stream over without operator intervention and
  without waiting indefinitely

### Requirement: Takeover Fences Out The Stale Owner

Ownership proof MUST accompany every claim, and every write performed under
a claim (offset write, dedup write) MUST verify that proof before applying.
A worker whose claim was taken over after its lease expired MUST have any
subsequent write it attempts as owner rejected as a stale owner, and that
rejection MUST leave the stored state unmodified — in particular it MUST NOT
rewind an offset the new owner already advanced.

#### Scenario: A stale owner's write is rejected and leaves state unmodified

- GIVEN a worker whose claim was taken over by another worker after its
  lease expired
- WHEN that first worker resumes and attempts to write offset or dedup state
  as the owner
- THEN the write is rejected as a stale owner and leaves the stored state
  unmodified, including not rewinding an offset the new owner already
  advanced

### Requirement: Normal Release Makes the Stream Immediately Reclaimable

A worker holding a valid claim that finishes its batch and releases the
claim normally MUST make the stream immediately claimable again — release
MUST NOT require waiting for the lease to expire.

#### Scenario: A released claim is claimable immediately

- GIVEN a worker holding a valid claim
- WHEN it finishes its batch and releases the claim normally
- THEN the stream becomes immediately claimable again, without waiting for
  the lease to expire

### Requirement: Claiming Preserves Existing Per-Stream Ordering

Claiming MUST NOT reorder, interleave, or skip events within a stream. While
a claim is held, events MUST still be handled in ascending version order per
`(tenant, tag)`, exactly as before this capability existed.

#### Scenario: Ordering is unchanged under an active claim

- GIVEN a stream whose claim is held by one worker
- WHEN that worker processes a batch
- THEN events are handled in ascending version order per `(tenant, tag)`,
  exactly as before this capability

### Requirement: Expiry Is Evaluated Consistently, Never Against An Individual Worker's Own Clock

Lease expiry MUST be evaluated against one consistent, deterministic time
source shared across the takeover decision — never against an individual
worker's own local wall clock read independently. This prevents clock skew
between replicas from causing premature or delayed takeover.

#### Scenario: Expiry does not depend on which worker's clock is asked

- GIVEN two workers with independently drifting local clocks
- WHEN a lease-expiry decision is made
- THEN the decision is consistent regardless of which worker observes it —
  it does not vary because one worker's local clock reads differently

### Requirement: `Profile::Production` Fails Closed Without A Durable Claim Mechanism

A composition declaring `Profile::Production` that registers read-side
progress but no durable claim mechanism MUST be refused at
composition/bootstrap time — never deferred to the first poll or the first
batch — with an error naming the missing capability and the exact call that
fixes it. A composition declaring `Profile::Production` that registers a
durable claim mechanism MUST succeed, and multi-replica read-side becomes
supported under the stated operational constraint (handler effects remain
at-least-once — see the boundary requirement below).

#### Scenario: Missing claim mechanism is refused at bootstrap, not first poll

- GIVEN a composition declaring `Profile::Production` that registers read-side
  progress but no durable claim mechanism
- WHEN `build()` is called
- THEN it is refused at composition/bootstrap time, with an error naming the
  missing capability and the exact call that fixes it

#### Scenario: A registered durable claim mechanism allows the build to succeed

- GIVEN a composition declaring `Profile::Production` that registers a
  durable claim mechanism
- WHEN `build()` is called
- THEN it succeeds, and multi-replica read-side becomes supported under the
  stated operational constraint

### Requirement: This Capability Bounds Handler-Execution Count, Never External Side-Effect Count

Atomic claiming bounds how many times the framework invokes the handler for
one claim holder's batch; it MUST NOT be described, documented, or read as a
guarantee about an external effect a handler performs. A single worker
holding a valid claim for a whole batch that crashes after the handler
succeeds but before the batch is fully recorded MAY have the handler run
again for those events on resume — this capability does NOT prevent that,
and no delivered artifact may describe this capability as exactly-once
processing or exactly-once external effects.

#### Scenario: A crash after handler success still allows a re-run on resume

- GIVEN a single worker holding a valid claim for the whole batch
- WHEN it crashes after the handler succeeds but before the batch is fully
  recorded
- THEN the handler MAY run again for those events on resume; this is not
  prevented, and no delivered artifact may describe it as exactly-once
  processing or exactly-once external effects

## Non-Goals

- Distributed consensus, global leader election, a distributed transaction
  coordinator, a Kafka consumer-group replacement, or an `EventStore`
  redesign.
- Exactly-once **external** side effects of any kind — `Handler<E>` permits
  arbitrary I/O; claiming bounds handler-execution count only. Avoiding a
  duplicated external effect requires the handler's own effect boundary to
  carry a fence or be idempotent on its own.
- Retry/backoff for `Transient` errors — an adjacent, independently shippable
  concern.
- Cross-table atomicity between dedup and offset writes — a pre-existing
  condition this capability neither creates nor closes.
- Intra-process cross-tag concurrency — `TagSchedulerImpl` remains sequential
  within one process; this capability makes cross-*process* concurrency
  exclusion-safe only.
- Any backend other than PostgreSQL.
- Prescribing the exact port method set, the storage mechanism (claim table,
  row lock, or advisory lock), or the migration number — `design.md`
  decisions.
