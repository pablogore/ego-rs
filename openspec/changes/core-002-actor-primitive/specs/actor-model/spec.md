## ADDED Requirements

### Requirement: Actor abstraction

An actor SHALL be a minimal behavioral contract defined by its message type. The actor trait SHALL be the minimal primitive — it defines what message type an actor accepts, not how processing, dispatch, or runtime execution occurs.

```rust
/// The minimal actor contract.
///
/// An actor is identified by its message type. Runtime behavior
/// (mailbox, dispatch, sequential execution, supervision) is owned
/// by the runtime layer (CORE-003), not by this contract.
pub trait Actor {
    /// The message type this actor accepts.
    type Message;
}
```

An actor MUST NOT: own transport, own persistence, own workflow orchestration, own runtime scheduling, expose runtime primitives, or manage observability infrastructure.

#### Scenario: Actor defines only message type
- **WHEN** an actor type is defined
- **THEN** it SHALL declare only `type Message`, not `fn receive`, not output semantics, not effect types

### Requirement: Actor identity and addressing

Actor identity (`ActorId`) SHALL be a logical reference that is location-transparent. The identity MUST NOT encode location, transport, deployment topology, or runtime affinity. Actor addressing SHALL support point-to-point delivery. No address pattern SHALL assume locality.

```rust
/// A unique, location-transparent actor identifier.
///
/// Does not encode location, transport, deployment topology,
/// or runtime affinity. Resolution is a runtime concern (CORE-003).
pub struct ActorId(String);
```

#### Scenario: Identity is location-transparent
- **WHEN** an `ActorId` is inspected
- **THEN** it MUST NOT contain network addresses, process IDs, thread IDs, or deployment-specific information

### Requirement: actor_id! macro

The domain SHALL provide a compile-time `actor_id!` macro for declaring deterministic `ActorId` values. These IDs SHALL be `'static` — resolved at compile time, not constructed at runtime.

```rust
/// Compile-time actor identifier.
///
/// Produces a `&'static ActorId` that is deterministic and
/// cannot be constructed dynamically.
pub macro actor_id {
    ($name:ident) => {
        {
            static ID: std::sync::LazyLock<ActorId> = std::sync::LazyLock::new(|| {
                ActorId::new(stringify!($name))
            });
            &*ID
        }
    }
}
```

#### Scenario: Compile-time identity
- **WHEN** `actor_id!(my_actor)` is evaluated
- **THEN** it SHALL produce a `&'static ActorId` with value `"my_actor"`

### Requirement: Communication semantics

Messages from the same sender to the same receiver SHALL be delivered in FIFO order. Delivery SHALL be at-most-once. Messages SHALL be isolated — no shared mutable state between sender and receiver. These guarantees are enforced by the runtime (CORE-003), not by the actor contract.

### Requirement: Message model

Messages SHALL be treated as immutable. The serialization format SHALL be defined through the `Serializable` trait provided by the contracts layer (canonical contracts). Invalid messages (not conforming to expected type) SHALL be rejected by the runtime.

### Requirement: Actor lifecycle — semantic states only

Lifecycle states (Created, Starting, Running, Stopping, Stopped, Failed) are semantic. State transitions are enforced by the runtime (CORE-003). The domain contract defines what states exist; the runtime owns execution and transition mechanics.

#### Scenario: Terminal state immutability
- **WHEN** an actor is in Stopped or Failed state
- **THEN** it MUST NOT transition to any other state

### Requirement: Supervision contract — semantic only

Supervision SHALL be a parent-child relationship. When a child fails, the parent SHALL be notified. The parent selects a strategy (Restart, Stop, Escalate). Supervision *execution* — how failures are detected, how restarts are scheduled — is a runtime concern (CORE-003). This spec defines the contract only.

### Requirement: Determinism Axiom

Given identical actor state, identical message sequence, identical logical time, and identical context, the observable actor outcome MUST be identical.

#### Scenario: Identical execution produces identical outcome
- **WHEN** an actor processes the same message sequence twice with identical initial state
- **THEN** the observable outcome SHALL be identical

### Requirement: Domain vs Runtime boundary

| Owned by DOMAIN (CORE-002) | Owned by RUNTIME (CORE-003) |
|---|---|
| `Actor` trait (`type Message`) | `ActorSystem` |
| `ActorId` + `actor_id!` macro | Mailbox implementation |
| Message contract + immutability | Sequential execution guarantees |
| Semantic lifecycle states | Lifecycle transition execution |
| Semantic supervision contract | Supervision execution + detection |
| Communication semantics (what) | Dispatch implementation (how) |
| Determinism Axiom (contract) | Determinism enforcement (runtime) |

No leakage. No mixed ownership.

### Requirement: Testing contract

Tests SHALL use mock runtime adapters. No test SHALL require a real actor runtime. Tests SHALL be deterministic. Coverage SHALL be at least 95%.

#### Scenario: Unit test uses mock runtime
- **WHEN** a test exercises actor-dependent code
- **THEN** it SHALL inject a mock runtime and SHALL NOT start any real runtime