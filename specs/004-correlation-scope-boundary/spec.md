# Correlation Scope Boundary — Requirements

**Spec**: Cross-contract clarification for Persistence SPI (spec 001)
**Created**: 2026-06-03
**Status**: Draft
**Input**: User description: "EventStore has correlation_id, Repository does not mention it. This creates dual persistence semantics. Need to clarify: correlation_id is Event-only concept — NOT repository concern, NOT snapshot concern, ONLY event stream concern."

## User Scenarios & Testing

### User Story 1 — Developer understands which contracts own correlation_id (Priority: P1)

A framework developer reads the SPI contracts and can determine, without ambiguity, which persistence contracts are responsible for correlation_id and which are not.

**Why this priority**: Without explicit boundaries, developers may (a) add correlation_id to Repository or Snapshot unnecessarily, creating dual semantics, or (b) assume correlation_id is absent from event sourcing if they look only at Repository.

**Independent Test**: Read the SPI contracts and verify:
- EventStore documents correlation_id ownership
- Repository and Snapshot explicitly state correlation_id is NOT their concern

**Acceptance Scenarios**:

1. **Given** the EventStore contract, **When** a developer reads it, **Then** it clearly states that correlation_id is part of the event envelope and owned by the event stream
2. **Given** the Repository contract, **When** a developer reads it, **Then** it explicitly states that correlation_id is NOT a Repository concern
3. **Given** the Snapshot contract, **When** a developer reads it, **Then** it explicitly states that correlation_id is NOT a Snapshot concern

---

### User Story 2 — Repository users are not misled about correlation_id (Priority: P1)

An application developer using the Repository trait for aggregate persistence does not encounter correlation_id in the Repository API and correctly concludes it is not needed for aggregate persistence operations.

**Why this priority**: If Repository mentioned or hinted at correlation_id, developers would expect it as a parameter, creating confusion and unnecessary coupling between aggregate persistence and event tracing.

**Independent Test**: Save and load an aggregate via Repository, verify no correlation_id parameter is required or expected.

**Acceptance Scenarios**:

1. **Given** the Repository `save` operation, **When** called, **Then** it does not accept a correlation_id parameter
2. **Given** the Repository `load` operation, **When** called, **Then** it does not return correlation_id information
3. **Given** an aggregate saved via Repository, **When** loaded, **Then** the aggregate state is retrieved without any correlation_id processing

---

### User Story 3 — Event producers know where to attach correlation_id (Priority: P1)

A developer implementing a command handler knows that correlation_id must be attached to events (via StoredEvent envelope) before appending to EventStore, and that Repository/Snapshot operations do not participate in the correlation lifecycle.

**Why this priority**: Clear separation prevents architectural drift where correlation concerns leak into non-event contracts.

**Independent Test**: Append an event with correlation_id to EventStore, save the aggregate state via Repository, take a snapshot. Verify all three operations succeed independently and correlation_id exists only on the event.

**Acceptance Scenarios**:

1. **Given** a command handler that produces events and saves aggregate state, **When** correlation_id is attached to events, **Then** the event stream shows the correlation_id, and the Repository save succeeds without it
2. **Given** an event with correlation_id appended to EventStore, **When** a snapshot is taken at the same version, **Then** the snapshot does not carry correlation_id

---

### Edge Cases

- What if an implementation stores events and aggregate state in the same database table — does the column for aggregate state need a correlation_id column? (No — correlation_id is an event-only concern)
- What if a developer wants to trace which command last modified an aggregate — is that the same as correlation_id? (No — that is aggregate audit metadata, not event correlation)
- What if a backend implementation internally joins event and aggregate data — does the correlation_id leak into the aggregate view? (No — the correlation boundary is a SPI contract, not a storage schema)

## Requirements

### Functional Requirements

