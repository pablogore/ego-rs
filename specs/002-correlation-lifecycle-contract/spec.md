# Correlation Lifecycle Contract — Requirements

**Spec**: Amendment to Persistence SPI (spec 001)
**Created**: 2026-06-03
**Status**: Draft
**Input**: User description: "correlation_id comes from CommandContext, but SPI does not define: what happens if null, if required upstream, if immutable across retries. Missing implicit spec: Correlation Lifecycle Contract — created in CommandContext, propagated to EventStore, MUST survive retries, MUST NOT be regenerated downstream."

## User Scenarios & Testing

### User Story 1 — Command issues events with traceable correlation (Priority: P1)

A framework developer implementing a command handler creates a `CommandContext` that carries a `correlation_id` and passes it through event creation. Events persisted via the EventStore carry the same `correlation_id`, enabling end-to-end tracing.

**Why this priority**: Without creation-side correlation, no traceability exists. This is the root of the lifecycle.

**Independent Test**: Create a command context with a known correlation_id, produce events, append them. Load the stream and verify every event carries the original correlation_id.

**Acceptance Scenarios**:

1. **Given** a CommandContext with `correlation_id = "abc-123"`, **When** events are created and appended, **Then** all appended events carry `correlation_id = "abc-123"`
2. **Given** a CommandContext with `correlation_id = None`, **When** events are created and appended, **Then** all appended events carry `correlation_id = None`
3. **Given** a CommandContext with a valid `correlation_id`, **When** the command execution spans multiple event batches, **Then** all batches carry the same `correlation_id`
4. **Given** two different CommandContexts with different `correlation_id` values, **When** events are created and appended, **Then** each event preserves its originating `correlation_id`

---

### User Story 2 — Correlation_id survives retry (Priority: P1)

A command handler fails mid-execution (e.g., concurrency conflict) and retries. The retry attempt uses the same `correlation_id` as the original attempt, so all events from both attempts are traceable to the same original command.

**Why this priority**: Without retry survival, retried commands appear as separate trace chains, breaking observability.

**Independent Test**: Simulate a failed append, retry with the same CommandContext, and verify all persisted events (from both attempts) carry the same correlation_id.

**Acceptance Scenarios**:

1. **Given** a CorrelationContext with `correlation_id = "abc-123"`, **When** an append fails and the command is retried with the same context, **Then** the retried events also carry `correlation_id = "abc-123"`
2. **Given** a CommandContext with `correlation_id = "abc-123"`, **When** the same command is retried multiple times, **Then** every attempt's events carry the same `correlation_id`
3. **Given** a CommandContext with `correlation_id = None`, **When** the command is retried, **Then** retried events also carry `correlation_id = None`

---

### User Story 3 — Downstream consumers propagate original correlation_id (Priority: P1)

An event handler reads an event with `correlation_id = "abc-123"` and produces a new command in response. The new command's context carries the same `correlation_id`, preserving the trace chain across causal hops.

**Why this priority**: Without propagation, trace chains break at the first downstream consumer, defeating end-to-end observability.

**Independent Test**: Produce a source event with a known correlation_id, process it through a handler that produces a new command, verify the new command's correlation_id matches the source.

**Acceptance Scenarios**:

1. **Given** an event with `correlation_id = "abc-123"`, **When** a downstream handler processes it and issues a new command, **Then** the new command's context carries `correlation_id = "abc-123"`
2. **Given** an event with `correlation_id = None`, **When** a downstream handler processes it and issues a new command, **Then** the new command's context carries `correlation_id = None`
3. **Given** an event with `correlation_id = "abc-123"`, **When** a downstream handler processes it, **Then** the handler MUST NOT replace the correlation_id with a new value or a generated value

---

### User Story 4 — External commands without correlation_id are accepted (Priority: P2)

An external system sends a command without a correlation_id. The system accepts the command, processes it, and persisted events carry `correlation_id = None`.

**Why this priority**: Backward compatibility with external systems that do not support correlation tracing.

**Independent Test**: Submit a command with no correlation_id. Verify events are persisted and loaded with `correlation_id = None`.

**Acceptance Scenarios**:

1. **Given** an external command with no correlation_id, **When** the command is processed and events are appended, **Then** events carry `correlation_id = None`
2. **Given** an external command with no correlation_id, **When** events are loaded downstream, **Then** `correlation_id` remains `None`

---

### Edge Cases

- What happens when a downstream handler produces multiple new commands from a single event — do all carry the original correlation_id?
- What happens when a retry occurs after partial event persistence (if the system permits non-atomic retry)?
- How does the system behave when the correlation_id is an empty string (distinct from None)?
- What happens when the correlation_id exceeds reasonable length limits?
- What happens when a downstream handler creates a new causality chain (not causally related to the source event)?

## Requirements

### Functional Requirements

