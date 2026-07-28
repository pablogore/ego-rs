# Delta for read-side

The canonical `read-side` capability (`openspec/specs/read-side/spec.md`) today
specifies CORE-026 spawn/stop lifecycle behavior only — it carries no
observability requirement. This delta ADDS production observability: a
target-checkpoint query, a per-model lag definition, bounded-cardinality
operational metrics, and a read-side health-state mapping. It changes none of
the existing lifecycle requirements.

## ADDED Requirements

### Requirement: Read-Side Store Exposes a Target Checkpoint Query

The read-side store SPI MUST expose a target-checkpoint query returning the head
version — the highest available `event_version` — for a `(tenant, tag)` pair, so
that lag can be computed as the difference between that target and the processed
position. The read-side store MUST NOT be assumed to already provide this: today
the only query is an offset-paginated forward fetch, and no head/latest/target
query exists, so lag is not computable until this query is added. The query MUST
fail closed on an empty tenant — an empty tenant MUST return "no head" rather
than another tenant's head. A stream with no events MUST return "no head" rather
than an error or a fabricated version.

#### Scenario: Head version is returned for a populated stream
- GIVEN a `(tenant, tag)` whose read-side stream contains events up to
  `event_version` N
- WHEN the target-checkpoint query is called for that `(tenant, tag)`
- THEN it returns N as the head version

#### Scenario: An empty stream reports no head, not an error
- GIVEN a `(tenant, tag)` for which no events have been written
- WHEN the target-checkpoint query is called
- THEN it returns "no head" (absent), and NOT an error and NOT a fabricated
  version

#### Scenario: An empty tenant fails closed
- GIVEN an empty tenant string
- WHEN the target-checkpoint query is called
- THEN it returns "no head", never surfacing any other tenant's head version

#### Scenario: The forward-fetch contract is unchanged
- GIVEN an existing read-side store consumer that only calls the offset-paginated
  fetch
- WHEN the target-checkpoint query is added to the SPI
- THEN the fetch contract (offset pagination, tenant isolation, ascending order)
  is unchanged and the consumer's fetch behavior is unaffected

### Requirement: Projection Lag Is Defined Per Read-Side Model

Lag MUST be defined as `max(0, target − processed)` in events, and MUST be
defined SEPARATELY for each read-side model because their position semantics
differ. For the polling projection, processed is the last committed
`Offset::Sequence` (read via the offset store) and target is the head version
(from the target-checkpoint query); lag is the version delta. For the reactive
scheduler, processed is the last consumed bus sequence per entity stream and
target is the highest observed bus sequence; lag is the sequence delta, and any
outstanding gap range counts as undelivered events within its span. A lag of
zero MUST mean caught up. Lag MUST NOT be reported as a single cross-model number
that conflates the two position spaces.

#### Scenario: Polling projection lag is the version delta
- GIVEN a polling projection whose processed offset is `Sequence(P)` and whose
  head version is `H`, with `H >= P`
- WHEN its lag is computed
- THEN the lag is `H − P` events, and is `0` exactly when `P == H`

#### Scenario: Reactive scheduler lag is the sequence delta
- GIVEN a reactive entity stream whose last consumed sequence is `C` and whose
  highest observed bus sequence is `S`, with `S >= C`
- WHEN its lag is computed
- THEN the lag is `S − C` events, and any outstanding gap range is counted as
  undelivered within that span

#### Scenario: A momentarily stale target never yields negative lag
- GIVEN a processed position that momentarily exceeds a stale head read
- WHEN lag is computed
- THEN the reported lag is clamped to `0`, never negative

#### Scenario: The two models are not merged into one lag number
- GIVEN both a polling projection and a reactive scheduler running
- WHEN their lag is reported
- THEN each is reported under its own model, not summed or averaged into a single
  cross-model figure that mixes version and bus-sequence positions

### Requirement: Time Since Last Progress Complements Lag

The read-side MUST expose time-since-last-progress — the wall-clock duration
since the last successful batch commit / delivery — as a complement to lag, for
both models. This is REQUIRED because lag alone cannot distinguish a caught-up
idle projection (lag zero, no recent progress, healthy) from a projection that is
behind and no longer advancing. A projection whose lag is zero MUST NOT be
treated as stalled regardless of how long it has been idle. The progress
timestamp MUST NOT be labeled by tenant, tag, or entity.

