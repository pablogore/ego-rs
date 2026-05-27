## ADDED Requirements

### Requirement: Architecture compliance for examples

Example code SHALL comply with the same hexagonal architecture rules as production code. Examples SHALL preserve architectural boundaries. Examples SHALL use deterministic patterns and respect fail-closed behavior. Examples MAY be single-crate, multi-crate, or modular, provided that dependency direction remains valid, hexagonal boundaries are preserved, and ports or adapters are respected.

Examples MUST NOT implement toy-only flows that violate architectural rules.

#### Scenario: Example has invalid dependency direction
- **WHEN** an example declares a dependency that violates inward-only dependency direction
- **THEN** this SHALL be a governance violation

#### Scenario: Example preserves boundaries across crates
- **WHEN** an example uses multiple crates
- **THEN** each crate SHALL respect layer boundaries and dependency direction

#### Scenario: Example implements toy-only flow
- **WHEN** an example bypasses architectural rules for convenience
- **THEN** the example SHALL be rejected

### Requirement: Ports and adapters in examples

Every external concern in example code SHALL be accessed through a trait defined in the domain or application layer. Example code MUST depend only on traits, never on concrete infrastructure types. Examples MUST NOT bypass ports or adapters for convenience or brevity.

#### Scenario: Example accesses external system through concrete type
- **WHEN** an example accesses a database, message broker, or HTTP endpoint through a concrete implementation instead of a port trait
- **THEN** this SHALL be a governance violation

#### Scenario: Example demonstrates port and adapter wiring
- **WHEN** an example demonstrates how to wire an adapter to a port
- **THEN** the wiring SHALL use the same pattern as production code

### Requirement: Architecture validation spans examples

Architecture validation SHALL apply to example code identically to production code. Any architecture violation in example code SHALL be treated identically to a violation in production code.

#### Scenario: Validation detects example violation
- **WHEN** architecture validation detects a dependency direction violation in example code
- **THEN** the violation SHALL be treated with the same severity as a production code violation

#### Scenario: Example is excluded from validation
- **WHEN** example code is excluded from architecture validation
- **THEN** this SHALL be a governance violation
