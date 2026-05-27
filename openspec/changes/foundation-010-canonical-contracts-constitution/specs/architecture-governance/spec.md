## ADDED Requirements

### Requirement: Canonical contract governance for architectural ports

Architectural boundary contracts defined at layer boundaries (port and adapter boundaries) SHALL be governed by the Canonical Contracts Constitution (`specs/canonical-contracts-constitution/spec.md`). Port boundaries between hexagonal layers SHALL comply with canonical contract semantics, compatibility governance, and evolution governance as defined by the Canonical Contracts Constitution.

#### Scenario: Architectural boundary contract
- **WHEN** an architectural boundary contract is defined at a layer boundary
- **THEN** the boundary contract SHALL be governed by the Canonical Contracts Constitution

#### Scenario: Architectural contract evolution
- **WHEN** an architectural boundary contract evolves
- **THEN** the evolution SHALL comply with Canonical Contracts Constitution compatibility and evolution governance
