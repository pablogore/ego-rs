# Contract: ExecutionBackend

**Feature**: `006-persistent-entity-runtime/final-consistency-lock`
**Created**: 2026-06-07

The ExecutionBackend is a synchronous trait contract that abstracts the execution of pure `PersistentEntity` computations from the underlying runtime. It is an internal interface — not exposed to application developers.

## Trait Definition

```rust
use std::fmt::Debug;
use std::sync::Arc;

/// ExecutionBackend: abstracts the runtime mechanics of executing
/// a PersistentEntity's command handler.
///
/// The backend receives fully pre-decided execution requests from
/// the Actor (Execution Authority) and returns computation results.
/// It does NOT own state, decide ordering, enforce budgets, or access
/// persistence.
pub trait ExecutionBackend: Debug + Send + Sync {
    /// Execute the entity's command handler with the given inputs.
    ///
    /// This is a SYNCHRONOUS call — the command handler is pure
    /// computation with no I/O or awaiting. The caller (EntityActor)
    /// owns the async context.
    ///
    /// # Arguments
    /// * `entity` - The PersistentEntity trait object
    /// * `state` - Current entity state (before command)
    /// * `command` - The command to process
    /// * `context` - Command context (tenant, correlation, etc.)
    ///
    /// # Returns
    /// On success: `(events, new_state)` where events may be empty
    /// On business error: `EntityError` (handler returned error)
    fn execute<C, E, S>(
        &self,
        entity: &dyn PersistentEntity<Command = C, Event = E, State = S>,
        state: &S,
        command: C,
        context: &CommandContext,
    ) -> Result<(Vec<E>, S), EntityError>
    where
        C: Debug + Send + 'static,
        E: DomainEvent + Send + 'static,
        S: Clone + Debug + Send + Sync + 'static;
}
```

## Invariants

1. **Same input → Same output**: Identical `(state, command, context)` MUST produce identical `(events, state)` across all backend implementations and invocations.

2. **No semantic decisions**: The backend MUST NOT decide whether to execute, in what order to execute, or what state changes to make. It executes exactly what the Actor gives it.

3. **No state access**: The backend MUST NOT access Actor state, Scheduler state, Event Store, or any component outside the given inputs.

4. **No identity awareness**: The backend MUST NOT be aware of ExecutionKey, deduplication, or entity identity.

5. **Panic safe**: If the backend panics, the Actor catches it and transitions the entity to FAILED. The backend panic MUST NOT corrupt entity state.

## Default Implementation: TokioExecutionBackend

```rust
#[derive(Debug, Default, Clone)]
pub struct TokioExecutionBackend;

impl ExecutionBackend for TokioExecutionBackend {
    fn execute<C, E, S>(
        &self,
        entity: &dyn PersistentEntity<Command = C, Event = E, State = S>,
        state: &S,
        command: C,
        context: &CommandContext,
    ) -> Result<(Vec<E>, S), EntityError>
    where
        C: Debug + Send + 'static,
        E: DomainEvent + Send + 'static,
        S: Clone + Debug + Send + Sync + 'static,
    {
        // Create a blocking context for the sync call
        let entity_ref = entity;
        let state_clone = state.clone();
        let ctx_clone = context.clone();

        // Use Runtime::block_on if inside async context, or just call directly
        // Since the handler is pure (no await), we can block the current thread
        // The Actor is on its own task — blocking is acceptable for pure computation
        tokio::task::block_in_place(|| {
            // Actually, handlers are async (async_trait), so we need to handle that:
            // This is where the Tokio integration is needed.
            // The handler is async but has no real I/O — the async_trait is for
            // trait compatibility, not actual async behavior.
            futures::executor::block_on(
                entity_ref.handle_command(&state_clone, command, &ctx_clone)
            )
        })
        .map(|events| {
            let new_state = apply_events_sync(entity, state, &events);
            (events, new_state)
        })
    }
}
```

**Note**: The `PersistentEntity::handle_command` uses `#[async_trait]` for trait compatibility (Rust limitation). The backend bridges async → sync via `block_in_place` or `futures::executor::block_on`. Future evolution may separate the pure handler from the async wrapper.

## Test Backend: SyncTestBackend

```rust
/// Synchronous test backend — no async runtime needed
#[derive(Debug, Default, Clone)]
pub struct SyncTestBackend;

impl ExecutionBackend for SyncTestBackend {
    fn execute<C, E, S>(
        &self,
        entity: &dyn PersistentEntity<Command = C, Event = E, State = S>,
        state: &S,
        command: C,
        context: &CommandContext,
    ) -> Result<(Vec<E>, S), EntityError>
    where
        C: Debug + Send + 'static,
        E: DomainEvent + Send + 'static,
        S: Clone + Debug + Send + Sync + 'static,
    {
        // Same pattern as Tokio — but no async runtime overhead
        // This backend is for unit tests that need deterministic execution
        futures::executor::block_on(
            entity.handle_command(state, command, context)
        )
        .map(|events| {
            let new_state = apply_events_sync(entity, state, &events);
            (events, new_state)
        })
    }
}
```
