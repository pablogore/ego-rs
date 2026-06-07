# Data Model: Persistent Entity Runtime

**Feature**: `006-persistent-entity-runtime`
**Date**: 2026-06-07
**Status**: Draft

## Overview

Core entities and value types for the persistent entity runtime. Types prefixed with `pub` form the public API; un-prefixed types are internal.

---

## 1. Entity Identity (Reuse from `ego-domain`)

```rust
// Already defined in ego-domain — reused, NOT redefined:
pub struct TenantId(String);
pub struct EntityId(String);
// Entity type is represented as a &'static str or TypeId at registration
```

**Entity triple**: `(TenantId, EntityType, EntityId)` — unique identifier scoped to a single entity stream.

---

## 2. Public API Types

### `PersistentEntity<C, E, S>` trait

```rust
#[async_trait]
pub trait PersistentEntity<C: Command, E: DomainEvent, S: Send + 'static> {
    /// The initial state before any events exist.
    fn initial_state() -> S;

    /// Given current state and a command, produce events or return an error.
    /// Pure function — no side effects.
    async fn handle_command(
        state: &S,
        command: C,
        ctx: CommandContext,
    ) -> Result<Vec<E>, Self::Error>;

    /// Given current state and an event, produce the next state.
    /// Pure function — no side effects.
    async fn apply_event(
        state: &S,
        event: E,
    ) -> S;

    /// The error type for command handler failures.
    type Error: std::error::Error + Send + 'static;
}
```

### `CommandContext`

```rust
#[derive(Clone, Debug)]
pub struct CommandContext {
    pub tenant_id: TenantId,
    pub correlation_id: CorrelationId,
    pub causation_id: CausationId,
    pub approval_id: Option<ApprovalId>,
    pub metadata: HashMap<String, String>,
}
```

### `EntityRef<C, E, S>`

```rust
/// An ephemeral handle used to send a single command to an entity.
/// Created per command invocation. Does NOT hold entity state.
pub struct EntityRef<C, E, S> {
    // Internal: mpsc sender, entity triple, expected version
}

impl<C, E, S> EntityRef<C, E, S> {
    /// Send a command and await the result.
    pub async fn send(
        command: C,
        ctx: CommandContext,
    ) -> Result<CommandResult<E, S>, EntityError>;
}
```

### `CommandResult<E, S>`

```rust
pub enum CommandResult<E, S> {
    /// Command produced events (mutation).
    Events {
        events: Vec<E>,
        new_state: S,
        new_version: u64,
    },
    /// Command produced no events (zero-event/query).
    NoEvents {
        state: S,
    },
}
```

---

## 3. Runtime Types

### `EntityRuntime`

```rust
/// Central lifecycle manager. Holds the passivation registry, active entity map,
/// persistence backends, and configuration.
pub struct EntityRuntime<E, S> {
    // Internal fields
}

impl EntityRuntime {
    /// Obtain an EntityRef to send a command to a specific entity.
    pub fn entity_ref(
        &self,
        tenant_id: TenantId,
        entity_type: &'static str,
        entity_id: EntityId,
        expected_version: Option<u64>,
    ) -> EntityRef;
}
```

### `EntityRuntimeBuilder`

```rust
pub struct EntityRuntimeBuilder {
    mailbox_capacity: usize,
    concurrency_budget: usize,
    passivation_timeout: Duration,
    snapshot_strategy: Box<dyn SnapshotStrategy>,
    event_store: Box<dyn EventStore>,
    snapshot_store: Box<dyn Snapshot>,
    publisher: Box<dyn EventPublisher>,
}

impl EntityRuntimeBuilder {
    pub fn new() -> Self;
    pub fn mailbox_capacity(mut self, cap: usize) -> Self;
    pub fn concurrency_budget(mut self, budget: usize) -> Self;
    pub fn passivation_timeout(mut self, timeout: Duration) -> Self;
    pub fn snapshot_strategy(mut self, strategy: impl SnapshotStrategy) -> Self;
    pub fn with_event_store(mut self, store: impl EventStore) -> Self;
    pub fn with_snapshot_store(mut self, store: impl Snapshot) -> Self;
    pub fn with_publisher(mut self, publisher: impl EventPublisher) -> Self;
    pub fn build() -> EntityRuntime;
}
```

---

## 4. Internal Types

### `LifecycleState`

