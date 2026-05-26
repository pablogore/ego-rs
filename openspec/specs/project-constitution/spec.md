## ADDED Requirements

### Requirement: Deterministic-first behavior
Project behavior SHALL be deterministic by default. Given the same inputs and persisted state, domain and application logic MUST produce the same outputs without relying on hidden randomness, wall-clock time, external services, or mutable global state.

#### Scenario: Domain logic receives identical inputs
- **WHEN** domain logic is executed twice with the same inputs and state
- **THEN** it SHALL produce equivalent outputs and events

#### Scenario: Non-deterministic input is required
- **WHEN** a workflow needs time, randomness, or external data
- **THEN** that input SHALL be provided through an explicit port or parameter

### Requirement: Fail-closed decisions
Validation, authorization, parsing, and governance checks SHALL fail closed. Unknown, invalid, incomplete, or ambiguous input MUST result in rejection or an explicit error, not permissive fallback behavior.

#### Scenario: Required validation input is missing
- **WHEN** a decision cannot be made because required input is missing
- **THEN** the system SHALL reject the operation or return an explicit error

#### Scenario: Governance check cannot complete
- **WHEN** a governance check cannot inspect the required artifact
- **THEN** the check SHALL fail instead of assuming compliance

### Requirement: Explicit state only
Project code SHALL avoid hidden mutable state. State transitions MUST be represented through explicit inputs, outputs, persisted events, or declared repositories/ports.

#### Scenario: Domain state changes
- **WHEN** domain state changes
- **THEN** the change SHALL be represented by an explicit command result, event, or returned value

#### Scenario: Hidden mutable state is introduced
- **WHEN** code introduces global mutable state or implicit process-local state for business behavior
- **THEN** the change SHALL be rejected unless a constitution amendment permits it

### Requirement: Append-only lineage
Project decisions, specs, events, and migrations SHALL preserve append-only lineage. Historical records MUST remain auditable and MUST NOT be silently rewritten.

#### Scenario: Spec rule changes
- **WHEN** a constitution or governance rule changes
- **THEN** the change SHALL be introduced through a new OpenSpec change with migration notes

#### Scenario: Domain event is recorded
- **WHEN** a domain event is persisted
- **THEN** it SHALL be appended as an immutable record rather than updated in place

### Requirement: OpenSpec-driven development
All non-trivial project changes SHALL start with an OpenSpec change before implementation. Implementation tasks MUST be traceable to proposal, design, and spec artifacts.

#### Scenario: New feature is proposed
- **WHEN** a contributor proposes a new feature, architectural rule, or behavior change
- **THEN** the contributor SHALL create or update an OpenSpec change before implementation

#### Scenario: Implementation task is performed
- **WHEN** an implementation task is completed
- **THEN** it SHALL correspond to a task entry in the active change

### Requirement: Mandatory architecture governance
All production code SHALL comply with the architecture governance spec. Hexagonal architecture, dependency direction, ports/adapters, CQRS, and event-driven design are mandatory project standards.

#### Scenario: New crate or module is introduced
- **WHEN** a new crate or module is added
- **THEN** it SHALL declare its architectural layer and comply with the allowed dependency direction

#### Scenario: Command and query behavior is added
- **WHEN** behavior mutates state or reads state
- **THEN** commands and queries SHALL remain separated and events SHALL represent state-changing outcomes

### Requirement: Mandatory testing governance
All testable project code SHALL comply with the testing governance spec. Project-wide coverage SHALL be at least 95%, unit tests MUST use mocks for external resources, and tests MUST NOT require real infrastructure.

#### Scenario: Unit test exercises a dependency
- **WHEN** a unit test needs a database, network, filesystem, broker, or HTTP dependency
- **THEN** it SHALL use a mock or in-memory test double through a trait boundary

#### Scenario: Coverage is measured
- **WHEN** coverage is measured in CI
- **THEN** the project SHALL fail the check if coverage is below 95%

### Requirement: Observability by default
New production workflows SHALL expose enough structured logs, metrics, or traces to diagnose decisions and failures without adding ad hoc instrumentation after an incident.

#### Scenario: Command handler processes a command
- **WHEN** a command handler accepts, rejects, or fails a command
- **THEN** it SHALL emit structured observability data that identifies the decision path without leaking sensitive data

#### Scenario: External adapter fails
- **WHEN** an adapter call fails
- **THEN** the failure SHALL be observable with error category, operation name, and correlation context

### Requirement: Backward compatibility strategy
Breaking changes SHALL include an explicit compatibility and migration strategy. Silent breaking changes are prohibited.

#### Scenario: Public contract changes
- **WHEN** a public API, event schema, command schema, or persisted format changes incompatibly
- **THEN** the OpenSpec change SHALL document the migration path and rollback strategy

#### Scenario: Backward compatibility cannot be preserved
- **WHEN** backward compatibility cannot be preserved
- **THEN** the change SHALL state the breaking impact and require explicit approval through the OpenSpec review process

### Requirement: Constitution amendments
SPEC-000 SHALL be immutable by default. Any change to constitution requirements MUST be proposed as a dedicated OpenSpec change and MUST preserve the rationale for the previous rule.

#### Scenario: Contributor wants to alter a constitution rule
- **WHEN** a contributor wants to add, remove, or modify a constitution requirement
- **THEN** they SHALL create a dedicated OpenSpec change that explains why the amendment is necessary

#### Scenario: Constitution amendment is archived
- **WHEN** a constitution amendment is archived
- **THEN** the final spec history SHALL retain enough context to audit what changed and why
