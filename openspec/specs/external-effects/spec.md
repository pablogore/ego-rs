# Spec: CORE-019 Reliable External Effects

## Capability: external-effects (new)

Purpose: a write-side effect delivery subsystem that turns
`Effect::ExternalEffectDescription` from a documented-but-undelivered contract
into an honestly-labeled, at-least-once-attempted, dedup-aware delivery
pipeline — without coupling the domain to HTTP, brokers, databases, or e-mail.

### Requirement: Post-Commit Effect Acceptance

The runtime MUST accept a command's produced external effects only after the
atomic commit for that command's events has succeeded, and before the
command's *successful* reply is returned to the caller. Acceptance MUST NOT be
refused outright at intake — there is no synchronous "your effect list is
invalid, rejected" path — and a saturated acceptance path MUST delay the
reply, never the commit, which has already completed. Recording an accepted
effect MAY nonetheless ultimately fail: a transient `EffectStateStore` error
MUST be retried under a bounded, configurable retry policy, and only if that
policy is exhausted (or the store error is non-retryable) does the caller
receive an explicit post-commit acceptance error. Such a failure MUST NOT roll
back the already-committed event; it means the command succeeded but at least
one described effect could not be durably-enough registered and may be lost to
the post-commit dual-write gap.

#### Scenario: Effects accepted only after commit succeeds

- GIVEN a command whose handler describes one or more external effects
- WHEN the atomic commit of that command's events succeeds
- THEN the effects are accepted into the delivery subsystem after commit,
  before the command's reply is sent

#### Scenario: Acceptance backpressure delays the reply, not the commit

- GIVEN the acceptance path is at capacity
- WHEN an already-committed command attempts to hand off its effects
- THEN the commit remains complete and unaffected; only the reply is delayed
  until acceptance succeeds

#### Scenario: Transient acceptance failure is retried, then surfaced explicitly

- GIVEN an already-committed command whose effects are being accepted
- AND the `EffectStateStore` returns a transient error on every attempt
- WHEN the bounded acceptance retry policy is exhausted
- THEN the caller receives an explicit post-commit acceptance error, the
  committed event is NOT rolled back, and the error is not read as the command
  having failed

### Requirement: Runtime-Minted Effect Identity

The runtime MUST mint a unique effect identifier for every accepted effect at
acceptance time, immediately after the triggering commit succeeds and before
the effect enters dedup/state bookkeeping. `ExternalEffectDescription` MUST
stay unchanged; the identifier lives only in runtime-owned delivery metadata.

#### Scenario: Every accepted effect receives a runtime identity

- GIVEN a handler describes an external effect with no identifier field
- WHEN the effect is accepted after commit
- THEN the runtime assigns a unique effect identifier before any dedup or
  state-store interaction occurs

### Requirement: EffectStateStore and EffectDedupStore Are the Public Delivery Ports

Delivery-state ownership MUST be exposed as exactly two public ports:
`EffectStateStore` (pending → in-flight → succeeded | retryable-failed |
terminal-failed state and retry bookkeeping) and `EffectDedupStore` (scoped
idempotency dedup). Exactly these two delivery-state operations are exposed as
independently-swappable extension points; the in-process admission/ordering
mechanism is not part of the public contract and MAY vary between
implementations without being a compatibility concern. One type MAY implement
both public ports for the shipped in-memory implementation; a future durable
implementation MUST be able to satisfy each port independently.

#### Scenario: Two ports, no third swappable queue port

- GIVEN the public extension surface of this capability
- WHEN it is enumerated
- THEN exactly `EffectStateStore`, `EffectDedupStore`, and the executor
  registry are swappable contracts; no public trait exists for the admission
  queue

#### Scenario: One composite MAY satisfy both ports for slice 1

- GIVEN the shipped in-memory delivery store
- WHEN it is inspected
- THEN one struct implements both ports, and each port's contract remains
  independently satisfiable by a future separate implementation

### Requirement: ExternalEffectExecutor Registry — One Owner Per Type

`ExternalEffectExecutor` MUST be registered explicitly per `effect_type`,
transport-agnostically. Registering a second executor for an `effect_type`
that already has one registered MUST fail at registration time — no
last-wins, first-wins, or multicast. One executor instance MAY be registered
for more than one `effect_type`.

#### Scenario: Duplicate registration for the same effect_type fails

