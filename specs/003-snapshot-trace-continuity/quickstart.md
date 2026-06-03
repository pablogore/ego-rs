# Quickstart: Snapshot Trace Continuity

**Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

## Prerequisites

- Existing Persistence SPI tests pass (`cargo test`)
- Snapshot contract exists and works without correlation_id

## Validation Scenarios

### Scenario 1 — Snapshot contract does not carry correlation_id

1. Read `specs/001-persistence-spi/contracts/snapshot.md`
2. Verify Snapshot trait signature has no correlation_id parameter
3. Verify `load_snapshot` returns `(version, payload)` without correlation_id
4. **Expected**: Snapshot trait is correlation_id-free

### Scenario 2 — Trace continuity documented in spec 001

1. Open `specs/001-persistence-spi/spec.md`
2. Navigate to Snapshot-related Contract Invariants
3. Verify trace continuity guarantee is documented: snapshot restore + delta replay preserves correlation_ids
4. **Expected**: Explicit invariant that trace continuity is maintained across snapshot boundaries

### Scenario 3 — Verify snapshot restore + delta replay

1. Use existing InMemoryEventStore: append events with known correlation_ids
2. Use InMemorySnapshotStore: take snapshot at version N
3. Load snapshot, load delta events (version > N) from EventStore
4. Verify all delta events carry original correlation_ids unchanged
5. **Expected**: Delta events preserve correlation_ids — trace continuity is maintained

### Scenario 4 — No behavioral changes

1. `cargo test`
2. **Expected**: All existing tests pass — documentation-only change
