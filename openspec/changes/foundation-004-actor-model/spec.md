# FOUNDATION-004: Actor Model — Constitutional Specification

## 1. Actor Abstraction Model

### 1.1 Definition

An actor SHALL be defined as a behavioral abstraction. The actor abstraction SHALL specify what an actor does, not how it is implemented. Execution realization is a runtime adapter concern and MUST NOT be part of the actor contract.

### 1.2 Actor Responsibilities

An actor SHALL:
- Receive messages
- Process messages one at a time within a single logical execution boundary
- Maintain encapsulated state
- Participate in supervision
- Expose a stable identity

### 1.3 Actor Non-Responsibilities

An actor MUST NOT:
- Own transport or networking
- Own persistence or durable data management
- Own business workflow orchestration or saga coordination
- Own runtime scheduling or execution decisions
- Expose runtime primitive types (scheduler handles, thread identifiers, executor references)
- Manage observability infrastructure (metrics, tracing, logging implementation)
- Rely on observability propagation as a behavioral contract; observability MAY propagate through runtime contract mechanisms, but actor behavior MUST remain observability-neutral
- Assume any specific concurrency implementation model

### 1.4 Actor Invariants

- Actor state SHALL be encapsulated: internal state MUST NOT be directly accessible from outside the actor
- State transitions SHALL occur only through message processing
- The physical execution strategy MUST NOT affect the behavioral contract

### 1.5 Actor Isolation Guarantee (Constitutional Invariant)

An actor SHALL maintain a single logical execution boundary. Within this boundary, exactly one message SHALL be processed at a time. Concurrent message processing within a single actor MUST NOT occur.

#### Scenario: Single message processing
- **WHEN** an actor is processing a message
- **THEN** the actor SHALL NOT begin processing another message until the current message processing is complete

#### Scenario: Concurrent access prevention
- **WHEN** multiple messages arrive for an actor simultaneously
- **THEN** they SHALL be processed sequentially, one at a time, within the actor's single logical execution boundary

#### Scenario: Actor non-responsibility enforcement
- **WHEN** an actor implementation owns transport, persistence, runtime scheduling, or observability infrastructure
- **THEN** this SHALL be a violation of the actor contract

## 2. Actor Identity and Addressing

### 2.1 Identity Definition

Actor identity SHALL be a logical reference that is location-transparent by contract. The identity MUST NOT encode location, transport, deployment topology, or runtime affinity. The identity SHALL be unique within its resolution scope. The runtime adapter SHALL resolve identity to concrete delivery.

### 2.2 Location Transparency Contract

The core MUST NOT distinguish between local, remote, embedded, simulated, or distributed actors at the identity level. Location resolution is a runtime adapter concern.

### 2.3 Addressing Semantics

Actor addressing SHALL support point-to-point delivery (message to a specific actor). No address pattern SHALL assume locality.

### 2.4 Identity Invariants

- Identity MUST NOT contain network addresses, process identifiers, thread identifiers, runtime handles, or deployment-specific information
- Identity MUST be unique within its resolution scope
- Identity MUST be stable for the lifetime of the actor

#### Scenario: Actor identity is location-transparent
- **WHEN** an actor reference is used to send a message
- **THEN** the sender MUST NOT be able to determine whether the target actor is local, remote, embedded, simulated, or distributed

#### Scenario: Actor identity uniqueness
- **WHEN** an actor is created
- **THEN** its identity SHALL be unique within its resolution scope, and the identity MUST NOT collide with any other actor's identity

#### Scenario: Identity does not encode location
- **WHEN** an actor identity is inspected
- **THEN** it MUST NOT contain network addresses, process identifiers, thread identifiers, runtime handles, or any deployment-specific information

## 3. Communication Model

### 3.1 Communication Semantics

Actor-to-actor communication SHALL be defined by observable semantics, not by delivery realization. Concrete delivery realization is a runtime adapter concern and MUST NOT affect actor contract semantics.

### 3.2 Ordering Guarantees

