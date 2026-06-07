# Contracts: Activation Ordering Model

**Date**: 2026-06-07  
**Source**: Existing `crates/persistent-entity/src/` implementations

## Contract: `PersistentEntity<C, E, S>`

```rust
#[async_trait]
pub trait PersistentEntity<C: Send + 'static, E: Send + 'static, S: Send + 'static>:
    Send + Sync
{
    async fn handle_command(&self, state: &S, command: C, ctx: CommandContext) -> Result<Vec<E>, String>;
    async fn apply_event(&self, state: &S, event: E) -> S;
    fn initial_state(&self) -> S;
}
```

**Source**: `crates/persistent-entity/src/persistent_entity.rs`

**Contracts**:
- `handle_command` must be deterministic (same state + command → same events)
- `apply_event` must be deterministic (same state + event → same new state)
- `initial_state` must return the same value on every call for a given entity type

---

## Contract: `EventPublisher<E>`

```rust
#[async_trait]
pub trait EventPublisher<E>: Send + Sync {
    async fn publish(&self, events: &[E]) -> Result<(), ()>;
}
```

**Source**: `crates/persistent-entity/src/publisher.rs`

**Contracts**:
- Must not panic
- Failure is non-fatal (logged, caller continues)
- Must be idempotent if retried

---

## Contract: `EventStore<E>` (domain SPI)

```rust
pub trait EventStore<E: DomainEvent>: Send {
    fn append(&mut self, aggregate_id: &str, tenant_id: Option<&str>,
              expected_version: i64, events: Vec<StoredEvent<E>>) -> Result<i64, PersistenceError>;
    fn load(&self, aggregate_id: &str, tenant_id: Option<&str>)
            -> Result<Vec<StoredEvent<E>>, PersistenceError>;
    fn list_aggregate_ids(&self, tenant_id: Option<&str>) -> Result<Vec<String>, PersistenceError>;
}
```

**Source**: `crates/domain/src/persistence/`

**Contracts**:
- `append` must reject if `expected_version ≠ current_version` (optimistic concurrency)
- `load` must return events in append order
- Must be consistent within a single actor's recovery window (no partial writes)

---

## Contract: `Snapshot` (domain SPI)

```rust
pub trait Snapshot: Send {
    fn save_snapshot(&mut self, aggregate_id: &str, tenant_id: Option<&str>,
                     version: i64, payload: Value) -> Result<(), PersistenceError>;
    fn load_snapshot(&self, aggregate_id: &str, tenant_id: Option<&str>)
                     -> Result<Option<(i64, Value)>, PersistenceError>;
}
```

**Source**: `crates/domain/src/persistence/`

**Contracts**:
- `save_snapshot` overwrites any existing snapshot for the same aggregate
- `load_snapshot` returns the latest snapshot (highest version)
- Snapshot payload must be self-contained (no references to event stream)

---

## Contract: `SnapshotStrategy`

```rust
pub trait SnapshotStrategy: Send + Sync {
    fn should_snapshot(&self, version: u64) -> bool;
    fn clone_boxed(&self) -> Box<dyn SnapshotStrategy>;
}
```

**Source**: `crates/persistent-entity/src/snapshot.rs`

**Contracts**:
- `should_snapshot` must be deterministic (same version → same result)
- `clone_boxed` must produce an independent copy with identical behavior

---

## Contract: `StateRecovery`

```rust
#[async_trait]
pub trait StateRecovery: Send + 'static {
    type State: Send + 'static;
    type Event: DomainEvent + Clone + serde::de::DeserializeOwned + 'static;
    async fn recover(&self, persistence: &PersistenceFacade<Self::Event>,
                     aggregate_id: &str, tenant_id: Option<&str>)
                     -> Result<(Self::State, u64), EntityError>;
}
```

**Source**: `crates/persistent-entity/src/recovery.rs`

**Contracts**:
- Must return deterministic state given the same event stream
- Recovery failure must return `EntityError` explaining the cause