- **FR-001 (EventStore owns correlation_id)**: The EventStore contract SHALL be the sole persistence contract responsible for correlation_id. Correlation_id SHALL be defined in the event envelope (`StoredEvent<E>`) and SHALL NOT appear in any other persistence contract's trait signature.
- **FR-002 (Repository excluded)**: The Repository contract SHALL NOT define, accept, return, or process correlation_id in any of its operations (`save`, `load`, `delete`). Correlation_id is orthogonal to aggregate persistence.
- **FR-003 (Snapshot excluded)**: The Snapshot contract SHALL NOT define, accept, return, or process correlation_id in any of its operations (`save_snapshot`, `load_snapshot`). Correlation_id is orthogonal to snapshot persistence.
- **FR-004 (Explicit documentation)**: Each persistence contract (EventStore, Repository, Snapshot) SHALL explicitly document its relationship to correlation_id in its behavioral contract. EventStore SHALL state "correlation_id is owned here"; Repository and Snapshot SHALL state "correlation_id is not a concern of this contract."
- **FR-005 (No dual semantics)**: No SPI implementation SHALL require correlation_id for Repository or Snapshot operations. Implementations that share a backing store between events and aggregates MAY store correlation_id in the event table only, never in the aggregate or snapshot tables.

### Key Entities

- **CorrelationId**: An opaque traceability identifier owned exclusively by the EventStore event envelope. Not present in Repository or Snapshot contracts.
- **EventStore**: The persistence contract that owns correlation_id as part of the `StoredEvent<E>` envelope.
- **Repository**: The persistence contract for aggregate state. Correlation_id is out of scope.
- **Snapshot**: The persistence contract for aggregate snapshots. Correlation_id is out of scope.
- **Event Stream**: The sole traceability boundary. Correlation_id lives and dies with the event stream.

## Contract Invariants

The following behavioral guarantees apply across all persistence contracts regarding correlation_id ownership.

### Ownership

- Correlation_id SHALL be defined in exactly one persistence contract: EventStore (via `StoredEvent<E>` envelope).
- No other persistence contract SHALL reference, depend on, or transport correlation_id.

### Separation

- Repository operations SHALL be correlation_id-agnostic. An aggregate save, load, or delete SHALL succeed regardless of whether correlation_id is present in the event stream.
- Snapshot operations SHALL be correlation_id-agnostic. A snapshot save or load SHALL succeed regardless of correlation_id state.

### No Propagation Requirement

- Repository and Snapshot implementations SHALL NOT be required to propagate, preserve, or handle correlation_id. The correlation lifecycle (spec 002) applies exclusively to the EventStore and the event stream.

## Constraints

- The EventStore contract SHALL NOT impose correlation_id requirements on Repository or Snapshot implementations.
- The Repository and Snapshot contracts SHALL remain correlation_id-free in their trait signatures.
- Correlation_id MUST NOT appear in contract test suites for Repository or Snapshot — only EventStore contract tests validate correlation behavior.

## Out of Scope

The following concerns are NOT addressed by this specification:

- Implementation details of how events and aggregates share a backing store — storage schema is an infrastructure concern.
- Audit logging or command-level trace metadata on aggregates — these are separate concerns from event correlation.
- Cross-cutting observability that correlates aggregate state with event causation — this is an application-layer concern.
- Whether the EventStore and Repository share a transaction boundary — transaction semantics are separate from correlation ownership.

## Success Criteria

### Measurable Outcomes

- **SC-001**: A developer can read the SPI contracts (EventStore, Repository, Snapshot) and determine in under 1 minute which contract(s) involve correlation_id.
- **SC-002**: A contract test written against the Repository trait passes without any reference to correlation_id in the test setup or assertions.
- **SC-003**: A contract test written against the Snapshot trait passes without any reference to correlation_id in the test setup or assertions.
- **SC-004**: An in-memory implementation of all three SPI contracts can be written where correlation_id is handled exclusively in the EventStore implementation and never referenced in Repository or Snapshot code.

## Assumptions

- The Persistence SPI (spec 001) defines EventStore, Repository, and Snapshot as independent contracts.
- The StoredEvent envelope (spec 001) wraps domain events with optional correlation_id — this is the only correlation_id carrier.
- The Correlation Lifecycle Contract (spec 002) defines the behavioral rules for correlation_id within the EventStore scope.
- The Snapshot Trace Continuity spec (spec 003) confirms snapshots do not carry correlation_id and that trace continuity is maintained via EventStore delta replay.