Messages from the same sender to the same receiver SHALL be delivered in the order they were sent. No ordering guarantee SHALL be made for messages from different senders to the same receiver.

### 3.3 Delivery Expectations

Delivery expectations SHALL be runtime-defined and explicit. At-most-once MAY be the default behavior of a conforming runtime adapter unless overridden by runtime capability. Silent ambiguity is forbidden; actor semantics MUST remain deterministic regardless of runtime delivery guarantees.

### 3.4 Isolation Semantics

Messages SHALL be isolated: no shared mutable state between sender and receiver. The sender MUST NOT observe the receiver's internal state through message delivery.

### 3.5 Visibility Rules

The sender MUST NOT retain access to mutable state within a message after sending. The receiver SHALL receive a logically independent copy or ownership of the message.

### 3.6 Determinism Guarantees

Given the same sequence of messages, the same actor state, the same logical time, and the same runtime capabilities, the observable communication outcome MUST be identical.

#### Scenario: Ordered delivery between sender and receiver
- **WHEN** actor A sends messages M1, M2, M3 in sequence to actor B
- **THEN** actor B SHALL receive M1, M2, M3 in the order they were sent

#### Scenario: Deterministic delivery expectations
- **WHEN** a message is delivered from actor A to actor B
- **THEN** the delivery expectation SHALL be runtime-defined and explicit; duplicate delivery MUST NOT occur unless explicitly allowed by the runtime capability

#### Scenario: Message isolation
- **WHEN** actor A sends a message to actor B
- **THEN** actor A MUST NOT retain any reference to mutable state within the message after sending, and actor B MUST NOT observe any state of actor A through the message

### 3.7 Request/Response Communication Semantics

Request-response interaction SHALL be defined as semantic message exchange, not transport mechanism. The actor contract MUST NOT assume synchronous waiting, blocking semantics, or transport coupling. A request message MAY carry a correlation identifier that allows a response message to be associated with the original request. Correlation SHALL be defined as a semantic property of the message, not as a transport or execution concern.

- Request and response are both messages; the contract makes no distinction between their delivery semantics
- Correlation MUST remain implementation-neutral: no assumptions about handles, channels, promises, or futures
- The actor contract MUST NOT define APIs, method signatures, or interface contracts for request-response patterns
- Request-response interaction MUST be realizable through one-way message delivery semantics alone

#### Scenario: Request-response via correlation
- **WHEN** actor A sends a message to actor B with a correlation identifier
- **THEN** actor B MAY send a response message that includes the same correlation identifier, and the actor contract MUST NOT assume any blocking, waiting, or synchronous execution

#### Scenario: Request-response without transport assumptions
- **WHEN** actor A expects a response from actor B
- **THEN** the actor contract MUST NOT assume that response delivery uses a different transport, channel, or mechanism than one-way message delivery

## 4. Message Model

### 4.1 Immutability Expectations

Messages SHALL be treated as immutable by convention at the contract level. Message content MUST NOT be mutated by the sender after sending or by the receiver during processing.

### 4.2 Canonical Message Boundaries

A message SHALL represent a complete unit of communication. Messages SHALL be self-contained and MUST NOT require out-of-band context to interpret.

### 4.3 Ownership Semantics

A message SHALL belong to exactly one actor at a time. Ownership SHALL be explicit and transferable only through delivery. The sending actor MUST relinquish ownership; the receiving actor SHALL acquire ownership.

### 4.4 Serialization Neutrality

The contract MUST NOT assume any specific serialization format. Serialization SHALL be a runtime adapter concern when crossing location boundaries.

### 4.5 Invalid Message Handling

Messages that do not conform to the expected type or schema for a given actor SHALL be handled by the runtime adapter according to its failure model. The actor MUST NOT process invalid messages.

#### Scenario: Message immutability
- **WHEN** a message is delivered to an actor
- **THEN** the message content MUST NOT be mutated by either the sender after sending or the receiver during processing

