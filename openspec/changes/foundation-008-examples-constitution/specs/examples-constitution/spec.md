## ADDED Requirements

### Requirement: Mandatory Examples Policy

Examples SHALL exist for every major runtime capability, every major integration, every platform vertical, and every operator-facing workflow. Examples MUST evolve with the specs they demonstrate. A change that breaks an existing example without updating it SHALL be considered incomplete.

Examples SHALL be considered part of the product and onboarding experience, not optional documentation. Examples SHALL follow the same engineering standards as production code.

#### Scenario: New capability is introduced without example
- **WHEN** a new runtime capability, integration, platform vertical, or operator-facing workflow is added
- **THEN** a corresponding example SHALL be created in the same OpenSpec change

#### Scenario: Spec requirement changes without example update
- **WHEN** an existing spec requirement changes
- **THEN** its corresponding example SHALL be updated in the same change

#### Scenario: Breaking change without updated example
- **WHEN** a spec change breaks an existing example and the example is not updated
- **THEN** the change SHALL be rejected as incomplete

### Requirement: Example Categories

Examples SHALL be organized into the following mandatory categories:

1. **Foundational** — platform setup, bootstrap, configuration
2. **Runtime** — execution lifecycle, persistence, CQRS, event-driven flows
3. **Integrations** — external system adapters (databases, messaging, observability, auth)
4. **UX** — user-facing interfaces (TUI, web)
5. **Real-world** — production-style scenarios combining multiple capabilities

#### Scenario: New example is assigned a category
- **WHEN** a contributor creates a new example
- **THEN** it SHALL be placed in the category that matches its primary concern

#### Scenario: New category is proposed
- **WHEN** a contributor proposes a new example category
- **THEN** it SHALL require a constitutional amendment to this spec

### Requirement: Architecture Compliance

Example code SHALL comply with the Architecture Governance spec.

#### Scenario: Example fails architecture validation
- **WHEN** an example fails architecture validation under Architecture Governance rules
- **THEN** the violation SHALL be treated identically to a production code violation

### Requirement: Testing Requirements

Example testing SHALL comply with the Testing Governance spec.

#### Scenario: Example fails testing validation
- **WHEN** an example fails testing validation under Testing Governance rules
- **THEN** the failure SHALL be treated as a validation failure

### Requirement: Documentation Requirements

Every example SHALL include documentation describing its purpose, execution instructions, expected behavior, observability expectations, and the governing specification it demonstrates. Documentation SHALL be onboarding-friendly.

#### Scenario: Example lacks documentation
- **WHEN** a new example is added without accompanying documentation
- **THEN** the documentation gap SHALL block the change

#### Scenario: Example documentation is incomplete
- **WHEN** an example is documented but the documentation omits purpose, execution instructions, expected behavior, observability expectations, or governing specification reference
- **THEN** the documentation SHALL be considered incomplete

### Requirement: Repository discoverability

Examples SHALL reside in the canonical examples location defined by this constitution. The canonical location MAY currently be `examples/`. Examples SHALL be organized in subdirectories corresponding to mandatory categories. Naming SHALL use kebab-case. The structure SHALL NOT be modified without a constitutional amendment.

#### Scenario: Example is placed outside canonical location
- **WHEN** an example is added outside the canonical examples location
- **THEN** the example SHALL be relocated

#### Scenario: Category directory is missing
- **WHEN** a mandatory category directory is absent from the canonical location
- **THEN** the repository SHALL be considered non-conformant

### Requirement: CI governance

Deterministic unit validation of examples SHALL block pull requests. Integration validation MAY run in a separate stage and SHALL NOT block primary validation, but SHALL block release and full validation pipelines.

#### Scenario: Example compilation or unit test fails
- **WHEN** an example fails to compile or a unit test fails in the primary pipeline
- **THEN** the pull request SHALL be blocked

#### Scenario: Integration example validation fails
- **WHEN** an integration example fails in the full validation pipeline
- **THEN** the release SHALL be blocked

### Requirement: Synchronization with governing specs

Examples SHALL remain synchronized with the governing capabilities and specifications they demonstrate. A change that modifies a spec without updating its corresponding example SHALL be considered incomplete.

#### Scenario: Example is out of sync with its governing spec
- **WHEN** a spec is modified and the corresponding example is not updated
- **THEN** the change SHALL be considered incomplete

### Requirement: Executable documentation

Examples SHALL be treated as executable documentation and onboarding artifacts. Examples SHALL demonstrate platform capabilities through runnable, architecture-compliant implementations. The purpose of an example is to teach a capability through executable code, not to serve as a toy or throwaway demonstration.

#### Scenario: Developer learns a capability
- **WHEN** a developer or contributor needs to understand a platform capability
- **THEN** a runnable example SHALL exist that demonstrates the capability according to its governing spec

#### Scenario: Example diverges from documented behavior
- **WHEN** an example behavior diverges from its governing specification
- **THEN** the example SHALL be considered invalid and SHALL be corrected

### Requirement: Governance enforcement severity

Violations SHALL be classified using constitutional severity. The following severity levels SHALL apply:

- **Constitutional violation**: An example violates architectural governance, testing governance, or a constitutional invariant.
- **Incomplete change**: A required example is absent, not updated, or its documentation is missing.
- **Validation failure**: An example fails deterministic validation (compilation, unit test, determinism check).

#### Scenario: Architecture boundary violation
- **WHEN** an example violates architectural governance
- **THEN** the violation SHALL be treated as a constitutional violation

#### Scenario: Missing mandatory example
- **WHEN** a required example is absent or not updated
- **THEN** the change SHALL be treated as incomplete

#### Scenario: Example compilation or validation failure
- **WHEN** an example fails deterministic validation
- **THEN** the failure SHALL be treated as a validation failure

### Requirement: Example ownership

Every example SHALL explicitly reference the governing capability or specification it demonstrates. Examples MUST NOT become orphaned onboarding artifacts.

#### Scenario: Example without governing capability
- **WHEN** an example does not reference a governing specification or capability
- **THEN** the example SHALL be considered invalid

#### Scenario: Capability evolves
- **WHEN** a governing specification changes
- **THEN** the corresponding example SHALL evolve with it
