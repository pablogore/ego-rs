## ADDED Requirements

### Requirement: Runtime Execution Model authority boundary

Durable truth semantics SHALL remain governed by Persistence Model while governed execution semantics SHALL be governed by Runtime Execution Model.

The Persistence Model SHALL remain authoritative for:
- HOW durable truth is preserved and restored,
- durable state semantics,
- persistence lifecycle semantics,
- replay-safe persistence,
- snapshot semantics,
- restoration semantics,
- lineage trustworthiness.

The Runtime Execution Model SHALL remain authoritative for:
- HOW governed execution actually happens,
- execution boundary semantics,
- execution isolation semantics,
- execution ordering semantics,
- execution consistency expectations,
- execution retry semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Durable truth evaluated
- **WHEN** durable truth semantics are evaluated
- **THEN** Persistence Model SHALL govern HOW durable truth is preserved and restored

#### Scenario: Runtime execution evaluated
- **WHEN** governed execution semantics are evaluated
- **THEN** Runtime Execution Model SHALL govern HOW governed execution actually happens

#### Scenario: Authority overlap detected
- **WHEN** Persistence Model and Runtime Execution Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