#### Scenario: Message ownership transfer
- **WHEN** a message is sent from actor A to actor B
- **THEN** ownership of the message SHALL transfer from A to B; actor A MUST NOT access the message after sending

#### Scenario: Serialization neutrality
- **WHEN** a message crosses a location boundary
- **THEN** serialization format SHALL be chosen by the runtime adapter; the actor contract MUST NOT mandate any specific format

#### Scenario: Invalid message handling
- **WHEN** actor B receives a message that does not conform to its expected message types
- **THEN** the runtime adapter SHALL treat the message as invalid according to its failure model; the actor SHALL NOT process the invalid message

## 5. Actor Lifecycle

### 5.1 Lifecycle States

The actor lifecycle SHALL consist of the following states:
- **Created**: Actor definition instantiated but not yet initialized
- **Starting**: Runtime performing actor initialization
- **Running**: Actor operational and processing messages
- **Restarting**: Supervisor-initiated restart in progress
- **Stopped**: Actor terminated gracefully
- **Failed**: Actor terminated due to unrecoverable error

### 5.2 Valid Transitions

```
Created → Starting: Runtime begins actor initialization
Starting → Running: Initialization completes successfully
Running → Restarting: Supervisor initiates restart
Restarting → Starting: Restart re-initialization
Restarting → Failed: Restart not permitted or exhausted
Running → Stopped: Graceful stop completes
Running → Failed: Unhandled failure, supervisor cannot recover
Stopped → [terminal]
Failed → [terminal]
```

### 5.3 Lifecycle Invariants

- Every actor SHALL terminate in exactly one terminal state (Stopped or Failed)
- An actor in a terminal state MUST NOT transition to any other state
- Lifecycle execution is delegated to the runtime adapter
- All transitions MUST be deterministic

### 5.4 Restart Semantics

Restart SHALL NOT imply state preservation. Restart SHALL NOT imply state reset. Post-restart actor state SHALL be determined by actor initialization semantics and runtime contract capabilities. Residual state assumptions (that any pre-restart state survives or that state is automatically cleared) are forbidden.

A restart SHALL return the actor to Starting state. The actor's state after restart SHALL be determined by the initialization semantics, not by residual pre-restart state.

### 5.5 Fail-Closed Semantics

All ambiguous lifecycle states MUST result in a transition to Failed. The lifecycle MUST fail closed.

### 5.6 Actor Instantiation Semantics

Actor existence is runtime-mediated. Actor materialization SHALL be performed by the runtime adapter. The core MUST NOT assume direct ownership of actor creation or construction mechanics. The lifecycle transition into Created state SHALL remain deterministic: given identical actor definition and identical runtime capabilities, the transition to Created SHALL occur identically.

The actor contract defines lifecycle semantics, not creation mechanics. Spawning, construction, factories, and initialization APIs are runtime adapter concerns.

#### Scenario: Actor materialization is runtime-mediated
- **WHEN** an actor is instantiated
- **THEN** the runtime adapter SHALL manage materialization; core code MUST NOT assume direct ownership of actor creation

#### Scenario: Deterministic creation
- **WHEN** an actor transitions into Created state
- **THEN** the transition SHALL be deterministic given identical actor definition and runtime capabilities

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

## 6. Supervision Model

### 6.1 Supervision Definition

Supervision SHALL be defined as a parent-child relationship. Every actor MAY have a supervisor (parent). The parent-child relationship SHALL express parent-child supervision semantics.

### 6.2 Failure Propagation

When a child actor fails, the parent supervisor SHALL be notified. Failure SHALL propagate from child to parent.

### 6.3 Escalation Semantics

The parent SHALL decide one of the following supervision strategies:
- **Restart**: The child SHALL transition to Restarting state
- **Stop**: The child SHALL remain in Failed state or transition to Stopped state
- **Escalate**: The failure SHALL propagate to the grandparent supervisor

Escalation SHALL traverse successive parent-child relationships until a supervisor handles the failure or the root supervisor is reached.