```rust
#[derive(Debug, Clone, PartialEq)]
enum LifecycleState {
    /// Entity is loading snapshot + replaying events. No command processing.
    Recovering,
    /// Fully operational. Commands drained from mailbox and executed.
    Active,
    /// Mailbox frozen. Current command + drain in progress.
    Passivating,
    /// No in-memory state, no running task. Registry entry only.
    Passivated,
    /// Irrecoverable error. On-demand recovery only.
    Failed,
}
```

### `EntityMailbox`

```rust
/// Bounded FIFO mailbox backed by Tokio mpsc.
struct EntityMailbox<C> {
    sender: mpsc::Sender<CommandEnvelope<C>>,
    receiver: mpsc::Receiver<CommandEnvelope<C>>,
    capacity: usize,
}
```

### `CommandEnvelope<C>`

```rust
/// Internal wrapper around a user command with signaling.
struct CommandEnvelope<C> {
    command: C,
    ctx: CommandContext,
    response_tx: oneshot::Sender<Result<CommandResult, EntityError>>,
    expected_version: Option<u64>,
}
```

### `PassivationRegistry`

```rust
/// Lightweight in-memory registry tracking passivated entities.
struct PassivationRegistry {
    entries: HashMap<EntityTriple, PassivationEntry>,
    /// Per-entity Mutex for single-flight reactivation guard.
    /// Constitution §5 forbids CAS — Mutex is used instead.
    spawn_locks: HashMap<EntityTriple, Arc<tokio::sync::Mutex<()>>>,
}

struct PassivationEntry {
    last_known_version: u64,
    passivated_at: Instant,
}
```

### `EntityTriple`

```rust
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
struct EntityTriple {
    tenant_id: TenantId,
    entity_type: &'static str,
    entity_id: EntityId,
}
```

---

## 5. Error Types

### `EntityError`

```rust
#[derive(Debug, thiserror::Error)]
pub enum EntityError {
    #[error("entity not found: {0}")]
    EntityNotFound(EntityId),

    #[error("version conflict: expected {expected}, current {current}")]
    VersionConflict { expected: u64, current: u64 },

    #[error("entity is passivating, retry later")]
    EntityPassivating,

    #[error("mailbox at capacity ({0})")]
    MailboxFull(usize),

    #[error("reentrancy not allowed")]
    ReentrancyNotAllowed,

    #[error("handler error: {0}")]
    Handler(Box<dyn std::error::Error + Send>),

    #[error("runtime error: {0}")]
    Runtime(String),
}
```

---

## 6. SPI Traits (Implementation Contracts)

### `EventPublisher`

```rust
#[async_trait]
pub trait EventPublisher<E>: Send + Sync {
    /// Publish committed events to downstream consumers.
    /// Called after event commit. Failure is logged, NOT propagated.
    async fn publish(&self, events: &[StoredEvent<E>]);
}
```

### `SnapshotStrategy`

```rust
#[async_trait]
pub trait SnapshotStrategy: Send + Sync {
    /// Determine whether a snapshot should be taken after the given event.
    fn should_snapshot(&self, new_version: u64) -> bool;
}
```

---

## 7. State Relationships

```text
EntityRuntime
├── owns: PassivationRegistry (HashMap<EntityTriple, PassivationEntry>)
├── owns: ActiveEntityMap (HashMap<EntityTriple, ActorHandle>)
├── owns: EventStore (from ego-domain)
├── owns: SnapshotStore (from ego-domain)
├── owns: EventPublisher (custom SPI)
└── config: MailboxCapacity, ConcurrencyBudget, PassivationTimeout

EntityActor (per ACTIVE/RECOVERING entity)
├── owns: EntityMailbox (bounded mpsc)
├── owns: InMemoryState (S)
├── owns: LifecycleState
└── owns: StreamVersion (u64)

EntityRef (per command invocation)
└── holds: mpsc::Sender (clone), EntityTriple, ExpectedVersion
```

---

## 8. Lifecycle State Transitions

```text
[NotExist] ────(creation command)────> [Recovering] ──(recovery ok)──> [Active]
     ▲                                      │                              │
     │                                      │ (recovery fails)             │ (passivation trigger)
     │                                      ▼                              ▼
     │                                  [Failed]                      [Passivating]
     │                                      │                              │
     │                                      │ (on-demand recovery)         │ (drain complete)
     │                                      ▼                              ▼
     │                              [Recovering] <─── [Passivated] <────────┘
     │                                      │
     └──────────────────────────────────────┘
           (non-creation command → EntityNotFound)

[Active] ──(runtime error)──> [Failed]
```

Transitions defined in FR-021, FR-022, FR-023, and spec §3 Entity Lifecycle Model.
