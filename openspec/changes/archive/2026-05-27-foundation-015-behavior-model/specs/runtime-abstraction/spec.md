## ADDED Requirements

### Requirement: Behavior Model authority boundary

Runtime execution semantics SHALL remain governed by Runtime Abstraction while behavior execution semantics SHALL be governed by Behavior Model.

The Runtime Abstraction SHALL remain authoritative for:
- execution lifecycle (Pending, Running, Completed, Failed, Cancelled, TimedOut),
- execution boundaries (isolation, cancellation, timeout, error scope),
- runtime capability model (mandatory, optional, forbidden capabilities),
- runtime SPI ports (Execution, Clock, Context, Backpressure),
- concurrency model semantics,
- failure model (fail-closed),
- ordering and isolation guarantees,
- capability inflation protection.

The Behavior Model SHALL remain authoritative for:
- command handling semantics,
- event handling semantics,
- state transition semantics,
- lifecycle semantics,
- read-only behavior semantics,
- failure behavior semantics,
- behavior observability semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Runtime execution evaluated
- **WHEN** runtime execution semantics are evaluated
- **THEN** Runtime Abstraction SHALL govern HOW execution is implemented

#### Scenario: Behavior execution evaluated
- **WHEN** behavior execution semantics are evaluated
- **THEN** Behavior Model SHALL govern HOW behavior executes

#### Scenario: Authority overlap detected
- **WHEN** Runtime Abstraction and Behavior Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation