## ADDED Requirements

### Requirement: Interaction model cross-reference

Participant interaction behavior across architectural boundaries SHALL be constitutionally governed by the Interaction Model.

Architecture Governance SHALL remain authoritative for:
- layer boundaries,
- dependency direction,
- architectural placement,
- port and adapter boundaries.

Interaction Model SHALL remain authoritative for:
- participant interaction semantics,
- interaction expectations,
- response expectations,
- interaction sequencing semantics.

The ownership model MUST remain explicit and non-overlapping. Participant interaction across architectural boundaries SHALL comply with both governing specs without duplication or ambiguity.

#### Scenario: Participant interaction across layers
- **WHEN** participant interaction crosses an architectural layer boundary
- **THEN** the interaction SHALL comply with both Architecture Governance layer rules and Interaction Model governance

#### Scenario: Cross-layer interaction governance
- **WHEN** participant interaction crosses an architectural boundary
- **THEN** Architecture Governance SHALL govern architectural correctness and Interaction Model SHALL govern participant interaction semantics