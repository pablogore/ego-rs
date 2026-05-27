## ADDED Requirements

### Requirement: Transport binding cross-reference

Transport exposure of service contracts across architectural boundaries SHALL be constitutionally governed by the Transport Binding Model.

Architecture Governance SHALL remain authoritative for:
- layer boundaries,
- dependency direction,
- architectural placement,
- port and adapter boundaries.

Transport Binding Model SHALL remain authoritative for:
- transport exposure semantics,
- endpoint exposure binding,
- exposure descriptor binding,
- transport policy attachment,
- deterministic transport exposure behavior.

The ownership model MUST remain explicit and non-overlapping. Transport exposure across architectural boundaries SHALL comply with both governing specs without duplication or ambiguity.

#### Scenario: Cross-layer transport exposure governance
- **WHEN** a service contract is transport-exposed across an architectural boundary
- **THEN** Architecture Governance SHALL govern architectural correctness and Transport Binding Model SHALL govern exposure binding semantics