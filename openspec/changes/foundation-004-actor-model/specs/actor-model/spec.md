## ADDED Requirements

### Requirement: Actor abstraction model

An actor SHALL be defined as a behavioral abstraction with the following responsibilities: receive messages, process messages one at a time within a single logical execution boundary, maintain encapsulated state, participate in supervision, and expose a stable identity. An actor MUST NOT: own transport, own persistence, own business workflow orchestration, own runtime scheduling, expose runtime primitives, or manage observability infrastructure. Actor execution mechanics (mailbox, scheduling, threading) are runtime adapter concerns and MUST NOT be part of the actor contract.

#### Scenario: Actor receives and processes a message
- **WHEN** an actor receives a message within its logical execution boundary
- **THEN** the actor SHALL process the message to completion before accepting the next message, and SHALL NOT process messages concurrently within its single execution boundary

#### Scenario: Actor encapsulates state
- **WHEN** an actor holds internal state
- **THEN** that state SHALL NOT be directly accessible from outside the actor; all state mutations SHALL occur only through message processing

#### Scenario: Actor non-responsibility enforcement
- **WHEN** an actor implementation owns transport, persistence, runtime scheduling, or observability infrastructure
- **THEN** this SHALL be a violation of the actor contract

### Constitutional Invariant: Actor isolation guarantee

An actor SHALL maintain a single logical execution boundary. Within this boundary, exactly one message SHALL be processed at a time. Concurrent message processing within a single actor MUST NOT occur. The physical execution strategy (thread, coroutine, event loop) is a runtime adapter concern and MUST NOT affect the isolation guarantee.

#### Scenario: Single message processing
- **WHEN** an actor is processing a message
- **THEN** the actor SHALL NOT begin processing another message until the current message processing is complete

#### Scenario: Concurrent access prevention
- **WHEN** multiple messages arrive for an actor simultaneously
- **THEN** they SHALL be processed sequentially, one at a time, within the actor's single logical execution boundary

### Requirement: Actor identity and addressing

Actor identity SHALL be a logical reference that is location-transparent by contract. The identity MUST NOT encode location, transport, deployment topology, or runtime affinity. The identity SHALL be unique within its scope. The runtime adapter SHALL resolve identity to concrete delivery. Actor addressing SHALL support: point-to-point delivery (message to a specific actor), and no address pattern SHALL assume locality.

#### Scenario: Actor identity is location-transparent
- **WHEN** an actor reference is used to send a message
- **THEN** the sender MUST NOT be able to determine whether the target actor is local, remote, embedded, simulated, or distributed

#### Scenario: Actor identity uniqueness
- **WHEN** an actor is created
- **THEN** its identity SHALL be unique within its resolution scope, and the identity MUST NOT collide with any other actor's identity

#### Scenario: Identity does not encode location
- **WHEN** an actor identity is inspected
- **THEN** it MUST NOT contain network addresses, process identifiers, thread identifiers, runtime handles, or any deployment-specific information

### Requirement: Communication model

Actor-to-actor communication SHALL follow these semantics: messages are delivered from one actor to another through their identities; ordering guarantees SHALL be defined between a single sender and a single receiver (messages from the same sender to the same receiver SHALL be delivered in the order they were sent); delivery expectations SHALL be at-most-once by default unless otherwise specified by the runtime adapter; messages SHALL be isolated (no shared state between sender and receiver); the sender MUST NOT observe the receiver's internal state through message delivery.

#### Scenario: Ordered delivery between sender and receiver
- **WHEN** actor A sends messages M1, M2, M3 in sequence to actor B
- **THEN** actor B SHALL receive M1, M2, M3 in the order they were sent

#### Scenario: At-most-once delivery
- **WHEN** a message is delivered from actor A to actor B
- **THEN** the message SHALL be delivered at most once; duplicate delivery MUST NOT occur unless explicitly allowed by the runtime adapter

#### Scenario: Message isolation
- **WHEN** actor A sends a message to actor B
- **THEN** actor A MUST NOT retain any reference to mutable state within the message after sending, and actor B MUST NOT observe any state of actor A through the message

### Requirement: Message model

Messages SHALL be treated as immutable by convention at the contract level. A message SHALL belong to exactly one actor at a time (ownership is explicit and transferable only through delivery). The contract MUST NOT assume any specific serialization format. Serialization SHALL be a runtime adapter concern when crossing location boundaries. Invalid messages (messages that do not conform to the expected type or schema for a given actor) SHALL be handled by the runtime adapter according to its failure model.

#### Scenario: Message immutability
- **WHEN** a message is delivered to an actor
- **THEN** the message content MUST NOT be mutated by either the sender after sending or the receiver during processing; immutability SHALL be a convention enforced by contract