### 6.4 Supervision Invariants

- Supervision strategies are semantic policies within the contract
- Implementation of supervision detection and execution is a runtime adapter concern
- Supervision MUST fail closed: ambiguity MUST NOT result in implicit success
- Root supervision (no parent available) SHALL be handled by the runtime adapter

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

### 6.5 Topology Neutrality (Constitutional Invariant)

The Actor Contract MUST NOT assume any topology beyond parent-child supervision semantics. The following are runtime adapter concerns and MUST NOT appear in the actor contract:

- Supervision trees
- Routing groups
- Actor registries
- Placement strategies
- Sharding
- Mesh topologies
- Orchestration topology
- Discovery infrastructure

Only parent-child supervision semantics are constitutional. Topology, placement, grouping, and discovery belong to runtime adapters.

#### Scenario: Topology assumption detected
- **WHEN** the actor contract references supervision trees, routing, registries, placement, sharding, mesh, orchestration, or discovery
- **THEN** this SHALL be a governance violation

## 7. Concurrency Semantics

### 7.1 Actor Isolation

Each actor SHALL have a single logical execution boundary. Within this boundary, exactly one message SHALL be processed at a time. Physical concurrency strategy is a runtime adapter concern and MUST NOT affect actor contract semantics.

### 7.2 Intra-Actor Concurrency

Intra-actor concurrency (processing multiple messages within the same actor simultaneously) MUST NOT occur.

### 7.3 Inter-Actor Concurrency

Multiple actors MAY process messages concurrently. Inter-actor concurrency is permitted but not guaranteed.

### 7.4 Ordering Expectations

No ordering guarantee SHALL be made across different actors. Within a single actor, messages SHALL be processed sequentially.

### 7.5 Visibility Guarantees

Concurrent actor executions MUST NOT share mutable state. Actor execution boundaries MUST be fully isolated.

#### Scenario: Inter-actor concurrency
- **WHEN** two independent actors each have messages to process
- **THEN** the runtime adapter MAY process both actors' messages concurrently

#### Scenario: Intra-actor sequential processing
- **WHEN** a single actor has multiple pending messages
- **THEN** the messages SHALL be processed sequentially, one at a time

#### Scenario: Isolation boundary
- **WHEN** actor A and actor B process messages concurrently
- **THEN** they MUST NOT share any mutable state; their execution boundaries MUST be fully isolated

## 8. Determinism Axiom (Constitutional Invariant)

Given identical actor state, identical message sequence, identical logical time, identical runtime capabilities, and identical context, the observable actor outcome MUST be identical.

Observable outcome SHALL include:
- Actor state transitions
- Lifecycle transitions
- Messages emitted by the actor
- Supervision outcomes
- Failure outcomes

Ambiguity MUST NOT produce implicit success. When determinism cannot be guaranteed, the system SHALL fail closed.

#### Scenario: Identical execution produces identical outcome
- **WHEN** an actor processes the same sequence of messages twice with identical initial state, logical time, runtime capabilities, and context
- **THEN** the observable outcome SHALL be identical in both executions

#### Scenario: Determinism failure is fail-closed
- **WHEN** the runtime adapter cannot guarantee deterministic execution for an actor
- **THEN** the runtime adapter SHALL fail closed rather than proceeding with non-deterministic behavior

#### Scenario: Observable outcome includes all actor effects
- **WHEN** an actor processes a message
- **THEN** the observable outcome SHALL include all state transitions, lifecycle transitions, emitted messages, supervision outcomes, and failure outcomes

### 8.1 Logical Time Semantics

Actor behavior SHALL depend only on logical time provided through the Runtime Contract (FOUNDATION-003). Actor contracts MUST NOT depend on wall-clock time, system clock access, or runtime clock mechanics. Reliance on wall-clock time within actor contracts SHALL be forbidden.

