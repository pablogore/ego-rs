# Quickstart: Correlation Semantic Boundary

**Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

## Prerequisites

- All existing Persistence SPI contracts and tests pass (`cargo test`)

## Validation Scenarios

### Scenario 1 — Verify negative semantics are documented in spec 001

1. Open `specs/001-persistence-spi/spec.md`
2. Navigate to "Correlation ID Semantic Boundaries" section under Contract Invariants
3. Verify all four boundaries are present: Security, Correctness, Ordering, Deduplication
4. Each boundary has explicit "MUST NOT" or "NOT" language
5. **Expected**: Four subsections with clear negative semantics

### Scenario 2 — Verify negative semantics are documented in event-store contract

1. Open `specs/001-persistence-spi/contracts/event-store.md`
2. Navigate to "Correlation ID Semantic Boundaries" under Behavioral Contract
3. Verify all four "NOT" statements are present
4. **Expected**: Four bullet points matching the boundaries in the spec

### Scenario 3 — Verify data-model mentions boundaries

1. Open `specs/001-persistence-spi/data-model.md`
2. Find the StoredEvent entity description
3. Verify "Semantic boundaries" is listed with the four "NOT" statements
4. **Expected**: StoredEvent entry includes the negative semantics

### Scenario 4 — Verify no behavioral changes

1. Run existing tests: `cargo test`
2. Verify no new test failures
3. **Expected**: All existing tests pass — this is a documentation-only change