- GIVEN an executor is already registered for `effect_type = "invoice.created"`
- WHEN a second executor is registered for the same `effect_type`
- THEN registration fails immediately; the first registration remains the
  sole owner

#### Scenario: One executor MAY own multiple effect types

- GIVEN one executor instance capable of handling several effect types
- WHEN it is registered under two distinct `effect_type` keys
- THEN both registrations succeed and each dispatches to that instance

### Requirement: Delivery Guarantee Is At-Least-Once, Never Exactly-Once

The capability MUST guarantee at-least-once attempted delivery within the
lifetime/durability of the registered `EffectStateStore`, plus mandatory
idempotency-key propagation to executors. With a cooperating destination
this composes to a logical once-only outcome; "exactly once" MUST NOT appear
in the public contract or its docs. With the shipped in-memory store, the
guarantee MUST be documented as degrading to at-most-once across a crash.
With a durable store satisfying the Durability requirement, the guarantee
MUST hold as durable at-least-once from acceptance onward — the post-commit
dual-write gap (crash between event commit and effect acceptance) is narrowed
by a durable store, not closed; "exactly once" still MUST NOT appear in the
public contract for any provider, durable or not.

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

### Requirement: Delivery State Is Reconstructable After a Restart

An `EffectStateStore` implementation MUST be able to list the effects whose
retry time has elapsed so they can be (re-)dispatched, and MUST be able to
signal which effects were mid-delivery when the process stopped so they are
treated as not-yet-confirmed and become eligible for redispatch. These
affordances are what makes crash recovery possible for a durable store; the
shipped in-memory store exposes them but still loses all state on a crash
(per the Delivery Guarantee requirement above). For a durable store, this
recovery MUST hold across a real process boundary, and a stale claim (held
by a process that crashed before completing) MUST become recoverable rather
than permanently stuck.

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

### Requirement: Executor Classifies Protocol Errors; Runtime Classifies the Rest

Attempt outcomes MUST include `Success`, `RetryableFailure`, and
`TerminalFailure` from the executor, extended by runtime-derived outcomes:

| Outcome | Source | Effect |
|---|---|---|
| `Timeout` | runtime, per-attempt deadline | retryable |
| `Cancelled` | runtime, shutdown | remains pending |
| `ExecutorMissing` | runtime, no registration | terminal, loud signal |
| `InvalidEffect` | runtime, key/payload/destination conflict | terminal |
| executor panic | runtime, task isolation | one retryable attempt, same cap |

The runtime MUST compute backoff (exponential with jitter, an attempt cap,
per-`effect_type` override) for every retryable outcome; exact default
numeric values are not fixed here. Terminal effects MUST remain queryable in
the store as `terminal-failed`.

#### Scenario: Effect with no registered executor fails closed

- GIVEN an accepted effect whose `effect_type` has no registered executor
- WHEN the delivery runner attempts to dispatch it
- THEN the outcome is `ExecutorMissing`, the effect is marked
  terminal-failed, and a loud signal is emitted

#### Scenario: Executor panic is treated as one retryable attempt

- GIVEN an executor panics while attempting one effect
- WHEN the panic is caught by the runtime's task isolation
- THEN it counts as one retryable attempt against the same attempt cap as
  any other retryable failure

### Requirement: ImmediateDeliveryPolicy Is a Configuration, Not a Second Pipeline

There MUST be exactly one execution pipeline (accept → queue → delivery
runner → executor), and immediate delivery MUST be observable as that same
pipeline: an effect accepted under `ImmediateDeliveryPolicy` MUST traverse the
identical accept → queue → run → execute path that any other configuration
does, with no bypass path in existence. The profile behaves, from an
operator's or caller's vantage, as one tuned for minimal backlog and no
retries. A caller MUST NOT be able to observe a second, bypassing execution
model or a distinct no-op store standing in for the pipeline.

#### Scenario: Immediate delivery still passes through the one pipeline

- GIVEN a runtime configured with `ImmediateDeliveryPolicy`
- WHEN an effect is accepted
- THEN it still flows through accept → queue → delivery runner → executor; a
  failed attempt under this profile is signaled, not retried

### Requirement: Runtime Lifecycle Integration