- Logical time source belongs to the Runtime Contract, not actor implementation
- Actor contracts MUST NOT assume timers, timeouts, or runtime clock mechanics
- Logical time remains runtime-neutral and capability-driven: the runtime provides time; the actor consumes it

#### Scenario: Actor uses logical time
- **WHEN** an actor behavior depends on time
- **THEN** it SHALL receive logical time through the Runtime Contract, never from a system clock or wall-clock source

#### Scenario: Wall-clock forbidden in actor contract
- **WHEN** an actor contract references wall-clock time, system time, or real-time clock
- **THEN** this SHALL be a violation of the actor contract

## 9. Actor Capability Model

### 9.1 Mandatory Capabilities

Every conforming actor SHALL support:
- **Receive work**: accept messages for processing within the actor's logical execution boundary
- **Process message**: execute message handling logic to completion
- **State transition**: transition between lifecycle states according to the defined state machine
- **Supervision participation**: participate in parent-child supervision relationships (as child, parent, or both)
- **Identity resolution**: expose a stable identity for addressing and communication

### 9.2 Optional Capabilities

An actor MAY provide:
- **Delayed delivery**: schedule message delivery at a future logical time
- **Lifecycle observation**: observe lifecycle transitions of supervised actors
- **Deterministic replay participation**: participate in deterministic replay of message sequences

Core code MUST NOT assume optional capabilities are present. A runtime adapter that does not provide an optional capability SHALL fail closed if core code attempts to use it.

### 9.3 Forbidden Capabilities

An actor MUST NOT:
- Own transport or networking infrastructure
- Own persistence or durable state management
- Own business workflow orchestration
- Expose runtime primitives (scheduler handles, thread identifiers, executor references)
- Manage observability infrastructure (metrics, tracing, logging implementation)

#### Scenario: Actor receives work
- **WHEN** a message is addressed to an actor
- **THEN** the actor SHALL accept the message for processing within its logical execution boundary

#### Scenario: Actor processes message to completion
- **WHEN** an actor begins processing a message
- **THEN** the processing SHALL continue uninterrupted until completion; partial processing MUST NOT be externally observable

#### Scenario: Actor participates in supervision
- **WHEN** an actor is a parent in a supervision relationship
- **THEN** it SHALL receive failure notifications from its children and SHALL select a supervision strategy

#### Scenario: Delayed delivery not available
- **WHEN** an actor attempts to schedule a message for future delivery and the runtime adapter does not support delayed delivery
- **THEN** the operation SHALL be rejected with an explicit error

#### Scenario: Forbidden capability detected
- **WHEN** an actor implementation owns transport, persistence, workflow orchestration, or runtime primitives
- **THEN** this SHALL be a violation of the actor contract

## 10. Failure Model

### 10.1 Fail-Closed Principle

The actor model SHALL fail closed on all ambiguous, unknown, or invalid states. When the runtime adapter cannot determine the state of an actor, the actor SHALL be treated as failed.

### 10.2 Invalid Message Behavior

Messages that do not conform to the actor's expected message schema SHALL NOT be delivered. The runtime adapter SHALL handle the invalid message according to its failure model.

### 10.3 Actor Failure Propagation

When an actor fails, its supervisor SHALL be notified. Failure SHALL propagate according to parent-child supervision relationships.

### 10.4 Deterministic Error Behavior

Given identical failure conditions, identical actor state, and identical context, the failure outcome MUST be identical.

#### Scenario: Invalid message rejected
- **WHEN** a message does not conform to the actor's expected message schema
- **THEN** the message SHALL NOT be delivered; the runtime adapter SHALL handle the invalid message explicitly

#### Scenario: Ambiguous state is fail-closed
- **WHEN** the runtime adapter cannot determine an actor's lifecycle state
- **THEN** the actor SHALL be treated as failed rather than assumed to be operational

#### Scenario: Escalation on unhandled failure
- **WHEN** a parent supervisor cannot handle a child's failure
- **THEN** the failure SHALL escalate to the grandparent supervisor

## 11. Testing Contract

### 11.1 Deterministic Tests

