# Research: Snapshot Trace Continuity

**Phase**: 0 — Outline & Research | **Spec**: [spec.md](spec.md)

## Unknowns Assessment

No technical unknowns. The Snapshot contract already operates without correlation_id, and the EventStore preserves correlation_id on all delta events. The trace continuity guarantee documents existing behavior.

## Design Decisions

### Decision 1: Snapshot remains correlation_id-free

- **Decision**: No correlation_id field added to Snapshot trait.
- **Rationale**: Per spec 004 (Correlation Scope Boundary), correlation_id is an Event-only concept. Adding it to Snapshot would violate the scope boundary and create dual semantics.
- **Alternatives considered**: Adding optional correlation_id to Snapshot payload. Rejected — violates §004 and creates unnecessary complexity.

### Decision 2: Trace continuity via EventStore delta replay

- **Decision**: Trace continuity is guaranteed by the EventStore, not the Snapshot.
- **Rationale**: The EventStore preserves correlation_id on all events. Snapshot restore + delta replay from EventStore produces the complete trace chain. The Snapshot's responsibility is only to provide the version boundary for delta selection.
- **Alternatives considered**: Embedding trace metadata in snapshot payload. Rejected — violates scope boundary and duplicates EventStore data.

## Recommendations

Proceed to Phase 1 (Design & Contracts).
