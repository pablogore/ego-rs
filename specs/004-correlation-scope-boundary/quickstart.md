# Quickstart: Correlation Scope Boundary

**Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

## Prerequisites

- Existing Persistence SPI tests pass (`cargo test`)

## Validation Scenarios

### Scenario 1 — Repository contract states correlation_id is not a concern

1. Open `specs/001-persistence-spi/contracts/repository.md`
2. Verify Behavioral Contract states correlation_id is NOT a Repository concern
3. **Expected**: Explicit statement that correlation_id is out of scope for Repository

### Scenario 2 — Snapshot contract states correlation_id is not a concern

1. Open `specs/001-persistence-spi/contracts/snapshot.md`
2. Verify Behavioral Contract states correlation_id is NOT a Snapshot concern
3. **Expected**: Explicit statement that correlation_id is out of scope for Snapshot

### Scenario 3 — EventStore contract states correlation_id ownership

1. Open `specs/001-persistence-spi/contracts/event-store.md`
2. Verify correlation_id is documented as an EventStore-owned concept
3. **Expected**: EventStore declares ownership; others declare non-ownership

### Scenario 4 — No behavioral changes

1. `cargo test`
2. Verify Repository and Snapshot operations work without correlation_id
3. **Expected**: All tests pass — no behavioral changes introduced
