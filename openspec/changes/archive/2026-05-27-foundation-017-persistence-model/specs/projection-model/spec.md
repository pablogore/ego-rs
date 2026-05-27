## ADDED Requirements

### Requirement: Persistence Model authority boundary

Read materialization semantics SHALL remain governed by Projection Model while durable truth preservation semantics SHALL be governed by Persistence Model.

The Projection Model SHALL remain authoritative for:
- HOW behavior becomes materialized as read knowledge,
- read-side materialization semantics,
- projection lifecycle semantics,
- replay-safe projections,
- projection consistency expectations,
- projection failure semantics.

The Persistence Model SHALL remain authoritative for:
- HOW durable truth is preserved and restored,
- durable state semantics,
- persistence lifecycle semantics,
- replay-safe persistence,
- snapshot semantics,
- restoration semantics,
- lineage trustworthiness.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Projection materialization evaluated
- **WHEN** read materialization semantics are evaluated
- **THEN** Projection Model SHALL govern HOW behavior becomes materialized as read knowledge

#### Scenario: Persistence truth evaluated
- **WHEN** durable truth preservation semantics are evaluated
- **THEN** Persistence Model SHALL govern HOW durable truth is preserved and restored

#### Scenario: Authority overlap detected
- **WHEN** Projection Model and Persistence Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