All actor-dependent tests SHALL be deterministic. Given the same test inputs, runtime configuration, and message sequence, the test SHALL produce the same outcome every execution.

### 11.2 Mock-Only Testing

Testing of actor-dependent code SHALL use mock runtime adapters. No test SHALL require a real actor runtime.

### 11.3 Replayability and Reproducibility

Tests MUST support replayability: the same test with the same inputs SHALL reproduce the same behavior. Tests MUST NOT depend on wall-clock time or non-deterministic external state.

Deterministic replay SHALL reproduce the observable actor outcome without changing actor contract semantics. Replay MUST preserve determinism guarantees: given identical actor definition, message sequence, logical time, runtime capabilities, and context, the replayed outcome SHALL be identical to the original execution.

### 11.4 Coverage Requirement

Coverage of actor contract implementations SHALL be at least 95%.

### 11.5 No Infrastructure Dependencies

No test SHALL require infrastructure dependencies (network, persistence, transport).

#### Scenario: Unit test uses mock runtime
- **WHEN** a test exercises actor-dependent code
- **THEN** the test SHALL inject a mock runtime adapter and SHALL NOT start any real runtime

#### Scenario: Test is deterministic
- **WHEN** a test is executed twice with the same inputs and configuration
- **THEN** the test SHALL produce the same result both times

#### Scenario: Test runs without infrastructure
- **WHEN** a test suite is executed
- **THEN** it SHALL NOT require any network, persistence, or transport infrastructure

## 12. Hexagonal Boundaries

### 12.1 Layer Architecture

| Layer | Role | Depends On |
|---|---|---|
| **Core** | Domain entities, application use cases, actor definitions | Actor Contract only |
| **Actor Contract** | Actor behavioral abstraction, identity, communication, lifecycle, supervision, capability model | Runtime Contract (FOUNDATION-003) only |
| **Runtime Contract** | Runtime capability ports (Execution, Clock, Context, Backpressure) | Defined by FOUNDATION-003 |
| **Adapters** | Concrete runtime implementations | Runtime Contract (FOUNDATION-003); satisfy actor execution requirements through runtime compliance |

### 12.2 Dependency Direction

Core → Actor Contract → Runtime Contract (FOUNDATION-003) → Adapters

- Core MUST depend only on Actor Contract
- Actor Contract MUST depend only on Runtime Contract
- Concrete runtime implementations MUST exist behind runtime adapters

### 12.3 Boundary Violations

The following SHALL be architectural violations:
- Core code referencing concrete actor framework or runtime types
- Actor Contract referencing runtime adapter types
- Bypassing the Actor Contract to depend directly on the Runtime Contract or adapters

#### Scenario: Core depends on concrete actor implementation
- **WHEN** core domain or application code references a concrete actor framework type
- **THEN** this SHALL be a governance violation

#### Scenario: Actor contract depends on runtime adapter
- **WHEN** the actor contract references a type from a concrete runtime adapter
- **THEN** this SHALL be a governance violation

## 13. Governance

### 13.1 Constitutional Invariants

The following invariants SHALL be constitutionally enforced:
1. Core code MUST NOT depend on any concrete actor framework or runtime implementation
2. The actor contract MUST depend only on the Runtime Contract (FOUNDATION-003), never on concrete runtime adapters
3. Actor identity MUST be location-transparent; no identity SHALL encode location, transport, or deployment information
4. All actor lifecycle transitions MUST follow the defined state machine
5. Supervision failures MUST propagate according to parent-child relationships
6. The Determinism Axiom MUST hold for all observable actor outcomes
7. Tests MUST use mock runtimes, never real actor runtime instances
8. New actor capabilities MUST justify constitutional necessity

### 13.2 Forbidden Patterns

The following patterns are explicitly forbidden:
1. Core code referencing concrete actor framework types or runtime implementations
2. Actor contract referencing runtime adapter types
3. Identity encoding location, transport, network, or deployment topology
4. Supervision bypassing parent-child relationships
5. Direct actor state access from outside the actor's execution boundary
6. Non-deterministic actor behavior dependent on implicit runtime state
7. Framework-specific assumptions in actor contracts

