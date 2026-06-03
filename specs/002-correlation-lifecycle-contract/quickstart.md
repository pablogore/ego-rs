# Quickstart: Correlation Lifecycle Contract

**Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

## Prerequisites

- Existing Persistence SPI contract tests pass (`cargo test`)

## Validation Scenarios

### Scenario 1 — Lifecycle contract is documented in spec 001

1. Open `specs/001-persistence-spi/spec.md`
2. Navigate to "Correlation Lifecycle" under Contract Invariants
3. Verify four lifecycle rules are present: creation origin, propagation path, retry survival, no downstream regeneration
4. **Expected**: Four lifecycle invariants with clear "MUST" / "MUST NOT" language

### Scenario 2 — Lifecycle propagation invariant in event-store contract

1. Open `specs/001-persistence-spi/contracts/event-store.md`
2. Verify the lifecycle propagation invariant is documented
3. **Expected**: EventStore contract states correlation_id flows from CommandContext through to loaded events without regeneration

### Scenario 3 — Verify cross-reference to specs 004 and 005

1. Open `specs/001-persistence-spi/spec.md`
2. Verify the lifecycle section references the scope boundary (004) and semantic boundary (005)
3. **Expected**: Cross-references present in the lifecycle section or Assumptions

### Scenario 4 — No behavioral changes

1. `cargo test`
2. **Expected**: All existing tests pass — documentation-only change