#### Scenario: A caught-up idle projection is not a stall
- GIVEN a projection with lag `0` that has completed no batch for longer than the
  stall deadline because there is nothing new to process
- WHEN its progress signals are evaluated
- THEN it is not considered stalled — zero lag overrides an old progress
  timestamp

#### Scenario: A behind projection that stops advancing is observable as stalled
- GIVEN a projection with lag greater than zero whose last successful batch
  completed longer ago than the stall deadline
- WHEN its progress signals are evaluated
- THEN both lag greater than zero AND time-since-last-progress past the deadline
  are observable, jointly identifying a stall

### Requirement: Operational Metrics Are Emitted Through a Port With Bounded Labels

Read-side operational metrics — throughput, retries, errors, lag, and
time-since-last-progress — MUST be emitted through the domain observability port,
NOT directly from the domain to any telemetry backend; the domain MUST remain
free of OpenTelemetry and other infrastructure types, with the mapping performed
by an adapter outside the domain. Metric labels MUST be drawn ONLY from a closed,
enumerable set: `projection_id`, `read_side_model` (`polling` or `reactive`), and
a bounded `outcome`/result class. Tenant, tag, entity identifiers, and raw
sequence/offset values MUST NOT be used as labels; where a dimension is unbounded
(e.g. per-tenant or per-entity), the metric MUST be aggregated over it before
emission. Error messages MUST NOT be promoted to labels.

#### Scenario: Throughput, retries, and errors are counted with bounded labels
- GIVEN a running projection emitting batch completions, retries, and errors
- WHEN the corresponding metrics are emitted
- THEN each carries labels only from `{projection_id, read_side_model, outcome}`,
  and none carries a tenant, tag, entity, or sequence value

#### Scenario: Per-tenant lag is aggregated, never labeled per tenant
- GIVEN a projection processing many tenants under per-tenant tag streams
- WHEN read-side lag is emitted
- THEN it is aggregated across tenants (e.g. the maximum) under a bounded
  `projection_id` label, and NOT emitted as one time series per tenant

#### Scenario: The domain emits no telemetry types
- GIVEN the read-side domain modules
- WHEN their dependencies are inspected
- THEN they reference the observability port only, with no OpenTelemetry or other
  telemetry-backend type — the adapter that maps to a backend lives outside the
  domain

#### Scenario: The reactive scheduler's signals flow through the port, not tracing only
- GIVEN the reactive scheduler which today emits only tracing logs for consumed
  events and gaps
- WHEN this observability is in effect
- THEN its throughput, lag, and error signals are also emitted through the
  observability port, not tracing exclusively

### Requirement: Read-Side Health State Has Observable Thresholds

Read-side health MUST map lag and time-since-last-progress to a health state
using observable thresholds — a configured lag threshold (in events) and a
configured stall deadline (a duration) — not vague qualifiers. A projection whose
lag is within the threshold and whose progress is within the deadline MUST be
Healthy. A projection whose lag exceeds the threshold but which is still making
progress within the deadline MUST be Degraded, not Unhealthy — lagging-but-
progressing is a degradation, not a failure. A projection that is behind (lag
greater than zero) AND has made no progress past the stall deadline MUST be
Unhealthy. A projection still performing its initial replay (no processed offset
yet) MUST report an initialization-pending condition rather than a dependency
failure. Every reported reason code MUST come from the existing closed health
code set; no new code and no free-text reason is introduced.

#### Scenario: Caught up within budget is Healthy
- GIVEN a projection whose lag is at or below the configured lag threshold and
  whose time-since-last-progress is at or below the stall deadline
- WHEN its health state is evaluated
- THEN it is Healthy

#### Scenario: Lagging but progressing is Degraded, not Unhealthy
- GIVEN a projection whose lag exceeds the configured threshold but whose
  time-since-last-progress is still within the stall deadline
- WHEN its health state is evaluated
- THEN it is Degraded, and specifically NOT Unhealthy

#### Scenario: Behind and stalled is Unhealthy
- GIVEN a projection whose lag is greater than zero and whose
  time-since-last-progress exceeds the stall deadline
- WHEN its health state is evaluated
- THEN it is Unhealthy with a closed-set reason code (unavailable), and no
  free-text reason

#### Scenario: Initial replay is initialization-pending, not a failure
- GIVEN a projection that has not yet written any processed offset because it is
  still performing its initial replay
- WHEN its health state is evaluated
- THEN it reports an initialization-pending condition, distinguishable from a
  real dependency failure at the same status
