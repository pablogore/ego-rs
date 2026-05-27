## ADDED Requirements

### Requirement: Lifecycle Model authority boundary

Durable truth semantics SHALL remain governed by Persistence Model while lifecycle evolution semantics SHALL be governed by Lifecycle Model.

The Persistence Model SHALL remain authoritative for:
- HOW durable truth is preserved and restored,
- durable state semantics,
- persistence lifecycle semantics,
- replay-safe persistence,
- snapshot semantics,
- restoration semantics,
- lineage trustworthiness.

The Lifecycle Model SHALL remain authoritative for:
- HOW governed things evolve through lifecycle,
- activation semantics,
- suspension semantics,
- recovery semantics,
- restoration semantics,
- lifecycle transition semantics,
- lifecycle consistency expectations.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Durable truth evaluated
- **WHEN** durable truth semantics are evaluated
- **THEN** Persistence Model SHALL govern HOW durable truth is preserved and restored

#### Scenario: Lifecycle evolution evaluated
- **WHEN** lifecycle evolution semantics are evaluated
- **THEN** Lifecycle Model SHALL govern HOW governed things evolve through lifecycle

#### Scenario: Authority overlap detected
- **WHEN** Persistence Model and Lifecycle Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
