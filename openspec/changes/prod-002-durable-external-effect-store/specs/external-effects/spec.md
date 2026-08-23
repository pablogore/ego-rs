# Delta for external-effects

> Base capability spec: `openspec/specs/external-effects/spec.md` (CORE-019,
> shipped/archived). This delta adds durable-implementation requirements for
> the two existing ports (`EffectStateStore`, `EffectDedupStore`) and modifies
> two existing requirements plus the Non-Goals section. It introduces no new
> public trait, no change to `ExternalEffectDescription`, the executor
> contract, the pipeline shape, or the state model.

Scope: PROD-002. Durability is a mandatory provider contract; multi-node
safety is a provider-*declared* capability, not universally mandated.
PostgreSQL and Stoolap are the two day-zero providers, each satisfying both
ports independently.

## ADDED Requirements

### Requirement: Durability Is Mandatory; Multi-Node Safety Is a Declared Capability

Any `EffectStateStore`/`EffectDedupStore` implementation presented as durable
MUST make an accepted effect survive process death and MUST expose atomic
state transitions (no torn read between `Pending`/`InFlight`/terminal
states). Multi-node safety (safe concurrent claiming across independent
processes/hosts) is NOT required of every durable implementation — a
provider MUST expose a queryable capability declaration stating whether it is
`Durable`, `ConcurrentLocalSafe` (safe among concurrent claimers within a
single process or host ownership domain — not across independent
processes/hosts; deliberately not named "process-safe", which would wrongly
suggest cross-process safety), `MultiNodeSafe`, and `SupportsLeases`; the
exact declaration mechanism is a design decision, not fixed here.

#### Scenario: An accepted effect survives a real process restart
- GIVEN an effect accepted into a durable store
- WHEN the process is killed and restarted
- THEN the effect is still recorded with its last-known state, never lost

#### Scenario: A provider declares its capabilities honestly
- GIVEN a durable provider implementation
- WHEN its capability declaration is queried
- THEN it reports `Durable`, `ConcurrentLocalSafe`, `MultiNodeSafe`, and
  `SupportsLeases` accurately, and a caller can distinguish a `MultiNodeSafe`
  provider from one that is not without inspecting its internals

#### Scenario: A mixed durable/non-durable registration is not silently treated as durable
- GIVEN an `EffectStateStore` implementation that is `Durable` and an
  `EffectDedupStore` implementation that is not
- WHEN both capability declarations are inspected
- THEN the overall configuration is not durable — dedup/idempotency state is
  lost on crash even though effect state persists, and this MUST be
  observable from the two independent capability declarations, not hidden
  behind a single combined flag

### Requirement: Day-Zero Durable Providers — PostgreSQL and Stoolap

This capability MUST ship two durable providers from day zero, each
satisfying `EffectStateStore` and `EffectDedupStore` independently, per their
declared capabilities:

| Provider | Durable | ConcurrentLocalSafe | MultiNodeSafe |
|---|---|---|---|
| PostgreSQL | Yes | Yes | Yes |
| Stoolap (embedded) | Yes | Yes (local) | No |

A third-party provider satisfying the same two ports MUST be possible; this
is not a closed list.

#### Scenario: Both day-zero providers pass the same durability criteria
- GIVEN the PostgreSQL and Stoolap implementations
- WHEN each is exercised against restart, retry, and dedup scenarios
- THEN both satisfy the same behavioral criteria, proving the port contract
  rather than one backend's incidental behavior

#### Scenario: Stoolap declares no multi-node safety
- GIVEN the Stoolap provider
- WHEN its capability declaration is inspected
- THEN it reports `MultiNodeSafe: No`, and no cross-node coordination is
  claimed or silently assumed by the capability

### Requirement: Claim Ownership Is Exclusive While Leased, Not a Double-Dispatch Guarantee