- **FR-001 (Creation)**: The `correlation_id` SHALL originate in the command processing context at the point where a command is first received. The same `correlation_id` SHALL be used throughout the lifecycle of processing that command.
- **FR-002 (Optionality)**: `correlation_id` SHALL be optional (`Option`-like). A value of `None` or equivalent SHALL mean "no traceability link" and SHALL NOT be replaced with an auto-generated value at any layer.
- **FR-003 (Retry survival)**: When a command is retried due to failure, the retry SHALL use the same `correlation_id` as the original attempt. The correlation_id is bound to the command identity, not the execution attempt.
- **FR-004 (Causal propagation)**: When a downstream handler or consumer processes an event and produces a new command that is causally related to the source event, the new command's context SHALL carry the same `correlation_id` as the source event.
- **FR-005 (No regeneration)**: No layer (persistence, infrastructure, application) SHALL generate, replace, or overwrite a `correlation_id`. The only valid operations are: pass-through (from CommandContext to EventStore) and propagate (from source event to new causally-related command).
- **FR-006 (EventStore passthrough)**: The EventStore SHALL store and return the `correlation_id` as provided. It SHALL NOT inspect, validate, transform, or generate correlation_ids.
- **FR-007 (Immutability in storage)**: Once an event is appended with a given `correlation_id`, that value SHALL be immutable for the lifetime of the event. No operation may modify a persisted correlation_id.

### Key Entities

- **CommandContext**: The processing context created when a command is received. Owns the `correlation_id` for the duration of command execution.
- **CorrelationId**: An opaque identifier (string) that links an event to the command that produced it. Optional — may be `None`.
- **StoredEvent**: The event envelope that wraps a domain event with its `correlation_id`. Defined in the Persistence SPI.
- **Downstream Handler**: Any component that consumes events and may produce new commands in response.

## Contract Invariants

The following behavioral guarantees apply to correlation_id across all layers.

### Origin

- The `correlation_id` is established at command ingress and MUST NOT be created or assigned by infrastructure or persistence layers.
- A CommandContext with `correlation_id = None` is valid and means the command origin did not provide one.

### Propagation

- `correlation_id` flows from CommandContext → Domain Events → EventStore (via StoredEvent envelope) → Loaded Events → Downstream Handlers.
- At each hop, the value SHALL be preserved byte-for-byte. No transformation, truncation, encoding, or decoding SHALL alter the value.

### Retries

- The `correlation_id` is part of the command's logical identity. All retries of the same logical command SHALL use an identical `correlation_id`.

### Downstream

- When a downstream handler produces a new command that is causally related to a source event, it SHALL propagate the source event's `correlation_id`.
- When a downstream handler produces a new command that starts an independent causality chain (not causally related), it SHALL use `correlation_id = None` unless an external correlation_id is provided.

### Immutability

- Persisted `correlation_id` values SHALL be immutable. No update, migration, or retroactive assignment is permitted.

## Constraints

- The EventStore SHALL NOT depend on the presence or format of correlation_id — it is a passive carrier.
- Correlation_id values SHALL be treated as opaque strings by all layers below the application boundary.
- The persistence layer SHALL NOT auto-generate correlation_ids under any circumstance.

## Out of Scope

The following concerns are NOT addressed by this specification:

- Correlation_id generation format or algorithm (UUID, ULID, etc.) — this is an application-layer concern.
- CommandContext implementation or lifecycle management — the spec defines what CommandContext MUST provide, not how it is built.
- Cross-system correlation (e.g., HTTP headers, message broker headers) — the spec covers in-system correlation only.
- Correlation_id validation or schema — the value is opaque.
- Event metadata beyond correlation_id (e.g., causation_id, user_id, tenant_id).
- Retry policy or backoff strategy — the spec defines correlation_id behavior under retry, not the retry mechanism itself.
- Observability infrastructure (tracing export, log correlation) — the correlation_id enables these but does not implement them.

## Success Criteria

### Measurable Outcomes

- **SC-001**: A developer can append an event with a known correlation_id, simulate a retry, load the stream, and verify all events across all attempts carry the same correlation_id.
- **SC-002**: A developer can append an event with `correlation_id = None`, load it, process it through a downstream handler, and verify the handler's new command also carries `correlation_id = None`.
- **SC-003**: A developer can verify that the EventStore never modifies, generates, or discards a provided correlation_id by appending known values and asserting they are returned unchanged.
- **SC-004**: A developer can verify downstream handlers propagate rather than regenerate correlation_ids by processing a known event and asserting the handler's output command carries the original value unchanged.

## Assumptions

- The Persistence SPI (spec 001) provides `StoredEvent<E>` with `correlation_id: Option<String>` as the event envelope.
- CommandContext is provided by the application layer — this spec defines its contractual obligations regarding correlation_id.
- Downstream handlers are capable of reading and propagating correlation_id — the spec defines the behavioral contract.
- Correlation_id values are within reasonable length limits appropriate for the storage backend (implementation detail, not specified here).
- The system has at-most-once or exactly-once command processing semantics that prevent duplicate command identity across retries.
