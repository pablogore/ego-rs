## ADDED Requirements

### Requirement: Projection behavior across architectural boundaries

Projection behavior across architectural boundaries SHALL comply with both Architecture Governance and Projection Model governance.

Architecture Governance SHALL remain authoritative for:
- hexagonal architecture layers (domain, application, infrastructure, transport),
- dependency direction (transport → application → domain, infrastructure → domain),
- ports and adapters pattern,
- SOLID principles compliance,
- cross-layer import enforcement.

Projection Model SHALL remain authoritative for:
- HOW behavior becomes materialized as read knowledge,
- read-side materialization semantics,
- projection lifecycle semantics,
- replay-safe projections,
- projection consistency expectations,
- projection failure semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Architectural boundary evaluation
- **WHEN** projection behavior is evaluated at an architectural boundary
- **THEN** both Architecture Governance and Projection Model governance SHALL apply within their respective authority scopes

#### Scenario: Layer projection governance
- **WHEN** projection executes within a governed architectural layer
- **THEN** Projection Model SHALL govern how behavior is materialized while Architecture Governance SHALL govern layer structure and dependency direction

#### Scenario: Authority overlap detected
- **WHEN** Architecture Governance and Projection Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
