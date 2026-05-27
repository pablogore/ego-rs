## ADDED Requirements

### Requirement: Service boundary cross-reference

Service interaction boundaries crossing architectural layers SHALL be constitutionally governed by BOTH Architecture Governance and the Service Contract Model.

Architecture Governance SHALL remain authoritative for:
- layer boundaries,
- dependency direction,
- architectural placement,
- port and adapter boundaries.

Service Contract Model SHALL remain authoritative for:
- service interaction semantics,
- endpoint contract semantics,
- exposure descriptor semantics,
- service policy attachment,
- deterministic interaction expectations.

The ownership model MUST remain explicit and non-overlapping. Service interaction boundaries SHALL comply with both governing specs without duplication or ambiguity.

#### Scenario: Service boundary across layers
- **WHEN** a service interaction crosses an architectural layer boundary
- **THEN** the interaction SHALL comply with both Architecture Governance layer rules and Service Contract Model governance

#### Scenario: Cross-layer service interaction governance
- **WHEN** a service interaction crosses an architectural boundary
- **THEN** Architecture Governance SHALL govern architectural correctness and Service Contract Model SHALL govern service interaction semantics