A provider declaring `MultiNodeSafe` MUST guarantee exclusive ownership of a
claim for as long as its lease remains valid — no other claimer may claim or
transition that effect while a valid lease is held. Once a lease expires, the
effect becomes eligible for redispatch by another claimer; because the
original claimer's execution may still be in progress at that moment,
duplicate external execution remains possible and is NOT prevented by claim
exclusivity alone — it is covered by the at-least-once delivery + mandatory
idempotency-key contract (see Delivery Guarantee requirement below). "Never
both dispatch" MUST NOT be claimed as an unconditional guarantee.
`ConcurrentLocalSafe` MUST guarantee that concurrent claimers operating
within the provider's local ownership domain (a single process or host)
cannot simultaneously acquire the same effect. This guarantee does NOT imply
or require leases. A provider that additionally declares `SupportsLeases`
further guarantees that ownership specifically remains exclusive only for as
long as a claim's lease is valid (see above for what happens after lease
expiry). A `ConcurrentLocalSafe`-only provider without lease support (e.g. an
embedded single-process store such as Stoolap, which declares
`ConcurrentLocalSafe: true` and `SupportsLeases: false`) guarantees
unconditional local exclusivity instead — there is no lease to expire. In no
case may `ConcurrentLocalSafe` be relied upon for cross-node exclusivity.

#### Scenario: Two claimers never hold overlapping valid claims against a MultiNodeSafe provider
- GIVEN two independent processes claiming due effects from the same
  `MultiNodeSafe` store
- WHEN both attempt to claim overlapping effects at the same time
- THEN each effect is claimed by exactly one claimer for as long as that
  claimer's lease remains valid

#### Scenario: Lease expiry allows redispatch, and duplicate execution is covered by idempotency, not claim exclusivity
- GIVEN a claim whose lease has expired while the original claimer's
  execution is still in progress
- WHEN a second claimer claims and dispatches the same due effect
- THEN both claimers may execute against the destination, and the
  destination's idempotency-key handling — not claim exclusivity — is what
  prevents a duplicate logical outcome

#### Scenario: A non-MultiNodeSafe provider is not used for cross-node claiming
- GIVEN a store declaring `MultiNodeSafe: No`
- WHEN it is queried for its capability
- THEN callers can determine that concurrent claiming is safe only within one
  process, never across independent nodes

#### Scenario: A ConcurrentLocalSafe-only provider without leases guarantees unconditional local exclusivity
- GIVEN a provider declaring `ConcurrentLocalSafe: true` and
  `SupportsLeases: false` (e.g. Stoolap)
- WHEN two concurrent claimers within that provider's local ownership domain
  attempt to claim the same effect
- THEN exactly one claimer acquires it, unconditionally, with no lease
  involved and no expiry after which the other could still be in progress

### Requirement: Retry Bookkeeping Persists Across Restart

Attempt count, next-due time, and per-`effect_type` backoff overrides MUST
survive a restart against a durable store; backoff MUST resume from where it
left off, never reset.

#### Scenario: Backoff resumes, not resets, after a restart
- GIVEN an effect with two prior retryable attempts recorded
- WHEN the process restarts and the effect becomes due again
- THEN the third attempt honors the backoff schedule as if no restart
  occurred

### Requirement: Dedup Identity Persists Durably

Scoped `(tenant, effect_type, key)` dedup identity and `DedupOutcome`
semantics (unchanged from the base spec) MUST persist across a restart.

#### Scenario: A replayed key is still deduplicated after a restart
- GIVEN a scoped key reserved before a crash
- WHEN the same scoped key is replayed after restart
- THEN it is recognized as already-owned/succeeded per `DedupOutcome`, not
  treated as fresh

### Requirement: Cleanup and Retention of Terminal Rows

A durable implementation MUST provide a way to remove `succeeded` and
`terminal-failed` rows once they are no longer needed for dedup or audit;
exact policy (TTL, count, operator-triggered) is a design decision.

#### Scenario: Terminal rows do not grow unbounded
- GIVEN a store accumulating succeeded and terminal-failed rows over time
- WHEN cleanup/retention is applied
- THEN rows past the retention policy are removed without affecting
  in-flight or pending effects

### Requirement: Fault-Injection TestKit Double for the Delivery Ports

TestKit MUST provide a real trait implementation (not a mock) of
`EffectStateStore`/`EffectDedupStore` capable of injecting transient store
errors, simulated crash points, and claim races, so retry, recovery, and
idempotency behavior is testable without a running external provider.

#### Scenario: Retry is exercised without a real durable backend
- GIVEN the TestKit fault-injection double configured to fail transiently
- WHEN an effect is accepted and dispatched
- THEN the retry path is exercised identically to a real durable store's
  transient failure

## MODIFIED Requirements

### Requirement: Delivery State Is Reconstructable After a Restart

