# CORE-002: Actor Primitive

## Why

ego-rs needs an actor model as its central behavioral abstraction — a contract that defines what an actor is without coupling to how it executes. The domain layer owns the contract. The runtime layer (CORE-003) owns the execution. This separation prevents premature API freezing (no `fn receive`, no output semantics, no effects) and keeps the domain crate infrastructure-free.

## What Changes

- Add `Actor` trait in `crates/domain/` — minimal: `type Message;` only
- Add `ActorId` — newtype over String, non-empty validation, location-transparent
- Add `actor_id!` macro — compile-time deterministic identity
- Add `ActorLifecycleState` enum — semantic states (no execution logic)
- Add `SupervisionStrategy` enum — semantic strategy (no execution)
- Rust docs on all public items

### Domain owns:
- `Actor` trait, `ActorId`, `actor_id!`
- Semantic lifecycle states
- Semantic supervision contract
- Communication guarantees (what, not how)

### Domain does NOT own:
- `ActorSystem`, `ActorRef`, `Mailbox` → CORE-003
- Supervision execution → CORE-003
- Scheduling, dispatch, orchestration → CORE-003

## Capabilities

### New Capabilities
- `actor-primitive`: Minimal domain contract for actor identity, messaging, lifecycle, and supervision semantics.

### Modified Capabilities
<!-- None -->

## Impact

- New types in `crates/domain/src/actor/` — no new crate
- Zero runtime dependencies in domain crate (no tokio, no async)
- CORE-003 depends on CORE-002 — unidirectional domain→runtime