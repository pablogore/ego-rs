# CORE-003: Runtime Actor Execution

## Why

CORE-002 defines the domain-layer actor contract: `Actor` trait, `ActorId`, semantic lifecycle states, and supervision contract. CORE-003 owns the runtime side: `ActorSystem`, mailboxes, sequential execution, message dispatch, and supervision execution. These concerns belong together — they are the runtime mechanics of a single concern (actor execution), not separate specs for mailbox, dispatch, and supervision.

## What Changes

- Add `ActorSystem` — spawn actors by `ActorId`, deliver messages, manage lifecycle
- Add `Mailbox<Msg>` — bounded, FIFO-ordered, non-blocking send
- Add `ActorRef<Msg>` — sendable handle that delivers to the actor's mailbox
- Add `RuntimeSupervisor` — parent-child lifecycle, restart/stop/escalate execution
- Implement sequential message processing — one message at a time per actor
- Enforce communication guarantees: FIFO ordering, at-most-once delivery, message isolation

## Capabilities

### New Capabilities
- `runtime-actor-execution`: Runtime mechanics for actor spawning, message delivery, mailbox management, sequential processing, and supervision execution.

### Modified Capabilities
<!-- None -->

## Impact

- New runtime implementation lives in a new `crates/runtime/` crate
- Depends on `core-002-actor-primitive` (for `Actor` trait, `ActorId`, lifecycle states) — unidirectional
- No changes to domain layer
- Infrastructure-agnostic: mailbox and dispatch are in-memory by default, no persistence or transport