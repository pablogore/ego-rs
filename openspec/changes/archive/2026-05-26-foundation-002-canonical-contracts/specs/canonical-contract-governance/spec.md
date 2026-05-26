## ADDED Requirements

### Requirement: Protobuf-first contract governance
Project contracts SHALL be governed as protobuf-first artifacts before runtime implementation. Contract semantics MUST be reviewed through OpenSpec before they are used to drive generated code, services, endpoints, or runtime behavior.

#### Scenario: Runtime behavior needs a new public contract
- **WHEN** a change needs a new public command, query, event, or cross-boundary interface
- **THEN** the change SHALL define the contract governance impact before implementation begins

#### Scenario: Runtime code changes contract semantics
- **WHEN** runtime code would add, remove, or reinterpret public contract semantics
- **THEN** the change SHALL include contract review before the runtime change is accepted

### Requirement: Contract-first development
Contract changes SHALL be proposed, reviewed, and validated before implementation work depends on them. Implementation tasks MUST NOT introduce public contract semantics that are absent from accepted contract artifacts.

#### Scenario: Implementation task depends on a contract
- **WHEN** an implementation task uses a public contract
- **THEN** the contract SHALL already be represented by an accepted OpenSpec change or existing governed contract

#### Scenario: Contract and implementation are proposed together
- **WHEN** one OpenSpec change includes both contract and implementation work
- **THEN** contract tasks SHALL precede implementation tasks and validation SHALL include contract governance checks

### Requirement: Canonical contracts repository structure
Contract artifacts SHALL live under a canonical `contracts/` tree organized for versioned protobuf governance. The structure MUST separate versioned contracts from generated Rust code and runtime service implementation.

#### Scenario: Contract artifact is added
- **WHEN** a protobuf contract artifact is introduced
- **THEN** it SHALL be placed under the canonical `contracts/` tree

#### Scenario: Generated code is produced
- **WHEN** Rust code is generated from contracts
- **THEN** the generated output SHALL be kept separate from canonical contract source artifacts

### Requirement: Versioning starts at v1
Contracts SHALL be versioned from first introduction, starting at `v1`. Incompatible contract lines MUST use a new major version and include migration guidance.

#### Scenario: First contract version is introduced
- **WHEN** a new contract family is added
- **THEN** it SHALL be introduced under a `v1` version path or equivalent governed version marker

#### Scenario: Incompatible change is proposed
- **WHEN** a contract change is not backward compatible
- **THEN** it SHALL introduce a new major version and document migration impact

### Requirement: Buf lint and breaking checks
Contract changes SHALL be validated with Buf linting and breaking-change checks. Missing configuration, missing prior state, or unavailable validation inputs MUST fail closed.

#### Scenario: Contract change is validated
- **WHEN** a contract change is submitted
- **THEN** Buf lint and breaking-change checks SHALL pass before the change is accepted

#### Scenario: Breaking-check baseline is unavailable
- **WHEN** the breaking-change check cannot compare against the required prior contract state
- **THEN** validation SHALL fail instead of assuming compatibility

### Requirement: prost/tonic generation policy
Rust contract code SHALL be generated from accepted protobuf contracts through governed prost/tonic configuration. Generated code MUST NOT be edited by hand, and generation policy MUST remain separate from runtime service behavior.

#### Scenario: Generated Rust contract code is updated
- **WHEN** generated Rust contract code changes
- **THEN** the change SHALL trace back to accepted contract source changes and generation configuration

#### Scenario: Developer edits generated code directly
- **WHEN** generated prost/tonic output is modified by hand
- **THEN** the change SHALL be rejected unless it is replaced by a source contract or generation configuration change

### Requirement: CQRS contract taxonomy
Contracts SHALL be classified as commands, queries, or events. Commands represent requested state changes, queries represent reads, and events represent historical facts.

#### Scenario: Command contract is reviewed
- **WHEN** a contract represents a request to change state
- **THEN** it SHALL be classified and reviewed as a command

#### Scenario: Event contract is reviewed
- **WHEN** a contract represents a fact that already occurred
- **THEN** it SHALL be classified and reviewed as an event

### Requirement: Backward compatibility rules
Contract changes SHALL preserve backward compatibility unless an explicit new major version and migration strategy are provided. Silent breaking changes are prohibited.

#### Scenario: Compatible field addition is proposed
- **WHEN** a contract change adds compatible schema surface
- **THEN** it SHALL remain in the current major version and pass breaking-change validation

#### Scenario: Breaking contract change is proposed
- **WHEN** a contract change removes or reinterprets existing public schema semantics
- **THEN** it SHALL require a new major version and migration guidance

### Requirement: Contract ownership and review
Each contract area SHALL have explicit ownership and review expectations. Review MUST cover versioning, compatibility, CQRS classification, Buf validation, generation impact, and testing strategy.

#### Scenario: Contract area is introduced
- **WHEN** a new contract area is created
- **THEN** ownership or reviewer responsibility SHALL be declared

#### Scenario: Contract change is reviewed
- **WHEN** a contract change is reviewed
- **THEN** reviewers SHALL verify versioning, compatibility, CQRS taxonomy, validation, generation impact, and tests

### Requirement: Contract testing governance
Contract tests SHALL validate schema compatibility and generated-code expectations without requiring real infrastructure. Tests MUST comply with testing governance and use mocks or local deterministic fixtures for external dependencies.

#### Scenario: Contract compatibility test runs
- **WHEN** contract tests validate compatibility
- **THEN** they SHALL run without live brokers, databases, network services, or external infrastructure

#### Scenario: Generated code expectation is tested
- **WHEN** generated Rust contract code is validated
- **THEN** the test SHALL verify generation output deterministically from contract source and configuration
