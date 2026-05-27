## ADDED Requirements

### Requirement: Projection Model authority boundary

Runtime execution implementation SHALL remain governed by Runtime Abstraction while projection semantics SHALL be governed by Projection Model.

The Runtime Abstraction SHALL remain authoritative for:
- execution lifecycle (Pending, Running, Completed, Failed, Cancelled, TimedOut),
- execution boundaries (isolation, cancellation, timeout, error scope),
- runtime capability model (mandatory, optional, forbidden capabilities),
- runtime SPI ports (Execution, Clock, Context, Backpressure),
- concurrency model semantics,
- failure model (fail-closed),
- ordering and isolation guarantees,
- capability inflation protection.

The Projection Model SHALL remain authoritative for:
- HOW behavior becomes materialized as read knowledge,
- read-side materialization semantics,
- projection lifecycle semantics,
- replay-safe projections,
- projection consistency expectations,
- projection failure semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Runtime execution evaluated
- **WHEN** runtime execution semantics are evaluated
- **THEN** Runtime Abstraction SHALL govern HOW execution is implemented

#### Scenario: Projection semantics evaluated
- **WHEN** projection semantics are evaluated
- **THEN** Projection Model SHALL govern HOW behavior becomes materialized as read knowledge

#### Scenario: Authority overlap detected
- **WHEN** Runtime Abstraction and Projection Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
