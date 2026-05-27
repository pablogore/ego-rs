# Design: Actor Primitive (CORE-002)

## Context

ego-rs needs an actor abstraction as its central primitive. The domain layer defines **what** an actor is (contract). The runtime layer (CORE-003) defines **how** actors execute. CORE-002 owns only the domain contract — no runtime mechanics.

## Goals / Non-Goals

**Goals:**
- Minimal `Actor` trait — `type Message;` only
- `ActorId` — location-transparent, deterministic identity
- `actor_id!` — compile-time identity macro
- Semantic lifecycle states (no execution logic)
- Semantic supervision contract (no execution logic)
- Clean domain/runtime separation

**Non-Goals:**
- `fn receive` — that's a runtime API decision
- Output semantics (`Vec<Self::Message>`, effects, replies)
- `ActorSystem`, `ActorRef`, mailbox — owned by CORE-003
- Supervision execution — owned by CORE-003
- Scheduling, dispatch, orchestration — runtime concerns

## Decisions

### Decision 1: Actor trait is minimal — `type Message` only

The domain contract declares what message type an actor accepts. How the actor processes, responds, or emits effects is a runtime concern (CORE-003). Freezing `fn receive` or output types in the domain contract would prematurely constrain the runtime.

```rust
pub trait Actor {
    type Message;
}
```

### Decision 2: `ActorId` is location-transparent

Actor identity is a logical string. It encodes nothing about location, transport, or deployment. Resolution to a physical address is owned by CORE-003. This keeps the domain contract portable across runtime implementations.

```rust
pub struct ActorId(String);
```

### Decision 3: `actor_id!` is compile-time and deterministic

Actor identities must be deterministic for replay and testing. The `actor_id!` macro produces `&'static ActorId` at compile time — no runtime construction, no dynamic identity, no ambiguity.

```rust
macro_rules! actor_id {
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

### Decision 4: Domain defines semantics, runtime enforces them

| Contract (CORE-002) | Enforcement (CORE-003) |
|---|---|
| Messages are FIFO between same sender/receiver | Mailbox implements FIFO ordering |
| One message at a time per actor | Sequential message processing |
| Supervision has restart, stop, escalate | RuntimeSupervisor executes strategies |
| Terminal states are immutable | Runtime rejects transitions from terminal |

### Decision 5: Tokio-first, never Tokio-bound

CORE-003 uses Tokio as the first runtime. CORE-002's domain contract is runtime-agnostic — no async, no tokio types, no scheduling assumptions in the domain layer.

## Risks / Trade-offs

- **[Minimalism vs. usability]** A `type Message` trait is very minimal. Application code will need runtime mechnics from CORE-003. This is intentional — the domain contract should not depend on runtime decisions.
- **[Separate specs]** CORE-002 and CORE-003 are two specs for one concept (actors). This split is necessary to enforce domain/runtime separation. A single spec would encourage boundary leakage.