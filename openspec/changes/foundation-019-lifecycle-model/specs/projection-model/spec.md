## ADDED Requirements

### Requirement: Lifecycle Model authority boundary

Read materialization semantics SHALL remain governed by Projection Model while lifecycle evolution semantics SHALL be governed by Lifecycle Model.

The Projection Model SHALL remain authoritative for:
- HOW behavior becomes materialized as read knowledge,
- read-side materialization semantics,
- projection lifecycle semantics,
- replay-safe projections,
- projection consistency expectations,
- projection failure semantics.

The Lifecycle Model SHALL remain authoritative for:
- HOW governed things evolve through lifecycle,
- activation semantics,
- suspension semantics,
- recovery semantics,
- restoration semantics,
- lifecycle transition semantics,
- lifecycle consistency expectations.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Projection materialization evaluated
- **WHEN** read materialization semantics are evaluated
- **THEN** Projection Model SHALL govern HOW behavior becomes materialized as read knowledge

#### Scenario: Lifecycle evolution evaluated
- **WHEN** lifecycle evolution semantics are evaluated
- **THEN** Lifecycle Model SHALL govern HOW governed things evolve through lifecycle

#### Scenario: Authority overlap detected
- **WHEN** Projection Model and Lifecycle Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
