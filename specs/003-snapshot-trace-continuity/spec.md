# Snapshot Trace Continuity — Requirements

**Spec**: Amendment to Persistence SPI (spec 001) — Snapshot contract
**Created**: 2026-06-03
**Status**: Draft
**Input**: User description: "Snapshot model is payload + version, but does not include correlation context. Problem: snapshot restore + replay delta loses historical traceability. SPI decision needed: snapshots MAY NOT store correlation_id, BUT must not break event trace continuity."

## User Scenarios & Testing

### User Story 1 — Snapshot restore preserves event trace continuity (Priority: P1)

A framework developer loads an aggregate from a snapshot and replays delta events. The replayed events carry their original correlation_ids, preserving the complete trace chain from stream creation through snapshot to current state.

**Why this priority**: Without trace continuity across snapshot boundaries, observability is broken for aggregates that have been snapshotted — the correlation trail appears to start from the snapshot rather than from the original command.

**Independent Test**: Append events with known correlation_ids, create a snapshot, load from snapshot version, replay delta events, and verify all delta events carry their original correlation_ids unchanged.

**Acceptance Scenarios**:

1. **Given** an event stream with events carrying `correlation_id = "abc-123"`, **When** a snapshot is taken at version N and the stream is restored from the snapshot, **Then** delta events (version > N) are returned with their original `correlation_id` values
2. **Given** multiple snapshots at different versions, **When** restoring from any snapshot, **Then** all subsequent delta events preserve their original correlation_ids
3. **Given** a snapshot at version N and delta events at versions N+1 through N+M, **When** the aggregate is restored from the snapshot, **Then** the full set of delta events is available for replay with complete metadata

---

### User Story 2 — Snapshot itself is not required to carry correlation_id (Priority: P2)

The Snapshot trait MAY omit correlation_id from its own payload. The snapshot is a performance optimization, not a traceability source — trace continuity is maintained by replaying delta events from the EventStore.

**Why this priority**: Relaxing this requirement prevents unnecessary complexity in the Snapshot trait while maintaining full traceability through the event stream.

**Independent Test**: Create a snapshot without any correlation metadata, restore from it, verify delta events carry correct correlation_ids.

**Acceptance Scenarios**:

1. **Given** a snapshot saved without correlation_id, **When** restored, **Then** the snapshot restore operation succeeds and delta events retain their correlation_ids
2. **Given** the Snapshot trait with `payload + version` only (no correlation_id field), **When** any snapshot operation is performed, **Then** the Snapshot trait compiles and behaves correctly

---

### User Story 3 — Snapshot restore + event replay is the only path to current state (Priority: P1)

The Snapshot contract guarantees that restore + delta replay produces an aggregate state equivalent to replaying the full event stream from version 0. No trace information is lost in this process.

**Why this priority**: This is the core correctness guarantee — snapshots are transparent to traceability.

**Independent Test**: Compare two methods: (a) replay full event stream from version 0, and (b) restore from snapshot + replay delta. Verify the resulting correlation_id chains are identical for the overlapping range.

**Acceptance Scenarios**:

1. **Given** a full event stream where events have known correlation_ids, **When** the stream is replayed from version 0, **Then** the trace chain contains every correlation_id in order
2. **Given** the same stream with a snapshot at version N, **When** restored + delta replayed, **Then** the trace chain for versions > N is identical to the full replay
3. **Given** a snapshot at version N and delta events, **When** the delta events are inspected, **Then** no correlation_id from the pre-snapshot portion leaks into the delta — the delta contains only its own events' correlation_ids

---

### Edge Cases

- What happens when delta events have mixed correlation_id values (some None, some Some)?
- What happens when multiple snapshots exist and the system restores from an older snapshot — are all delta events (up to latest) traceable?
- What happens when a snapshot version equals the latest event version (no delta to replay)?
- What happens when the snapshot payload is corrupted but the event stream is intact?

## Requirements

### Functional Requirements

