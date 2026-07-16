# Delta for external-effects

**Capability choice**: `external-effects` is a new capability directory —
`openspec/specs/` currently has no `external-effects` entry (confirmed
against `domain, http-transport, persistent-entity, reference-service,
security-jwt, security-sdk, service-sdk, testkit`), and the proposal itself
names this the new canonical spec target (§5, §16). Reliable delivery,
retry, dedup, executor registry, and lifecycle integration are a cohesive
concern distinct from `domain` (which only describes effects, unchanged
here) and from `persistent-entity`/`service-sdk` (whose existing specs cover
activation authority and context propagation — unrelated topics). The
integration points those two crates gain are expressed below as
requirements of this new capability rather than as edits to their existing,
unrelated specs.

## ADDED Requirements

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

#### Scenario: Logical once-only outcome with a cooperating destination

- GIVEN a destination that rejects a duplicate idempotency key
- WHEN the same effect is attempted more than once due to a retry
- THEN the destination's dedup plus the propagated key yields one logical
  outcome

#### Scenario: In-memory store loses undelivered effects on crash

- GIVEN the shipped in-memory `EffectStateStore`
- WHEN the process crashes before a pending or in-flight effect completes
- THEN that effect is lost, asserted by an explicit test, never hidden

### Requirement: Delivery State Is Reconstructable After a Restart

An `EffectStateStore` implementation MUST be able to list the effects whose
retry time has elapsed so they can be (re-)dispatched, and MUST be able to
signal which effects were mid-delivery when the process stopped so they are
treated as not-yet-confirmed and become eligible for redispatch. These
affordances are what makes crash recovery possible for a durable store; the
shipped in-memory store exposes them but still loses all state on a crash (per
the at-least-once requirement above).

#### Scenario: Due effects can be listed for redispatch

- GIVEN effects recorded in an `EffectStateStore`, some with an elapsed retry
  time
- WHEN the delivery subsystem asks the store for the effects due at the current
  time
- THEN it receives those whose retry time has elapsed, each carrying enough
  data (tenant and description) to be re-executed

#### Scenario: Mid-delivery effects are recoverable after a restart

- GIVEN effects that were mid-delivery when the process stopped
- WHEN the store is asked to recover after the restart
- THEN those effects are signalled as not-yet-confirmed and become eligible for
  redispatch, never silently treated as delivered

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
