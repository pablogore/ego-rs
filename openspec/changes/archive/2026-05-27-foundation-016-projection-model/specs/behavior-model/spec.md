## ADDED Requirements

### Requirement: Projection Model authority boundary

Behavior execution semantics SHALL remain governed by Behavior Model while read materialization semantics SHALL be governed by Projection Model.

The Behavior Model SHALL remain authoritative for:
- HOW behavior executes,
- command handling semantics,
- event handling semantics,
- state transition semantics,
- lifecycle semantics,
- read-only behavior semantics,
- failure behavior semantics.

The Projection Model SHALL remain authoritative for:
- HOW behavior becomes materialized as read knowledge,
- read-side materialization semantics,
- projection lifecycle semantics,
- replay-safe projections,
- projection consistency expectations,
- projection failure semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Behavior execution evaluated
- **WHEN** behavior execution semantics are evaluated
- **THEN** Behavior Model SHALL govern HOW behavior executes

#### Scenario: Projection materialization evaluated
- **WHEN** read materialization semantics are evaluated
- **THEN** Projection Model SHALL govern HOW behavior becomes materialized as read knowledge

#### Scenario: Authority overlap detected
- **WHEN** Behavior Model and Projection Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