- **FR-001 (Snapshot restore)**: The Snapshot trait SHALL support loading the latest snapshot for an aggregate. After loading, delta events from the EventStore (versions > snapshot version) SHALL be replayable.
- **FR-002 (Correlation preservation)**: Delta events loaded after snapshot restore SHALL carry the same `correlation_id` values they had when originally appended. The restore operation SHALL NOT alter, strip, or regenerate correlation_ids on delta events.
- **FR-003 (Snapshot optionality)**: The Snapshot trait MAY define its data as `(version, payload)` without including `correlation_id`. The correlation lifecycle is owned by the EventStore, not the Snapshot.
- **FR-004 (Trace equivalence)**: Replaying an aggregate from snapshot + delta events SHALL produce a trace chain (ordered list of correlation_ids from replayed events) that is identical to the corresponding segment of a full stream replay for the overlapping version range.
- **FR-005 (No trace leakage)**: Correlation_ids from events preceding the snapshot version SHALL NOT appear in the delta event stream. Delta events contain only their own metadata.
- **FR-006 (Empty delta)**: When snapshot version equals the latest event version (no delta), the restore operation SHALL succeed and produce no delta events.

### Key Entities

- **Snapshot**: A cached representation of aggregate state at a specific version, defined as `(version, payload)`. Correlation_id is not part of the snapshot model.
- **Delta Events**: Events that occurred after the snapshot version, loaded from the EventStore with their original correlation_ids intact.
- **Trace Chain**: The ordered sequence of correlation_ids across an aggregate's event stream, used for end-to-end observability.
- **EventStore**: The source of truth for events and their correlation_ids. Always consulted for delta events after snapshot restore.

## Contract Invariants

The following behavioral guarantees apply to the Snapshot contract as amended.

### Snapshot Transparency

- Snapshots are transparent to traceability. All trace information originates from the EventStore, not the snapshot.
- A snapshot restore followed by delta replay SHALL produce the same observable correlation state as replaying the full event stream.

### Correlation Integrity

- The Snapshot contract SHALL NOT define, store, or modify correlation_id. Correlation_id is owned by the EventStore.
- Delta events loaded after snapshot restore SHALL preserve all metadata (including correlation_id) from the original append.

### Ordering

- Delta events SHALL be returned in append order, as defined by the EventStore contract.
- The snapshot version SHALL define the upper bound of the delta: only events with version > snapshot version are replayed.

## Constraints

- The Snapshot trait SHALL remain runtime-neutral (no async in trait signatures), consistent with the existing SPI constraint.
- The correlation_id lifecycle contract (spec 002) SHALL apply to delta events without exception.
- The Snapshot trait SHALL NOT introduce new dependencies on the EventStore trait at the SPI level — snapshot and event store are independent contracts. The restore + replay pattern is an application-layer concern.

## Out of Scope

The following concerns are NOT addressed by this specification:

- Snapshot creation strategy or policy (frequency, version intervals, trigger conditions).
- Snapshot storage backend or serialization format.
- Migration of existing snapshots — the spec defines forward behavior only.
- Retroactive assignment of correlation_id to snapshots taken before this amendment.
- Performance characteristics of snapshot+replay vs full stream replay.
- Implementation of the restore+replay orchestration logic at the application layer.

## Success Criteria

### Measurable Outcomes

- **SC-001**: A developer can take a snapshot of an aggregate with events, restore from that snapshot, replay delta events, and verify every delta event carries the same correlation_id it had before the snapshot.
- **SC-002**: A developer can verify that a snapshot struct/type does not require a correlation_id field by compiling and testing the Snapshot trait with only `(version, payload)`.
- **SC-003**: A developer can prove trace equivalence by comparing the correlation_id chain from snapshot+replay against the same version range from full stream replay — both chains match.
- **SC-004**: When snapshot version equals latest event version, restore succeeds with zero delta events and no trace data loss.

## Assumptions

- The EventStore contract preserves correlation_id on all events through append and load (per spec 001, amendment spec 002).
- The Snapshot contract defines `save_snapshot(aggregate_id, tenant_id, version, payload)` and `load_snapshot(aggregate_id, tenant_id)` returning `(version, payload)` — as defined in spec 001.
- Application-layer code orchestrates the restore + replay workflow; the SPI defines the contracts, not the workflow.
- Delta events are loaded from the EventStore and carry the same behavioral guarantees as the EventStore contract.
