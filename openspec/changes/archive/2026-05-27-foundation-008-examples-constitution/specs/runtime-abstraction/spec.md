## ADDED Requirements

### Requirement: Runtime SPI examples demonstrate constitutional patterns

Runtime SPI examples SHALL demonstrate runtime capability ports through constitutional patterns, not through bypass or shortcut approaches. Examples SHALL use runtime SPI ports through their trait interfaces. Examples MUST NOT bypass the runtime SPI by accessing concrete runtime implementations.

#### Scenario: Runtime example accesses concrete runtime
- **WHEN** a runtime example directly instantiates a concrete runtime implementation instead of using SPI ports
- **THEN** the example SHALL be rejected

#### Scenario: Runtime example demonstrates SPI port usage
- **WHEN** a runtime example shows how to use a runtime capability port
- **THEN** it SHALL use the port trait and MUST NOT import a concrete runtime implementation

### Requirement: Example determinism

Runtime examples SHALL demonstrate deterministic execution behavior. Examples that use logical time, scheduling, or execution boundaries SHALL use deterministic patterns. Examples MUST NOT demonstrate non-deterministic patterns as acceptable usage.

#### Scenario: Example uses logical time
- **WHEN** a runtime example demonstrates time-dependent behavior
- **THEN** it SHALL use the logical time capability provided by the runtime and SHALL NOT use wall-clock time

#### Scenario: Example demonstrates retry boundaries
- **WHEN** a runtime example demonstrates retry behavior
- **THEN** it SHALL use runtime retry support through the defined SPI

### Requirement: Test runtime for examples

Example tests for runtime-dependent code SHALL use mock runtime implementations. No example test SHALL require a real runtime.

#### Scenario: Example test uses mock runtime
- **WHEN** an example test exercises code that depends on runtime capability ports
- **THEN** the test SHALL inject a mock runtime implementation

#### Scenario: Example test controls logical time
- **WHEN** an example test exercises code that depends on the logical time capability
- **THEN** the test SHALL inject a mock runtime that provides deterministic time control
