# Contract: `PersistentEntity<C, E, S>` Trait

**File**: `crates/persistent-entity/src/persistent_entity.rs`

## Purpose

Defines the user-facing contract for implementing an event-sourced persistent entity. Developers implement this trait for each domain entity type.

---

## Trait Definition

```rust
#[async_trait]
pub trait PersistentEntity<C: Command, E: DomainEvent, S: Send + 'static> {
    type Error: std::error::Error + Send + 'static;

    fn initial_state() -> S;

    async fn handle_command(
        state: &S,
        command: C,
        ctx: CommandContext,
    ) -> Result<Vec<E>, Self::Error>;

    async fn apply_event(state: &S, event: E) -> S;
}
```

## Contract Rules

### `initial_state()`
- MUST return the same value on every call.
- MUST NOT perform side effects.
- Called during recovery to initialize state before replay.

### `handle_command()`
- MUST be a pure function: same inputs always produce same outputs.
- MUST NOT access clock, network, filesystem, random, or global state.
- MUST produce events or return an error — never both.
- If it returns `Ok(events)`, the events are persisted to the event store.
- If it returns `Err`, no event is persisted (business rule violation).

### `apply_event()`
- MUST be deterministic: same (state, event) always produces same new state.
- MUST NOT produce side effects, emit events, or access external systems.
- Called during both original execution AND recovery replay.
- Called in strict event sequence order (seq 1, 2, 3, ...).

### `Error` type
- MUST be the error type for business rule violations only.
- Runtime errors (EntityNotFound, VersionConflict) are NOT part of this type.

---

## Design Notes

- `handle_command` and `apply_event` are `async` to allow framework-level instrumentation, not I/O. User implementations MUST NOT perform I/O.
- The trait uses associated generics (`C`, `E`, `S`) rather than trait-wide generics to allow type inference at the call site (each entity type has one command, one event, one state type).