#### Scenario: Message ownership transfer
- **WHEN** a message is sent from actor A to actor B
- **THEN** ownership of the message SHALL transfer from A to B; actor A MUST NOT access the message after sending

#### Scenario: Serialization neutrality
- **WHEN** a message crosses a location boundary (e.g., remote delivery)
- **THEN** serialization format SHALL be chosen by the runtime adapter; the actor contract MUST NOT mandate any specific format

#### Scenario: Invalid message handling
- **WHEN** actor B receives a message that does not conform to its expected message types
- **THEN** the runtime adapter SHALL treat the message as invalid according to its failure model; the actor SHALL NOT process the invalid message

### Requirement: Actor lifecycle

The actor lifecycle SHALL consist of the following states: Created, Starting, Running, Restarting, Stopped, Failed. Transitions between states MUST follow a deterministic state machine. Every actor SHALL terminate in exactly one terminal state (Stopped or Failed). An actor in a terminal state MUST NOT transition to any other state. Lifecycle execution is delegated to the runtime adapter.

Valid transitions:
- Created → Starting: Runtime begins actor initialization
- Starting → Running: Initialization completes successfully
- Running → Restarting: Supervisor initiates restart
- Restarting → Starting: Restart re-initialization
- Restarting → Failed: Restart not permitted or exhausted
- Running → Stopped: Graceful stop completes
- Running → Failed: Unhandled failure, supervisor cannot recover
- Stopped → [terminal]
- Failed → [terminal]

#### Scenario: Actor starts successfully
- **WHEN** an actor transitions from Created to Starting and initialization succeeds
- **THEN** the actor SHALL transition to Running state

#### Scenario: Actor stops gracefully
- **WHEN** an actor in Running state receives a stop signal
- **THEN** the actor SHALL transition to Stopped state after completing its current message processing

#### Scenario: Actor fails definitively
- **WHEN** an actor in Running state encounters an unhandled failure and the supervisor cannot recover
- **THEN** the actor SHALL transition to Failed state

#### Scenario: Actor restarts
- **WHEN** a supervisor initiates a restart for an actor in Running state
- **THEN** the actor SHALL transition to Restarting state, then to Starting state for re-initialization

#### Scenario: Terminal state immutability
- **WHEN** an actor is in Stopped or Failed state
- **THEN** it MUST NOT transition to any other state

### Requirement: Supervision model

Supervision SHALL be defined as a parent-child relationship. Every actor MAY have a supervisor (parent). The parent-child relationship SHALL form a supervision hierarchy. Failure propagation SHALL follow these rules: when a child actor fails, the parent supervisor SHALL be notified; the parent SHALL decide one of the following supervision strategies: restart the child, stop the child, or escalate the failure to the grandparent. Escalation SHALL continue up the hierarchy until a supervisor handles the failure or the root supervisor is reached. Supervision strategies are semantic policies within the contract. Implementation of supervision detection and execution is a runtime adapter concern.

#### Scenario: Child failure notification
- **WHEN** a child actor transitions to Failed state
- **THEN** the parent supervisor SHALL be notified of the failure

#### Scenario: Supervisor restarts child
- **WHEN** a child actor fails and the parent selects the restart strategy
- **THEN** the child SHALL transition to Restarting state

#### Scenario: Supervisor stops child
- **WHEN** a child actor fails and the parent selects the stop strategy
- **THEN** the child SHALL remain in Failed state or transition to Stopped state

#### Scenario: Failure escalation
- **WHEN** a child actor fails and the parent cannot handle the failure
- **THEN** the parent SHALL escalate the failure to its own supervisor

#### Scenario: Root supervision
- **WHEN** an actor has no parent supervisor and fails
- **THEN** the runtime adapter SHALL handle the failure according to its top-level supervision policy

### Requirement: Concurrency semantics

Concurrency semantics SHALL be defined in terms of actor isolation, not threads or executors. Each actor SHALL have a single logical execution boundary. Within this boundary, exactly one message SHALL be processed at a time. Multiple actors MAY process messages concurrently (inter-actor concurrency). Intra-actor concurrency (processing multiple messages within the same actor simultaneously) MUST NOT occur. Physical concurrency strategy is a runtime adapter concern.

#### Scenario: Inter-actor concurrency
- **WHEN** two independent actors each have messages to process
- **THEN** the runtime adapter MAY process both actors' messages concurrently

#### Scenario: Intra-actor sequential processing
- **WHEN** a single actor has multiple pending messages
- **THEN** the messages SHALL be processed sequentially, one at a time

#### Scenario: Isolation boundary
- **WHEN** actor A and actor B process messages concurrently
- **THEN** they MUST NOT share any mutable state; their execution boundaries MUST be fully isolated

