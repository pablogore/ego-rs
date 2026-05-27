## ADDED Requirements

### Requirement: Runtime Execution Model authority boundary

Ownership-in-space semantics SHALL remain governed by Placement Model while governed execution semantics SHALL be governed by Runtime Execution Model.

The Placement Model SHALL remain authoritative for:
- HOW execution ownership exists in space,
- ownership semantics,
- locality semantics,
- execution location abstraction,
- mobility semantics,
- placement lifecycle semantics,
- placement consistency expectations.

The Runtime Execution Model SHALL remain authoritative for:
- HOW governed execution actually happens,
- execution boundary semantics,
- execution isolation semantics,
- execution ordering semantics,
- execution consistency expectations,
- execution retry semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Ownership-in-space evaluated
- **WHEN** ownership-in-space semantics are evaluated
- **THEN** Placement Model SHALL govern HOW execution ownership exists in space

#### Scenario: Runtime execution evaluated
- **WHEN** governed execution semantics are evaluated
- **THEN** Runtime Execution Model SHALL govern HOW governed execution actually happens

#### Scenario: Authority overlap detected
- **WHEN** Placement Model and Runtime Execution Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
