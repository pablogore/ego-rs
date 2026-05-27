## ADDED Requirements

### Requirement: Placement Model authority boundary

Read materialization semantics SHALL remain governed by Projection Model while ownership-in-space semantics SHALL be governed by Placement Model.

The Projection Model SHALL remain authoritative for:
- HOW behavior becomes materialized as read knowledge,
- read-side materialization semantics,
- projection lifecycle semantics,
- replay-safe projections,
- projection consistency expectations,
- projection failure semantics.

The Placement Model SHALL remain authoritative for:
- HOW execution ownership exists in space,
- ownership semantics,
- locality semantics,
- execution location abstraction,
- mobility semantics,
- placement lifecycle semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Projection materialization evaluated
- **WHEN** read materialization semantics are evaluated
- **THEN** Projection Model SHALL govern HOW behavior becomes materialized as read knowledge

#### Scenario: Placement ownership evaluated
- **WHEN** ownership-in-space semantics are evaluated
- **THEN** Placement Model SHALL govern HOW execution ownership exists in space

#### Scenario: Authority overlap detected
- **WHEN** Projection Model and Placement Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
