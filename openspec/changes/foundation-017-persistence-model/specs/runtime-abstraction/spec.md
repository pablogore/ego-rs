## ADDED Requirements

### Requirement: Persistence Model authority boundary

Runtime execution implementation SHALL remain governed by Runtime Abstraction while persistence semantics SHALL be governed by Persistence Model.

The Runtime Abstraction SHALL remain authoritative for:
- execution lifecycle (Pending, Running, Completed, Failed, Cancelled, TimedOut),
- execution boundaries (isolation, cancellation, timeout, error scope),
- runtime capability model (mandatory, optional, forbidden capabilities),
- runtime SPI ports (Execution, Clock, Context, Backpressure),
- concurrency model semantics,
- failure model (fail-closed),
- ordering and isolation guarantees,
- capability inflation protection.

The Persistence Model SHALL remain authoritative for:
- HOW durable truth is preserved and restored,
- durable state semantics,
- persistence lifecycle semantics,
- replay-safe persistence,
- snapshot semantics,
- restoration semantics,
- lineage trustworthiness.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Runtime execution evaluated
- **WHEN** runtime execution semantics are evaluated
- **THEN** Runtime Abstraction SHALL govern HOW execution is implemented

#### Scenario: Persistence semantics evaluated
- **WHEN** persistence semantics are evaluated
- **THEN** Persistence Model SHALL govern HOW durable truth is preserved and restored

#### Scenario: Authority overlap detected
- **WHEN** Runtime Abstraction and Persistence Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
