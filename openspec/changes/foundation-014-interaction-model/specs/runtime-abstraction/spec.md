## ADDED Requirements

### Requirement: Runtime-mediated interaction governance

Runtime-mediated participant interaction SHALL be governed by the Interaction Model (`specs/interaction-model/spec.md`).

Runtime Abstraction SHALL remain authoritative for:
- runtime capability semantics,
- runtime execution expectations,
- capability mediation.

Interaction Model SHALL remain authoritative for:
- participant interaction semantics,
- interaction expectations,
- response expectations,
- interaction sequencing semantics.

Authority ownership MUST remain explicit and non-overlapping.

#### Scenario: Runtime-mediated participant interaction
- **WHEN** runtime capabilities mediate participant interaction
- **THEN** Runtime Abstraction SHALL govern runtime execution semantics and Interaction Model SHALL govern participant interaction semantics