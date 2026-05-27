## ADDED Requirements

### Constitutional Invariant: Determinism Axiom

Given identical inputs, runtime state, logical time, execution context, and capability availability, the observable execution outcome MUST be identical. Observable outcome includes: execution result, lifecycle transitions, propagated context, failure outcome, and ordering semantics. Ambiguity MUST NOT produce implicit success. When determinism cannot be guaranteed, the runtime SHALL fail closed.

#### Scenario: Identical execution produces identical outcome
- **WHEN** a unit of work is executed twice with identical inputs, state, logical time, context, and capability availability
- **THEN** the observable execution outcome SHALL be identical in both executions

#### Scenario: Determinism failure is fail-closed
- **WHEN** the runtime cannot guarantee deterministic execution for a unit of work
- **THEN** it SHALL reject the work rather than proceeding with non-deterministic behavior

### Requirement: Execution Lifecycle

The runtime SHALL define a standard execution lifecycle for every unit of work. The lifecycle states SHALL be: Pending, Running, Completed, Failed, Cancelled, TimedOut. Transitions between states MUST follow a deterministic state machine. Every unit of work SHALL terminate in exactly one final state. A unit of work in a terminal state MUST NOT transition to any other state.

Valid transitions:
- Pending → Running
- Running → Completed | Failed | Cancelled | TimedOut
- Terminal states (Completed, Failed, Cancelled, TimedOut) → no outgoing transitions

#### Scenario: Work completes successfully
- **WHEN** a unit of work finishes execution without error
- **THEN** its state SHALL transition to Completed and the result SHALL be delivered to the caller

#### Scenario: Work fails definitively
- **WHEN** a unit of work fails and retry is not permitted or exhausted
- **THEN** its state SHALL transition to Failed and the error SHALL be propagated

#### Scenario: Terminal state is immutable
- **WHEN** a unit of work is in Completed, Cancelled, TimedOut, or Failed state
- **THEN** it MUST NOT transition to any other state

### Requirement: Execution Boundaries

Every unit of work SHALL execute within a defined execution boundary. The boundary SHALL define: isolation scope (observable state not finalized until successful completion), cancellation scope (nested work cancelled with parent), timeout scope (nested work shares parent bound), and error scope (unhandled errors propagate to boundary).

#### Scenario: Work spawns nested work
- **WHEN** a unit of work spawns nested work within the same execution boundary
- **THEN** all nested work SHALL be cancelled if the parent is cancelled

#### Scenario: Failed work does not finalize state
- **WHEN** a unit of work reaches Failed, Cancelled, or TimedOut state
- **THEN** its observable execution state SHALL NOT be considered finalized

### Requirement: Failure Model — Fail-Closed

The runtime SHALL fail closed on all ambiguous, unknown, or invalid states. When the runtime cannot determine the outcome of a unit of work, it SHALL NOT assume success. The runtime SHALL propagate a definitive error.

#### Scenario: Runtime cannot determine work outcome
- **WHEN** the runtime cannot determine whether a unit of work completed or failed
- **THEN** it SHALL report an ambiguous-state outcome, never assume success

### Requirement: Concurrency Model

Concurrency SHALL mean: multiple units of work MAY execute in any order with respect to each other, unless an ordering constraint is declared. Isolation SHALL mean: concurrent units of work MUST NOT interfere with each other's execution boundaries unless they share an explicit communication channel. The runtime SHALL guarantee at-most-once execution semantics for each unit of work.

#### Scenario: At-most-once execution
- **WHEN** a unit of work reaches a terminal state
- **THEN** the runtime SHALL NOT re-execute it

### Requirement: Testing Contract

Testing of runtime-dependent code SHALL use mock runtime implementations. No test SHALL require a real runtime. The mock runtime SHALL provide deterministic control over logical time, execution order, and context. Coverage of runtime port implementations SHALL be at least 95%.

#### Scenario: Unit test uses mock runtime
- **WHEN** a test exercises code that depends on runtime capability ports
- **THEN** the test SHALL inject a mock runtime implementation and SHALL NOT start any real runtime