Startup MUST construct/start the delivery runner and store with zero
runner/queue cost when no executor is registered. The admission queue MUST
be bounded; when full, acceptance MUST block/wait for capacity — it MUST NOT
drop an effect or refuse acceptance of an already-committed effect. Shutdown
MUST stop accepting new work, drain pending effects until a configurable
deadline, transition in-flight attempts to `Cancelled` → pending, and emit an
explicit `drain_incomplete` signal for any remainder.

#### Scenario: Zero cost when the capability is unused

- GIVEN a runtime with no executor registered and no effects ever produced
- WHEN the runtime runs under load
- THEN no delivery-runner work or queue overhead is observable

#### Scenario: Shutdown drains within deadline or signals incompleteness

- GIVEN pending effects exist at shutdown
- WHEN the drain deadline is reached with effects still pending
- THEN in-flight attempts become `Cancelled` (remaining pending) and an
  explicit `drain_incomplete` signal is emitted, never silently

### Requirement: Runtime Remains Transport-Agnostic; Destination and Payload Are Opaque

The runtime MUST NOT branch on `effect_type` or `destination` values to
select behavior; all type-specific logic MUST live behind a registered
`ExternalEffectExecutor`. The runtime MUST treat `destination: String` and
`payload: Vec<u8>` as opaque — never parsed, interpreted, deserialized, or
inspected.

#### Scenario: No effect_type/destination match exists in runtime code

- GIVEN the runtime and service-sdk crates implementing this capability
- WHEN their source is inspected for branching on `effect_type` or
  `destination` literals
- THEN none exists outside the executor registry lookup itself

#### Scenario: Payload passes through unexamined

- GIVEN an effect with an opaque `payload: Vec<u8>`
- WHEN the runtime routes it to the registered executor
- THEN the bytes are forwarded unmodified and never deserialized or logged
  by the runtime

### Requirement: Observability Signals

The capability MUST emit signals for: accepted, dispatch started, attempt,
success, retry scheduled, terminal failure, deduplicated, executor missing,
per-attempt latency, queue depth, age of oldest pending effect, and drain
incomplete. Signals MUST carry the runtime effect identifier, `effect_type`,
destination, tenant (when scoped), and a redacted/hashed idempotency key.
`payload` MUST NOT be logged or exported by any signal by default.

#### Scenario: Payload never appears in a default signal

- GIVEN default observability configuration
- WHEN any listed signal is emitted for an effect carrying a payload
- THEN the payload bytes are absent from the emitted signal

### Requirement: Tenant Isolation

The runtime MUST attach the tenant established from the entity identity at
acceptance time; effects MUST NOT carry caller-supplied tenant hints as an
authoritative source. Dedup identity MUST be scoped `(tenant, effect_type,
key)`. An executor MUST receive the established tenant as a fact it cannot
substitute or mint.

#### Scenario: Cross-tenant dedup collision is impossible

- GIVEN two tenants each producing an effect with the identical `effect_type`
  and idempotency key string
- WHEN both are accepted
- THEN each is tracked under its own tenant-scoped dedup identity; neither is
  treated as a duplicate of the other

#### Scenario: Reused key with a different payload or destination is rejected

- GIVEN a scoped key already recorded for one payload/destination pair
- WHEN the same scoped key is reused with a different payload or destination
- THEN the effect is rejected as `InvalidEffect` (terminal, signaled), never
  silently deduplicated

### Requirement: Backward Compatibility

Existing `PersistentEntity` implementations that never describe external
effects MUST continue to compile and behave unchanged. Describing effects
MUST be additive and opt-in. An application producing no external effects
MUST incur no measurable runtime overhead from this capability. An effect
produced for an `effect_type` with no registered executor MUST fail closed
(terminal failure and signal) — never a silent drop.

#### Scenario: Unmodified handler compiles and runs unchanged

- GIVEN an existing handler that never returns `Effect::ExternalEffects`
- WHEN the workspace is rebuilt after this capability ships
- THEN the handler compiles and its existing tests pass without modification

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

### Non-Goals

- Official adapters for HTTP, Kafka, NATS, Iggy, SMTP, S3, or any concrete
  external system.
- Workflow engine, saga/temporal-style durable execution, distributed
  transaction manager, or transactional enlistment of effect acceptance
  inside the event commit.
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
- Physical exactly-once delivery against arbitrary systems.
- Read-side external providers (CORE-019A — related, sequenced after, no
  technical dependency, not designed here).
- Circuit breaker (deferred with a per-effect-type policy extension point).
