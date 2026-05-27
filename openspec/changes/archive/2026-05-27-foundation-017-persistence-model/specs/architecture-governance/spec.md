## ADDED Requirements

### Requirement: Persistence behavior across architectural boundaries

Persistence behavior across architectural boundaries SHALL comply with both Architecture Governance and Persistence Model governance.

Architecture Governance SHALL remain authoritative for:
- hexagonal architecture layers (domain, application, infrastructure, transport),
- dependency direction (transport → application → domain, infrastructure → domain),
- ports and adapters pattern,
- SOLID principles compliance,
- cross-layer import enforcement.

Persistence Model SHALL remain authoritative for:
- HOW durable truth is preserved and restored,
- durable state semantics,
- persistence lifecycle semantics,
- replay-safe persistence,
- snapshot semantics,
- restoration semantics,
- lineage trustworthiness.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Architectural boundary evaluation
- **WHEN** persistence behavior is evaluated at an architectural boundary
- **THEN** both Architecture Governance and Persistence Model governance SHALL apply within their respective authority scopes

#### Scenario: Layer persistence governance
- **WHEN** persistence behavior executes within a governed architectural layer
- **THEN** Persistence Model SHALL govern how durable truth is preserved while Architecture Governance SHALL govern layer structure and dependency direction

#### Scenario: Authority overlap detected
- **WHEN** Architecture Governance and Persistence Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
