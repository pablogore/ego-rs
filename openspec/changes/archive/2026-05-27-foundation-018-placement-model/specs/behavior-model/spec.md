## ADDED Requirements

### Requirement: Placement Model authority boundary

Behavior execution semantics SHALL remain governed by Behavior Model while ownership-in-space semantics SHALL be governed by Placement Model.

The Behavior Model SHALL remain authoritative for:
- HOW behavior executes,
- command handling semantics,
- event handling semantics,
- state transition semantics,
- lifecycle semantics,
- read-only behavior semantics,
- failure behavior semantics.

The Placement Model SHALL remain authoritative for:
- HOW execution ownership exists in space,
- ownership semantics,
- locality semantics,
- execution location abstraction,
- mobility semantics,
- placement lifecycle semantics,
- placement consistency expectations.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Behavior execution evaluated
- **WHEN** behavior execution semantics are evaluated
- **THEN** Behavior Model SHALL govern HOW behavior executes

#### Scenario: Placement ownership evaluated
- **WHEN** ownership-in-space semantics are evaluated
- **THEN** Placement Model SHALL govern HOW execution ownership exists in space

#### Scenario: Authority overlap detected
- **WHEN** Behavior Model and Placement Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