An `EffectStateStore` implementation MUST be able to list the effects whose
retry time has elapsed so they can be (re-)dispatched, and MUST be able to
signal which effects were mid-delivery when the process stopped so they are
treated as not-yet-confirmed and become eligible for redispatch. These
affordances are what makes crash recovery possible for a durable store; the
shipped in-memory store exposes them but still loses all state on a crash
(per the Delivery Guarantee requirement below). For a durable store, this
recovery MUST hold across a real process boundary, and a stale claim (held
by a process that crashed before completing) MUST become recoverable rather
than permanently stuck.
(Previously: recovery affordances were contractual but untested against a
real durability/restart boundary, and said nothing about stale-claim
recovery.)

#### Scenario: Due effects can be listed for redispatch

- GIVEN effects recorded in an `EffectStateStore`, some with an elapsed retry
  time
- WHEN the delivery subsystem asks the store for the effects due at the
  current time
- THEN it receives those whose retry time has elapsed, each carrying enough
  data (tenant and description) to be re-executed

#### Scenario: Mid-delivery effects are recoverable after a restart

- GIVEN effects that were mid-delivery when the process stopped
- WHEN the store is asked to recover after the restart
- THEN those effects are signalled as not-yet-confirmed and become eligible
  for redispatch, never silently treated as delivered

#### Scenario: A stale claim is recoverable, not permanently stuck
- GIVEN a durable store holding a claim taken by a process that crashed
  before marking the effect succeeded or terminal
- WHEN recovery runs after restart (single-process sweep, or lease
  expiry where the provider supports leases)
- THEN the claim is released and the effect becomes eligible for redispatch

### Requirement: Delivery Guarantee Is At-Least-Once, Never Exactly-Once

The capability MUST guarantee at-least-once attempted delivery within the
lifetime/durability of the registered `EffectStateStore`, plus mandatory
idempotency-key propagation to executors. With a cooperating destination
this composes to a logical once-only outcome; "exactly once" MUST NOT appear
in the public contract or its docs. With the shipped in-memory store, the
guarantee MUST be documented as degrading to at-most-once across a crash.
With a durable store satisfying the Durability requirement above, the
guarantee MUST hold as durable at-least-once from acceptance onward — the
post-commit dual-write gap (crash between event commit and effect
acceptance) is narrowed by a durable store, not closed; "exactly once" still
MUST NOT appear in the public contract for any provider, durable or not.
(Previously: only the in-memory store's at-most-once-across-crash
degradation was documented; no durable-store behavior was specified.)

#### Scenario: Logical once-only outcome with a cooperating destination

- GIVEN a destination that rejects a duplicate idempotency key
- WHEN the same effect is attempted more than once due to a retry
- THEN the destination's dedup plus the propagated key yields one logical
  outcome

#### Scenario: In-memory store loses undelivered effects on crash

- GIVEN the shipped in-memory `EffectStateStore`
- WHEN the process crashes before a pending or in-flight effect completes
- THEN that effect is lost, documented explicitly, never hidden

#### Scenario: A durable store's accepted effect survives a crash, never claiming exactly-once
- GIVEN an effect accepted into a durable store satisfying the Durability
  requirement
- WHEN the process crashes at any point after acceptance
- THEN the effect remains owed and is redispatched until terminal, and no
  documentation or signal implies exactly-once delivery

## MODIFIED Non-Goals

The base spec's Non-Goals section currently includes:

> - Durable delivery store implementation (Postgres outbox) — the ports are
>   shaped to enable one, but none ships in this capability.
> - CDC, Debezium, distributed scheduling, cluster coordination, sharding, or
>   cross-node leasing.

These two entries MUST be replaced with:

- Workflow engine, saga/temporal-style durable execution, distributed
  transaction manager, or transactional enlistment of effect acceptance
  inside the event commit — unchanged non-goals, carried forward verbatim.
- CDC, Debezium, distributed scheduling, sharding, Raft, leader election,
  service discovery, or cluster membership implemented inside this
  capability — unchanged. Cross-node claim coordination for a
  `MultiNodeSafe` provider (e.g. PostgreSQL's transactional locking) is now
  in scope as part of that provider's declared capability; what remains a
  non-goal is Ego building a generic distributed-coordination layer (e.g. an
  OpenRaft-based consensus/claims mechanism) for providers that do not
  declare `MultiNodeSafe` (e.g. Stoolap) — composing such a layer around a
  non-multi-node-safe provider is a host application's architecture to
  build, not this capability's.

The durable-store-implementation non-goal is retired outright: PROD-002 is
that shipment (PostgreSQL and Stoolap), and "none ships in this capability"
is no longer true.
