## ADDED Requirements

### Requirement: Persistence abstraction

The platform SHALL define persistence as a semantic durability contract — not a database abstraction, ORM, or repository pattern. Persistence SHALL be: storage-neutral, runtime-neutral, representation-neutral.

Persistence responsibilities:
- Durable recording and retrieval of events and state
- Deterministic guarantees for all operations
- Fail-closed behavior on ambiguity

Persistence non-responsibilities:
- Business orchestration
- Transport or networking
- Workflow execution
- Actor scheduling or lifecycle

### Requirement: EventStore trait

The `EventStore` trait SHALL define:
- `append(aggregate_id, expected_version, events) → Result<(), PersistenceError>` — optimistic concurrency via version
- `read(aggregate_id, from_version) → Result<Vec<Event>, PersistenceError>` — read events from version
- `read_all(aggregate_id) → Result<Vec<Event>, PersistenceError>` — read all events

#### Scenario: Append and read events
- **WHEN** events are appended with version N and read from version N
- **THEN** the read returns exactly those events in append order

#### Scenario: Version conflict rejected
- **WHEN** append is called with expected_version that differs from actual last version
- **THEN** the operation SHALL return `PersistenceError::VersionConflict`

### Requirement: SnapshotStore trait

The `SnapshotStore` trait SHALL define:
- `save(aggregate_id, version, state) → Result<(), PersistenceError>`
- `load(aggregate_id) → Result<Option<(u64, State)>, PersistenceError>` — returns latest snapshot if any

#### Scenario: Save and load snapshot
- **WHEN** a snapshot is saved at version V and loaded
- **THEN** the loaded snapshot SHALL match version V and the saved state

### Requirement: Replay semantics

Given identical event streams, replay SHALL produce identical reconstructed state. Snapshot + event catch-up: load latest snapshot, apply subsequent events to reach current state.

#### Scenario: Deterministic replay
- **WHEN** the same event stream is replayed twice
- **THEN** the reconstructed state SHALL be identical both times

### Requirement: Fail-closed behavior

Persistence operations SHALL fail closed. Storage ambiguity MUST NOT produce silent success. Version conflicts SHALL be explicit. Partial writes SHALL be rejected.

#### Scenario: Ambiguous outcome rejected
- **WHEN** a persist operation encounters an ambiguous outcome (timeout, partial write)
- **THEN** it SHALL return a failure acknowledgment, not success

### Requirement: Testing contract

Tests SHALL use in-memory adapters only. No test SHALL require a real database. Coverage SHALL be at least 95%.

#### Scenario: Test uses in-memory adapter
- **WHEN** a test exercises persistence-dependent code
- **THEN** it SHALL inject an in-memory adapter and SHALL NOT start any database