### Requirement: Determinism Axiom

The following determinism axiom SHALL be a constitutional invariant of the actor model:

> Given identical actor state, identical message sequence, identical logical time, identical runtime capabilities, and identical context, the observable actor outcome MUST be identical.

Observable outcome SHALL include: actor state transitions, lifecycle transitions, messages emitted by the actor, supervision outcomes, and failure outcomes. Ambiguity MUST NOT produce implicit success. When determinism cannot be guaranteed, the system SHALL fail closed.

#### Scenario: Identical execution produces identical outcome
- **WHEN** an actor processes the same sequence of messages twice with identical initial state, logical time, runtime capabilities, and context
- **THEN** the observable outcome SHALL be identical in both executions

#### Scenario: Determinism failure is fail-closed
- **WHEN** the runtime adapter cannot guarantee deterministic execution for an actor
- **THEN** the runtime adapter SHALL fail closed rather than proceeding with non-deterministic behavior

#### Scenario: Observable outcome includes all actor effects
- **WHEN** an actor processes a message
- **THEN** the observable outcome SHALL include all state transitions, lifecycle transitions, emitted messages, supervision outcomes, and failure outcomes

### Requirement: Actor capability model — mandatory

Every actor participant in the system MUST support the following capabilities:
- **Receive work**: accept messages for processing within the actor's logical execution boundary
- **Process message**: execute message handling logic to completion
- **State transition**: transition between lifecycle states according to the defined state machine
- **Supervision participation**: participate in the parent-child supervision hierarchy (as child, parent, or both)
- **Identity resolution**: expose a stable identity for addressing and communication

#### Scenario: Actor receives work
- **WHEN** a message is addressed to an actor
- **THEN** the actor SHALL accept the message for processing within its logical execution boundary

#### Scenario: Actor processes message to completion
- **WHEN** an actor begins processing a message
- **THEN** the processing SHALL continue uninterrupted until completion; partial processing MUST NOT be externally observable

#### Scenario: Actor participates in supervision
- **WHEN** an actor is a parent in the supervision hierarchy
- **THEN** it SHALL receive failure notifications from its children and SHALL select a supervision strategy

### Requirement: Actor capability model — optional

An actor participant MAY provide the following capabilities:
- **Delayed delivery**: ability to schedule message delivery at a future logical time
- **Lifecycle observation**: ability to observe lifecycle transitions of supervised actors
- **Deterministic replay participation**: ability to participate in deterministic replay of message sequences

Core code MUST NOT assume optional capabilities are present. A runtime adapter that does not provide an optional capability SHALL fail closed if core code attempts to use it.

#### Scenario: Delayed delivery not available
- **WHEN** an actor attempts to schedule a message for future delivery and the runtime adapter does not support delayed delivery
- **THEN** the operation SHALL be rejected with an explicit error

#### Scenario: Lifecycle observation
- **WHEN** an actor observes lifecycle transitions of its children and the runtime adapter supports lifecycle observation
- **THEN** the observer SHALL receive notifications on each lifecycle transition

### Requirement: Actor capability model — forbidden

An actor MUST NOT:
- Own transport or networking infrastructure
- Own persistence or durable state management
- Own business workflow orchestration
- Expose runtime primitives (scheduler handles, thread identifiers, executor references)
- Manage observability infrastructure (metrics, tracing, logging implementation)

#### Scenario: Forbidden capability detected
- **WHEN** an actor implementation owns transport, persistence, workflow orchestration, or runtime primitives
- **THEN** this SHALL be a violation of the actor contract

### Requirement: Failure model

The actor model SHALL fail closed on all ambiguous, unknown, or invalid states. Failure behaviors:
- Invalid message: the runtime adapter SHALL NOT deliver messages that do not conform to the actor's expected message schema; the invalid message SHALL be handled according to the runtime adapter's failure model
- Actor failure propagation: when an actor fails, its supervisor SHALL be notified
- Supervision failure visibility: when a supervisor cannot handle a child failure, the failure SHALL escalate
- Ambiguous-state handling: when the runtime adapter cannot determine the state of an actor, the actor SHALL be treated as failed

#### Scenario: Invalid message rejected
- **WHEN** a message does not conform to the actor's expected message schema
- **THEN** the message SHALL NOT be delivered; the runtime adapter SHALL handle the invalid message explicitly

#### Scenario: Ambiguous state is fail-closed
- **WHEN** the runtime adapter cannot determine an actor's lifecycle state
- **THEN** the actor SHALL be treated as failed rather than assumed to be operational

#### Scenario: Escalation on unhandled failure
- **WHEN** a parent supervisor cannot handle a child's failure
- **THEN** the failure SHALL escalate to the grandparent supervisor

### Requirement: Actor non-responsibilities

