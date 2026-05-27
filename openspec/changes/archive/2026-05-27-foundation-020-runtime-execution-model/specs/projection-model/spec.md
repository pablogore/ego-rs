## ADDED Requirements

### Requirement: Runtime Execution Model authority boundary

Read materialization semantics SHALL remain governed by Projection Model while governed execution semantics SHALL be governed by Runtime Execution Model.

The Projection Model SHALL remain authoritative for:
- HOW behavior becomes materialized as read knowledge,
- read-side materialization semantics,
- projection lifecycle semantics,
- replay-safe projections,
- projection consistency expectations,
- projection failure semantics.

The Runtime Execution Model SHALL remain authoritative for:
- HOW governed execution actually happens,
- execution boundary semantics,
- execution isolation semantics,
- execution ordering semantics,
- execution consistency expectations,
- execution retry semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Projection materialization evaluated
- **WHEN** read materialization semantics are evaluated
- **THEN** Projection Model SHALL govern HOW behavior becomes materialized as read knowledge

#### Scenario: Runtime execution evaluated
- **WHEN** governed execution semantics are evaluated
- **THEN** Runtime Execution Model SHALL govern HOW governed execution actually happens

#### Scenario: Authority overlap detected
- **WHEN** Projection Model and Runtime Execution Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
