# Contract: `EntityRuntime` and `EntityRuntimeBuilder`

**File**: `crates/persistent-entity/src/runtime.rs`

## Purpose

Central lifecycle manager. Holds all persistence backends, configuration, and the active/passivated entity registries.

---

## Public API

```rust
pub struct EntityRuntime { /* opaque */ }

impl EntityRuntime {
    pub fn entity_ref<C, E, S>(
        &self,
        tenant_id: TenantId,
        entity_type: &'static str,
        entity_id: EntityId,
    ) -> EntityRef<C, E, S>;
}
```

## Builder

```rust
pub struct EntityRuntimeBuilder { /* opaque */ }

impl EntityRuntimeBuilder {
    pub fn new() -> Self;

    /// Set the bounded mailbox capacity per entity. Default: 1000.
    pub fn mailbox_capacity(mut self, cap: usize) -> Self;

    /// Set global concurrency budget (max ACTIVE tasks). Default: 10000.
    pub fn concurrency_budget(mut self, budget: usize) -> Self;

    /// Set inactivity timeout before passivation. Default: 5 minutes.
    pub fn passivation_timeout(mut self, timeout: Duration) -> Self;

    /// Set snapshot strategy. Default: every 100 events.
    pub fn snapshot_strategy(mut self, strategy: Box<dyn SnapshotStrategy>) -> Self;

    /// Set the event store backend (required).
    pub fn with_event_store(mut self, store: Box<dyn EventStore>) -> Self;

    /// Set the snapshot store backend (required).
    pub fn with_snapshot_store(mut self, store: Box<dyn Snapshot>) -> Self;

    /// Set the event publisher. Default: no-op.
    pub fn with_publisher(mut self, publisher: Box<dyn EventPublisher>) -> Self;

    /// Build the runtime. Panics if required fields are missing.
    pub fn build(self) -> EntityRuntime;
}
```

## Contract Rules

### `EntityRuntime`
- Single instance per application. Created once at startup.
- Manages the passivation registry and active entity actor map.
- Routes commands to the correct entity via `entity_ref()`.
- Scoped by entity triple `(TenantId, EntityType, EntityId)`.

### `EntityRuntimeBuilder`
- All setters are optional except `with_event_store()` and `with_snapshot_store()`.
- Defaults are chosen for general-purpose use. Tune per deployment.
- `build()` consumes the builder. Runtime is immutable after construction.

---

## Thread Safety

`EntityRuntime` is `Send + Sync` (uses internal synchronization for the registry). `entity_ref()` is cheap — returns a new `EntityRef` containing a cloned mpsc sender and entity identity.