An actor MUST NOT:
- Own transport or networking
- Own persistence or durable data management
- Own business workflow orchestration or saga coordination
- Own runtime scheduling or execution decisions
- Expose runtime primitive types
- Manage observability infrastructure (metrics, tracing, logging)
- Assume any specific concurrency implementation model

#### Scenario: Actor non-responsibility violation
- **WHEN** an actor implementation assumes responsibility for transport, persistence, workflow orchestration, or observability
- **THEN** this SHALL be a violation of the actor contract

### Requirement: Testing contract

Testing of actor-dependent code SHALL use mock runtime adapters. No test SHALL require a real actor runtime. Tests SHALL be deterministic: given the same test inputs, the same runtime configuration, and the same message sequence, the test SHALL produce the same outcome every execution. Tests MUST support replayability. Coverage of actor contract implementations SHALL be at least 95%. No test SHALL require infrastructure dependencies (network, persistence, transport).

#### Scenario: Unit test uses mock runtime
- **WHEN** a test exercises actor-dependent code
- **THEN** the test SHALL inject a mock runtime adapter and SHALL NOT start any real runtime

#### Scenario: Test is deterministic
- **WHEN** a test is executed twice with the same inputs and configuration
- **THEN** the test SHALL produce the same result both times

#### Scenario: Test runs without infrastructure
- **WHEN** a test suite is executed
- **THEN** it SHALL NOT require any network, persistence, or transport infrastructure

### Requirement: Governance — constitutional invariants

The following invariants SHALL be constitutionally enforced:
1. Core code MUST NOT depend on any concrete actor framework or runtime implementation
2. The actor contract MUST depend only on the Runtime Contract (FOUNDATION-003), never on concrete runtime adapters
3. Actor identity MUST be location-transparent; no identity SHALL encode location, transport, or deployment information
4. All actor lifecycle transitions MUST follow the defined state machine
5. Supervision failures MUST propagate according to the parent-child hierarchy
6. The Determinism Axiom MUST hold for all observable actor outcomes
7. Tests MUST use mock runtimes, never real actor runtime instances
8. New actor capabilities MUST justify constitutional necessity; capabilities MUST NOT be introduced for convenience, implementation preference, specific runtime support, or speculative future requirements

#### Scenario: Core depends on concrete actor implementation
- **WHEN** core domain or application code references a concrete actor framework type
- **THEN** this SHALL be a governance violation

#### Scenario: Actor contract depends on runtime adapter
- **WHEN** the actor contract references a type from a concrete runtime adapter
- **THEN** this SHALL be a governance violation

#### Scenario: Location transparency maintained
- **WHEN** an actor identity is inspected
- **THEN** it MUST NOT contain any location or transport information

#### Scenario: Lifecycle compliance
- **WHEN** a runtime adapter performs an actor lifecycle transition
- **THEN** it SHALL comply with the defined lifecycle state machine

### Requirement: Governance — forbidden patterns

The following patterns are explicitly forbidden:
1. Core code referencing concrete actor framework types or runtime implementations
2. Actor contract referencing runtime adapter types
3. Identity encoding location, transport, network, or deployment topology
4. Supervision bypassing the parent-child hierarchy
5. Direct actor state access from outside the actor's execution boundary
6. Non-deterministic actor behavior dependent on implicit runtime state

#### Scenario: Forbidden pattern detected
- **WHEN** a review or verification process detects a forbidden pattern
- **THEN** the change SHALL be rejected until the pattern is removed

#### Scenario: Identity location encoding forbidden
- **WHEN** an actor identity contains a network address, process ID, thread ID, or runtime handle
- **THEN** this SHALL be rejected

### Requirement: Governance — capability inflation protection

New actor capabilities MUST satisfy all of the following criteria:
1. **Constitutional necessity**: The capability MUST be required to satisfy a constitutional invariant, not for convenience or implementation preference
2. **Runtime neutrality**: The capability MUST be implementable by any conforming runtime adapter, not specific to one execution engine
3. **Minimal surface**: The capability MUST be the minimal contract that satisfies the requirement
4. **Fail-closed**: Absence of the capability MUST cause explicit failure, not silent degradation

Capabilities MUST NOT be introduced for: convenience of a single runtime adapter, preference for a specific execution model, support for speculative future requirements, or workaround for limitations of any specific runtime adapter.

#### Scenario: Capability proposed without constitutional necessity
- **WHEN** a new actor capability is proposed without demonstrating constitutional necessity
- **THEN** the proposal SHALL be rejected pending justification

#### Scenario: Capability is runtime-specific
- **WHEN** a proposed actor capability can only be implemented by one runtime adapter
- **THEN** the proposal SHALL be rejected because it violates runtime neutrality
