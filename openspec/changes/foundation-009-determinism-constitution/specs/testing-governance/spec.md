## ADDED Requirements

### Requirement: Deterministic testing expectations

Testing SHALL comply with the deterministic validation expectations defined in the Determinism Constitution (`specs/determinism-constitution/spec.md`). Tests MUST NOT rely on wall-clock timing, hidden randomness, unstable concurrency timing, or environment-specific timing assumptions. Equivalent execution under equivalent inputs SHALL produce equivalent outcomes.

#### Scenario: Flaky execution
- **WHEN** a test produces different outcomes under equivalent conditions
- **THEN** validation SHALL fail

#### Scenario: Environment-independent testing
- **WHEN** test outcomes depend on machine timing, wall-clock time, or environment behavior
- **THEN** the test SHALL be treated as non-conformant

#### Scenario: Deterministic mock control
- **WHEN** a test uses a mock runtime, clock, or capability
- **THEN** the mock SHALL provide deterministic control over time, randomness, and ordering without real elapsed time
