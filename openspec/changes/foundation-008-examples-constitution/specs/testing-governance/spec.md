## ADDED Requirements

### Requirement: Example code coverage

Example code SHALL be subject to the same coverage requirements as production code. Coverage SHALL be measured using project-approved tooling. The project-wide coverage measurement SHALL include example code.

#### Scenario: CI measures example coverage
- **WHEN** coverage is measured
- **THEN** the measurement SHALL include both production code and example code

#### Scenario: Example coverage drops below threshold
- **WHEN** example code coverage falls below the project-wide threshold
- **THEN** the pipeline SHALL reject the change

### Requirement: Example determinism in tests

Example tests SHALL be deterministic. No example test SHALL depend on wall-clock time, random values, or external services unless those are explicitly injected through port parameters. Example tests MUST use mock runtime implementations for runtime-dependent code.

#### Scenario: Example test is non-deterministic
- **WHEN** an example test produces different results on repeated runs with identical inputs
- **THEN** the test SHALL be rejected

#### Scenario: Example test uses real runtime
- **WHEN** an example test imports or starts a concrete runtime implementation
- **THEN** the test SHALL be treated as a constitutional violation

### Requirement: Integration example test isolation

Examples that demonstrate integration with external systems SHALL be validated in a separate stage. Integration validation MUST NOT block the primary deterministic validation pipeline. Integration validation failures SHALL block release and full validation pipelines.

#### Scenario: Integration example test fails
- **WHEN** an integration example test fails in the full validation pipeline
- **THEN** the release SHALL be blocked

#### Scenario: Integration example test blocks primary pipeline
- **WHEN** an integration example test failure blocks the primary deterministic pipeline
- **THEN** this SHALL be a configuration violation
