## ADDED Requirements

### Requirement: Observability abstraction

The platform SHALL define observability as a semantic visibility contract — observing runtime behavior without owning execution. Observability SHALL be: runtime-neutral, transport-neutral, vendor-neutral, non-mutating.

Observability responsibilities:
- Capture semantic events (execution, lifecycle, message, failure)
- Provide deterministic correlation
- Support replay-safe observation

Observability non-responsibilities:
- Runtime execution or scheduling
- Transport or telemetry infrastructure
- Persistence lifecycle
- Cluster coordination

### Requirement: Observable semantics

The `Observability` trait SHALL define:
- `trace(event: SemanticEvent)` — record a semantic event
- `metric(name: &str, value: f64)` — record a metric
- `log(level: Level, message: &str)` — record a log entry

`SemanticEvent` SHALL contain: event_name, correlation_id, actor_id, lifecycle_state, timestamp, metadata map.

#### Scenario: Trace event captured
- **WHEN** a trace is recorded via the Observability trait
- **THEN** the event SHALL be inspectable by tests via the in-memory adapter

#### Scenario: Observability is non-mutating
- **WHEN** observability events are captured
- **THEN** they SHALL NOT alter runtime state or behavior

### Requirement: Replay-safe observability

Replay SHALL NOT create semantic ambiguity. Given identical inputs, replay SHALL produce identical observable events.

#### Scenario: Replay produces identical events
- **WHEN** an execution is replayed with identical inputs
- **THEN** the observable semantic events SHALL be identical

### Requirement: Testing contract

Tests SHALL use in-memory or noop adapters only. No test SHALL require telemetry infrastructure. Coverage SHALL be at least 95%.

#### Scenario: Test uses in-memory adapter
- **WHEN** a test exercises observability-dependent code
- **THEN** it SHALL inject an in-memory or noop adapter and SHALL NOT start any telemetry backend