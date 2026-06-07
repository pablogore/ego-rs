# Contract: `EntityRef<C, E, S>` API

**File**: `crates/persistent-entity/src/entity_ref.rs`

## Purpose

Ephemeral handle for sending commands to a specific entity. Created per command invocation by `EntityRuntime::entity_ref()`. Does NOT hold entity state.

---

## API Definition

```rust
pub struct EntityRef<C, E, S> { /* opaque */ }

impl<C: Command, E: DomainEvent, S: Send + 'static> EntityRef<C, E, S> {
    /// Send a command to the entity and await the result.
    pub async fn send(
        command: C,
        ctx: CommandContext,
        expected_version: Option<u64>,
    ) -> Result<CommandResult<E, S>, EntityError>;
}
```

## Contract Rules

### `send()`
- Enqueues the command in the entity's mailbox (if ACTIVE/RECOVERING).
- If entity is PASSIVATED, triggers automatic single-flight reactivation: entity transitions to RECOVERING, command enqueued.
- If entity is PASSIVATING, returns `EntityPassivating` error immediately.
- If mailbox is full, returns `MailboxFull` error immediately.
- If `expected_version` is `Some(v)` and does not match current stream version, returns `VersionConflict`.
- Returns `CommandResult::Events` if events were persisted, or `CommandResult::NoEvents` for zero-event commands.

### Observable behavior to caller
- Reactivation is always transparent: caller sends a command and receives a result.
- Recovery, task lifecycle, and passivation are invisible to the caller.
- If the entity does not exist and the command is not a creation command, returns `EntityNotFound`.

---

## Thread Safety

`EntityRef` is `Send` but NOT `Sync`. It is created per command invocation and consumed by `send()`.
