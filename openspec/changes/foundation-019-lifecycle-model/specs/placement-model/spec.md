## ADDED Requirements

### Requirement: Lifecycle Model authority boundary

Ownership-in-space semantics SHALL remain governed by Placement Model while lifecycle evolution semantics SHALL be governed by Lifecycle Model.

The Placement Model SHALL remain authoritative for:
- HOW execution ownership exists in space,
- ownership semantics,
- locality semantics,
- execution location abstraction,
- mobility semantics,
- placement lifecycle semantics,
- placement consistency expectations.

The Lifecycle Model SHALL remain authoritative for:
- HOW governed things evolve through lifecycle,
- activation semantics,
- suspension semantics,
- recovery semantics,
- restoration semantics,
- lifecycle transition semantics,
- lifecycle consistency expectations.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Ownership-in-space evaluated
- **WHEN** ownership-in-space semantics are evaluated
- **THEN** Placement Model SHALL govern HOW execution ownership exists in space

#### Scenario: Lifecycle evolution evaluated
- **WHEN** lifecycle evolution semantics are evaluated
- **THEN** Lifecycle Model SHALL govern HOW governed things evolve through lifecycle

#### Scenario: Authority overlap detected
- **WHEN** Placement Model and Lifecycle Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
