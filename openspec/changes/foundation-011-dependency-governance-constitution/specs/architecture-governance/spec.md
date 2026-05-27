## ADDED Requirements

### Requirement: Dependency governance cross-reference

Architectural layer dependency direction SHALL be governed by both Architecture Governance and Dependency Governance Constitution (`specs/dependency-governance-constitution/spec.md`). The Dependency Governance Constitution SHALL be treated as the governing spec for dependency direction rules, forbidden dependencies, hidden coupling prevention, version governance, and enforcement classification.

#### Scenario: Dependency direction evaluated against dependency governance
- **WHEN** dependency direction is evaluated
- **THEN** it SHALL comply with both Architecture Governance and Dependency Governance Constitution

#### Scenario: Dependency violation classification
- **WHEN** a dependency direction violation is detected
- **THEN** classification SHALL follow Dependency Governance Constitution severity levels