### 13.3 Capability Inflation Protection

New actor capabilities MUST satisfy all of the following criteria:
1. **Constitutional necessity**: The capability MUST be required to satisfy a constitutional invariant, not for convenience or implementation preference
2. **Runtime neutrality**: The capability MUST be implementable by any conforming runtime adapter, not specific to one execution engine
3. **Minimal surface**: The capability MUST be the minimal contract that satisfies the requirement
4. **Fail-closed**: Absence of the capability MUST cause explicit failure, not silent degradation

Capabilities MUST NOT be introduced for: convenience of a single runtime adapter, preference for a specific execution model, support for speculative future requirements, or workaround for limitations of any specific runtime adapter.

### 13.4 Violation Detection

Violation of actor model governance SHALL be detectable through:
1. **Dependency analysis**: Verify core code contains no direct dependencies on concrete actor framework implementations
2. **Port type inspection**: Verify actor contract port signatures contain only domain-defined types
3. **Lifecycle compliance audit**: Verify all lifecycle transitions conform to the defined state machine
4. **Identity inspection**: Verify actor identities contain no location or transport information
5. **Mock isolation**: Verify no test imports a concrete actor runtime implementation

#### Scenario: Core depends on concrete actor implementation
- **WHEN** core domain or application code references a concrete actor framework type
- **THEN** this SHALL be a governance violation

#### Scenario: Actor contract depends on runtime adapter
- **WHEN** the actor contract references a type from a concrete runtime adapter
- **THEN** this SHALL be a governance violation

#### Scenario: Capability proposed without constitutional necessity
- **WHEN** a new actor capability is proposed without demonstrating constitutional necessity
- **THEN** the proposal SHALL be rejected pending justification

#### Scenario: Capability is runtime-specific
- **WHEN** a proposed actor capability can only be implemented by one runtime adapter
- **THEN** the proposal SHALL be rejected because it violates runtime neutrality

## 14. Architectural Relationship

### 14.1 Dependency Chain

```
Core
  ↓
Actor Contract (FOUNDATION-004)
  ↓
Runtime Contract (FOUNDATION-003)
  ↓
Runtime Adapter
  ↓
Conforming Runtime Implementations
```

### 14.2 Contract Rules

 - Core code SHALL interact only through Actor Contract abstractions
- Actor Contract MUST depend only on Runtime Contract
- Concrete runtime implementations MUST exist behind runtime adapters
- Runtime adapters SHALL satisfy actor execution requirements through Runtime Contract compliance

### 14.3 Location Transparency

Actor identity and communication MUST remain location-transparent by contract. The core MUST NOT care whether an actor is local, remote, embedded, simulated, or distributed. Location is a runtime adapter concern.

### 14.4 Tokio-First, Never Tokio-Bound

Tokio SHALL be the first runtime adapter for the actor model. The actor contract MUST NOT be designed around Tokio's execution model. Tokio-specific constructs, types, or semantics MUST NOT appear in the actor contract. The contract MUST remain implementable by runtimes with fundamentally different execution models.

### 14.5 Platform Orientation

The actor model SHALL support service-oriented and process-oriented backend composition without coupling actor behavior to orchestration mechanics. Service composition, workflow orchestration, and process coordination are platform capabilities that build on the actor contract — they do not modify it.

## 15. Out of Scope

The following are explicitly NOT part of this specification:
- Runtime execution realization
- Delivery realization
- Concurrency realization
- Runtime scheduling realization
- Transport protocols
- Distributed clustering or actor remoting
- Persistence or durable state
- Observability implementation
- Rust implementation, crates, modules, or Cargo files
- Traits, interfaces, or APIs in language syntax
- Framework SDK design
- Actor discovery or registry infrastructure
- Any concrete runtime adapter implementation
