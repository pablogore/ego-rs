## ADDED Requirements

### Requirement: Lifecycle Model authority boundary

Behavior execution semantics SHALL remain governed by Behavior Model while lifecycle evolution semantics SHALL be governed by Lifecycle Model.

The Behavior Model SHALL remain authoritative for:
- HOW behavior executes,
- command handling semantics,
- event handling semantics,
- state transition semantics,
- read-only behavior semantics,
- failure behavior semantics.

The Lifecycle Model SHALL remain authoritative for:
- HOW governed things evolve through lifecycle,
- activation semantics,
- suspension semantics,
- recovery semantics,
- restoration semantics,
- lifecycle transition semantics,
- lifecycle consistency expectations.

Authority ownership SHALL remain explicit and non-overlapping.

Canonical FOUNDATION-015 Behavior Model wording granting lifecycle semantics ownership to Behavior Model SHALL be harmonized at FOUNDATION-019 archive time to remove lifecycle evolution semantics from Behavior Model authority.

#### Scenario: Behavior execution evaluated
- **WHEN** behavior execution semantics are evaluated
- **THEN** Behavior Model SHALL govern HOW behavior executes

#### Scenario: Lifecycle evolution evaluated
- **WHEN** lifecycle evolution semantics are evaluated
- **THEN** Lifecycle Model SHALL govern HOW governed things evolve through lifecycle

#### Scenario: Authority overlap detected
- **WHEN** Behavior Model and Lifecycle Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
