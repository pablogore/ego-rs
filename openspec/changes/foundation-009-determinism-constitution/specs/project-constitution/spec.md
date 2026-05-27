## MODIFIED Requirements

### Requirement: Deterministic-first behavior

Project behavior SHALL be deterministic by default. Given the same inputs and persisted state, domain and application logic MUST produce the same outputs without relying on hidden randomness, wall-clock time, external services, or mutable global state. Deterministic governance is constitutionally defined in the Determinism Constitution (`specs/determinism-constitution/spec.md`), which SHALL be treated as the governing spec for all determinism-related requirements.

#### Scenario: Determinism governance cross-reference
- **WHEN** determinism governance requirements are evaluated
- **THEN** the Determinism Constitution SHALL be the governing spec for determinism requirements, in addition to the requirements defined here

#### Scenario: Domain logic receives identical inputs
- **WHEN** domain logic is executed twice with the same inputs and state
- **THEN** it SHALL produce equivalent outputs and events

#### Scenario: Non-deterministic input is required
- **WHEN** a workflow needs time, randomness, or external data
- **THEN** that input SHALL be provided through an explicit port or parameter
