## ADDED Requirements

### Requirement: Placement Model authority boundary

Durable truth preservation semantics SHALL remain governed by Persistence Model while ownership-in-space semantics SHALL be governed by Placement Model.

The Persistence Model SHALL remain authoritative for:
- HOW durable truth is preserved and restored,
- durable state semantics,
- persistence lifecycle semantics,
- replay-safe persistence,
- snapshot semantics,
- restoration semantics,
- lineage trustworthiness.

The Placement Model SHALL remain authoritative for:
- HOW execution ownership exists in space,
- ownership semantics,
- locality semantics,
- execution location abstraction,
- mobility semantics,
- placement lifecycle semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Persistence truth evaluated
- **WHEN** durable truth preservation semantics are evaluated
- **THEN** Persistence Model SHALL govern HOW durable truth is preserved and restored

#### Scenario: Placement ownership evaluated
- **WHEN** ownership-in-space semantics are evaluated
- **THEN** Placement Model SHALL govern HOW execution ownership exists in space

#### Scenario: Authority overlap detected
- **WHEN** Persistence Model and Placement Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
