## ADDED Requirements

### Requirement: Placement Model authority boundary

Runtime execution implementation SHALL remain governed by Runtime Abstraction while ownership-in-space semantics SHALL be governed by Placement Model.

The Runtime Abstraction SHALL remain authoritative for:
- execution lifecycle (Pending, Running, Completed, Failed, Cancelled, TimedOut),
- execution boundaries (isolation, cancellation, timeout, error scope),
- runtime capability model (mandatory, optional, forbidden capabilities),
- runtime SPI ports (Execution, Clock, Context, Backpressure),
- concurrency model semantics,
- failure model (fail-closed),
- ordering and isolation guarantees,
- capability inflation protection.

The Placement Model SHALL remain authoritative for:
- HOW execution ownership exists in space,
- ownership semantics,
- locality semantics,
- execution location abstraction,
- mobility semantics,
- placement lifecycle semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Runtime execution evaluated
- **WHEN** runtime execution semantics are evaluated
- **THEN** Runtime Abstraction SHALL govern HOW execution is implemented

#### Scenario: Placement ownership evaluated
- **WHEN** ownership-in-space semantics are evaluated
- **THEN** Placement Model SHALL govern HOW execution ownership exists in space

#### Scenario: Authority overlap detected
- **WHEN** Runtime Abstraction and Placement Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation
