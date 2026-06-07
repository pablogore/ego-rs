# Runtime Architecture Blueprint: Persistent Entity Runtime

**Feature**: `006-persistent-entity-runtime`
**Date**: 2026-06-07
**Status**: Architecture Draft

**Purpose**: Directly implementable Rust architecture blueprint for the Persistent Entity Runtime, covering module structure, core types, actor execution loop, activation system, mailbox model, and persistence flow.

---

## 1. Module Structure

```
crates/persistent-entity/
├── Cargo.toml
├── src/
│   ├── lib.rs                          # Crate root, public re-exports
│   │
│   ├── runtime.rs                      # EntityRuntime: lifecycle manager
│   ├── builder.rs                      # EntityRuntimeBuilder
│   │
│   ├── registry.rs                     # EntityRegistry: active + passivated tracking
│   ├── activation.rs                   # ActivationFuture: single-flight coordination
│   │
│   ├── actor.rs                        # EntityActor: Tokio task loop
│   ├── mailbox.rs                      # Bounded FIFO (mpsc wrapper)
│   │
│   ├── lifecycle.rs                    # LifecycleStateMachine (5 states)
│   ├── supervisor.rs                   # Failure + recovery orchestration
│   ├── scheduler.rs                    # Global concurrency budget scheduler
│   │
│   ├── recovery.rs                     # State recovery: snapshot + replay
│   ├── snapshot.rs                     # SnapshotStrategy trait + built-ins
│   │
│   ├── entity_ref.rs                   # EntityRef: per-command sender handle
│   ├── persistent_entity.rs            # PersistentEntity trait (user-facing)
│   ├── command_context.rs              # CommandContext value type
│   │
│   ├── persistence.rs                  # Persistence facade (EventStore + Snapshot + Outbox)
│   ├── publisher.rs                    # EventPublisher SPI
│   │
│   ├── error.rs                        # EntityError enum
│   └── testing.rs                      # Test helpers, mock backends
│
└── tests/
    ├── entity_lifecycle.rs
    ├── concurrency.rs
    ├── recovery.rs
    ├── passivation.rs
    ├── activation.rs
    └── version_conflict.rs
```

### Dependency Flow

```text
persistent_entity.rs   entity_ref.rs   command_context.rs
         |                   |
         v                   v
    actor.rs  <──>  mailbox.rs
         |
         v
   lifecycle.rs
         |
     supervisor.rs
         |
    recovery.rs  ──>  snapshot.rs
         |
  persistence.rs  ──>  publisher.rs
         |
registry.rs  ──>  activation.rs
         |
  scheduler.rs
         |
   runtime.rs  <──  builder.rs

error.rs  ──>  (used by all public types)
```

---

## 2. Core Structs and Traits

### 2.1 `PersistentEntity` Trait (User-Facing)

```rust
/// Implemented by application developers for each domain entity.
#[async_trait]
pub trait PersistentEntity<C: Command, E: DomainEvent, S: Send + 'static> {
    type Error: std::error::Error + Send + 'static;

    /// Initial state when no events exist.
    fn initial_state() -> S;

    /// Pure function: (state, command, ctx) -> events | error.
    /// MUST NOT access persistence, clock, network, or global state.
    async fn handle_command(
        state: &S,
        command: C,
        ctx: CommandContext,
    ) -> Result<Vec<E>, Self::Error>;

    /// Pure function: (state, event) -> new_state.
    /// MUST NOT produce side effects.
    async fn apply_event(state: &S, event: E) -> S;
}
```

### 2.2 `EntityRuntime` and Builder

```rust
/// Central lifecycle manager. Created once at application startup.
/// Holds registry, persistence backends, scheduler, and configuration.
pub struct EntityRuntime {
    registry: Arc<EntityRegistry>,
    scheduler: Arc<Scheduler>,
    persistence: Arc<PersistenceFacade>,
    publisher: Arc<dyn EventPublisher>,
    config: RuntimeConfig,
}

impl EntityRuntime {
    /// Obtain a command sender handle for a specific entity.
    pub fn entity_ref<C, E, S>(
        &self,
        tenant_id: TenantId,
        entity_type: &'static str,
        entity_id: EntityId,
    ) -> EntityRef<C, E, S>;
}

pub struct EntityRuntimeBuilder {
    mailbox_capacity: usize,        // default: 1000
    concurrency_budget: usize,      // default: 10000
    passivation_timeout: Duration,  // default: 5 min
    event_store: Option<Box<dyn EventStore>>,
    snapshot_store: Option<Box<dyn Snapshot>>,
    publisher: Option<Box<dyn EventPublisher>>,
    snapshot_strategy: Option<Box<dyn SnapshotStrategy>>,
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
    pub fn build(self) -> EntityRuntime;
}
```

### 2.3 `EntityRef` — Per-Command Sender Handle

```rust
/// Ephemeral handle created per command invocation.
/// Holds a clone of the mpsc sender and entity identity.
pub struct EntityRef<C, E, S> {
    entity_id: EntityTriple,
    sender: mpsc::Sender<CommandEnvelope<C>>,
    // If sender is stale (channel closed), runtime triggers reactivation.
}

impl<C, E, S> EntityRef<C, E, S> {
    /// Send a command. If entity is PASSIVATED, triggers single-flight reactivation.
    pub async fn send(
        mut self,
        command: C,
        ctx: CommandContext,
        expected_version: Option<u64>,
    ) -> Result<CommandResult<E, S>, EntityError>;
}
```

### 2.4 `CommandEnvelope` — Internal Envelope

```rust
struct CommandEnvelope<C> {
    command: C,
    ctx: CommandContext,
    response_tx: oneshot::Sender<Result<CommandResult, EntityError>>,
    expected_version: Option<u64>,
}
```

### 2.5 `EntityActor` — The Tokio Actor Task

```rust
struct EntityActor<C, E, S> {
    entity_id: EntityTriple,
    mailbox: mpsc::Receiver<CommandEnvelope<C>>,
    state: Option<S>,
    version: u64,
    lifecycle: LifecycleStateMachine,
    registry: Arc<EntityRegistry>,
    persistence: Arc<PersistenceFacade>,
    publisher: Arc<dyn EventPublisher>,
    snapshot_strategy: Box<dyn SnapshotStrategy>,
    phantom: PhantomData<(C, E, S)>,
}

impl<C, E, S> EntityActor<C, E, S>
where
    C: Command,
    E: DomainEvent,
    S: Debug + Clone + Send + 'static,
{
    /// The main actor execution loop.
    async fn run(mut self);
}
```

### 2.6 `EntityRegistry` — Active + Passivated Tracking

```rust
struct EntityRegistry {
    /// Active actors: entity triple → actor handle (sender + join handle)
    active: Arc<DashMap<EntityTriple, ActorHandle>>,
    /// Passivation entries: entity triple → last known version
    passivated: Arc<DashMap<EntityTriple, PassivationEntry>>,
    /// Activation coordination: entity triple → shared activation future
    pending_activations: Arc<DashMap<EntityTriple, SharedActivation>>,
}

struct ActorHandle {
    sender: mpsc::Sender<CommandEnvelope>,
    join: JoinHandle<()>,
}

struct PassivationEntry {
    last_known_version: u64,
    passivated_at: Instant,
}

struct SharedActivation {
    /// Mutex-based single-flight guard per constitution §5 (CAS forbidden).
    lock: tokio::sync::Mutex<()>,
    future: tokio::sync::watch::Sender<Option<EntityError>>,
}
```

### 2.7 `EntityTriple` — Identity Key

```rust
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
struct EntityTriple {
    tenant_id: TenantId,
    entity_type: &'static str,
    entity_id: EntityId,
}
```

### 2.8 `EntityError` — Typed Error Enum

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

### 2.9 `CommandResult` — Return Value

```rust
pub enum CommandResult<E, S> {
    Events { events: Vec<E>, new_state: S, new_version: u64 },
    NoEvents { state: S },
}
```

### 2.10 `LifecycleStateMachine`

```rust
#[derive(Debug, Clone, PartialEq)]
enum LifecycleState {
    Recovering,
    Active,
    Passivating,
    Passivated,
    Failed,
}

struct LifecycleStateMachine {
    state: LifecycleState,
    // Timestamp for passivation timeout tracking
    entered_active_at: Option<Instant>,
}

impl LifecycleStateMachine {
    fn transition_to(&mut self, new: LifecycleState) -> Result<(), ()>;
    fn can_accept_commands(&self) -> bool;
    fn should_passivate(&self, timeout: Duration) -> bool;
}
```

### 2.11 `Scheduler` — Concurrency Budget

```rust
struct Scheduler {
    semaphore: Arc<tokio::sync::Semaphore>,
    /// Tracks pending entities for FIFO ordering with anti-starvation
    pending_queue: Arc<Mutex<VecDeque<EntityTriple>>>,
}
```

### 2.12 `PersistenceFacade` — Unified Persistence

```rust
struct PersistenceFacade {
    event_store: Box<dyn EventStore>,
    snapshot_store: Box<dyn Snapshot>,
}

impl PersistenceFacade {
    /// Load snapshot + events for recovery.
    async fn load_for_recovery<E>(
        &self,
        entity_id: &EntityTriple,
    ) -> Result<(Option<SnapshotData>, Vec<StoredEvent<E>>), PersistenceError>;

    /// Persist events atomically with version check.
    async fn persist_events<E>(
        &self,
        entity_id: &EntityTriple,
        expected_version: u64,
        events: &[E],
        metadata: EventMetadata,
    ) -> Result<u64, PersistenceError>;

    /// Store snapshot at given version.
    async fn store_snapshot<S: Serialize>(
        &self,
        entity_id: &EntityTriple,
        version: u64,
        state: &S,
    ) -> Result<(), PersistenceError>;
}
```

---

## 3. Tokio Actor Execution Loop

### 3.1 Actor Entry Point (spawned by Runtime Engine)

```rust
impl<C, E, S> EntityActor<C, E, S> {
    async fn run(mut self) {
        // Phase 1: Recovery
        self.lifecycle.transition_to(LifecycleState::Recovering).ok();
        match self.recover_state().await {
            Ok(()) => {
                self.lifecycle.transition_to(LifecycleState::Active).ok();
            }
            Err(e) => {
                self.lifecycle.transition_to(LifecycleState::Failed).ok();
                self.notify_queued_commands(Err(e)).await;
                self.registry.remove_active(&self.entity_id).await;
                return;
            }
        }

        // Phase 2: Command processing loop
        loop {
            tokio::select! {
                // Process next command from mailbox
                Some(envelope) = self.mailbox.recv() => {
                    if self.lifecycle.state() == LifecycleState::Passivating {
                        // Drain remaining commands during passivation
                        self.execute_command(envelope).await;
                        continue;
                    }

                    self.execute_command(envelope).await;

                    // Check passivation timeout
                    if self.lifecycle.should_passivate(self.config.passivation_timeout) {
                        break;
                    }
                }
                // Passivation timer
                _ = self.passivation_timer() => {
                    break;
                }
            }
        }

        // Phase 3: Passivation
        self.lifecycle.transition_to(LifecycleState::Passivating).ok();
        // Drain remaining mailbox
        while let Some(envelope) = self.mailbox.recv().await {
            self.execute_command(envelope).await;
        }
        // Snapshot final state if needed
        if let Some(state) = &self.state {
            let _ = self.persistence
                .store_snapshot(&self.entity_id, self.version, state)
                .await;
        }
        self.registry
            .mark_passivated(self.entity_id.clone(), self.version)
            .await;
        self.lifecycle.transition_to(LifecycleState::Passivated).ok();
    }
}
```

### 3.2 Command Execution

```rust
impl<C, E, S> EntityActor<C, E, S> {
    async fn execute_command(&mut self, envelope: CommandEnvelope<C>) {
        let state = match &self.state {
            Some(s) => s.clone(),
            None => return, // should not happen after recovery
        };

        // Execute handler (pure function, no I/O)
        let result = PersistentEntity::<C, E, S>::handle_command(
            &state,
            envelope.command,
            envelope.ctx.clone(),
        ).await;

        let response = match result {
            Ok(events) if events.is_empty() => {
                // Zero-event command: no persistence
                Ok(CommandResult::NoEvents { state })
            }
            Ok(events) => {
                // Persist events atomically with version check
                let metadata = EventMetadata::from(&envelope.ctx);
                match self.persistence
                    .persist_events(&self.entity_id, envelope.expected_version, &events, metadata)
                    .await
                {
                    Ok(new_version) => {
                        // Apply events to in-memory state
                        let mut new_state = state;
                        for event in &events {
                            new_state = PersistentEntity::<C, E, S>::apply_event(
                                &new_state, event.clone()
                            ).await;
                        }
                        self.state = Some(new_state.clone());
                        self.version = new_version;

                        // Snapshot if strategy triggers
                        if self.snapshot_strategy.should_snapshot(new_version) {
                            let _ = self.persistence
                                .store_snapshot(&self.entity_id, new_version, &new_state)
                                .await;
                        }

                        // Publish events (best-effort)
                        // (publication is async, failure logged only)
                        let _ = self.publisher.publish(&events).await;

                        Ok(CommandResult::Events {
                            events,
                            new_state,
                            new_version,
                        })
                    }
                    Err(PersistenceError::Conflict) => {
                        Err(EntityError::VersionConflict {
                            expected: envelope.expected_version.unwrap_or(0),
                            current: self.version,
                        })
                    }
                    Err(e) => {
                        self.lifecycle.transition_to(LifecycleState::Failed).ok();
                        Err(EntityError::Runtime(e.to_string()))
                    }
                }
            }
            Err(handler_error) => {
                // Business error: no event persisted
                Err(EntityError::Handler(handler_error.into()))
            }
        };

        // Send response back to caller
        let _ = envelope.response_tx.send(response);
    }

    async fn recover_state(&mut self) -> Result<(), EntityError> {
        let (snapshot, events) = self.persistence
            .load_for_recovery::<E>(&self.entity_id)
            .await
            .map_err(|e| EntityError::Runtime(e.to_string()))?;

        // Start from snapshot state (or initial state)
        let mut state = match snapshot {
            Some(snap) => bincode::deserialize(&snap.data)
                .map_err(|e| EntityError::Runtime(e.to_string()))?,
            None => PersistentEntity::<C, E, S>::initial_state(),
        };
        let mut version = snapshot.map(|s| s.version).unwrap_or(0);

        // Replay events after snapshot
        for stored in &events {
            if stored.seq <= version {
                continue; // already included in snapshot or duplicate
            }
            state = PersistentEntity::<C, E, S>::apply_event(
                &state, stored.event.clone()
            ).await;
            version = stored.seq;
        }

        self.state = Some(state);
        self.version = version;
        Ok(())
    }
}
```

---

## 4. Registry + Activation System

### 4.1 Command Dispatch Path

```text
┌──────────────┐
│   Caller     │
│  EntityRef   │
│   .send()    │
└──────┬───────┘
       │ try_send to mpsc
       ▼
┌──────────────┐     success     ┌──────────────┐
│  try_send    │ ──────────────► │   Mailbox     │
│  to actor    │                 │  (mpsc rx)    │
│  mpsc tx     │                 └──────────────┘
└──────┬───────┘
       │ error: channel closed
       ▼
┌──────────────┐
│  Registry    │  Entity is PASSIVATED
│  .route()    │
└──────┬───────┘
       │
       ▼
┌──────────────────────┐
│  Activation System   │
│  (single-flight)     │
└──────┬───────────────┘
       │
       ├── First caller: acquires Mutex, spawns actor
       │   Subsequent callers: await same
       │   SharedActivation future
       │
       ▼
┌──────────────┐
│   Actor      │
│   spawned    │
│  + mailbox   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Mailbox    │
│   (now rx)   │
│   receives   │
│   command    │
└──────────────┘
```

### 4.2 Activation Future Implementation

```rust
impl EntityRegistry {
    async fn route_command<C>(
        &self,
        entity_id: EntityTriple,
        envelope: CommandEnvelope<C>,
    ) -> Result<(), EntityError> {
        // 1. Check if active actor exists
        if let Some(handle) = self.active.get(&entity_id) {
            return handle.sender.send(envelope)
                .map_err(|_| EntityError::Runtime("actor dropped".into()));
        }

        // 2. Check if entity is known (passivated or existing)
        let is_known = self.passivated.contains_key(&entity_id)
            || self.active.contains_key(&entity_id);

        if !is_known {
            // Entity does not exist — non-creation commands fail
            // (creation commands are handled separately)
            return Err(EntityError::EntityNotFound(entity_id.entity_id));
        }

        // 3. Single-flight activation: acquire per-entity Mutex
        let activation = self.pending_activations
            .entry(entity_id.clone())
            .or_insert_with(|| SharedActivation::new());

        let mut guard = activation.lock.lock().await;

        // Double-check: another task may have spawned while we waited
        if let Some(handle) = self.active.get(&entity_id) {
            drop(guard);
            return handle.sender.send(envelope)
                .map_err(|_| EntityError::Runtime("actor dropped".into()));
        }

        // 4. We are the designated spawner
        let (mailbox_tx, mailbox_rx) = mpsc::channel(self.config.mailbox_capacity);
        let handle = tokio::spawn(Self::actor_task(
            entity_id.clone(),
            mailbox_rx,
            self.clone(),
        ));

        self.active.insert(entity_id.clone(), ActorHandle {
            sender: mailbox_tx.clone(),
            join: handle,
        });

        // Remove passivation entry
        self.passivated.remove(&entity_id);

        // Clear pending activation entry (future resolved)
        self.pending_activations.remove(&entity_id);

        // Release activation lock
        drop(guard);

        // 5. Deliver command to the now-active mailbox
        mailbox_tx.send(envelope)
            .map_err(|_| EntityError::Runtime("mailbox full".into()))
    }

    async fn actor_task<C, E, S>(
        entity_id: EntityTriple,
        mailbox_rx: mpsc::Receiver<CommandEnvelope<C>>,
        registry: Arc<EntityRegistry>,
    ) {
        // Build actor from persisted state
        let actor = EntityActor {
            entity_id,
            mailbox: mailbox_rx,
            state: None,
            version: 0,
            lifecycle: LifecycleStateMachine::new(),
            registry,
            persistence: /* from config */,
            publisher: /* from config */,
            snapshot_strategy: /* from config */,
            phantom: PhantomData,
        };
        actor.run().await;
    }
}
```

### 4.3 Registry API

```rust
impl EntityRegistry {
    /// Create new registry with given configuration.
    fn new(config: RuntimeConfig) -> Self;

    /// Route a command to the correct entity.
    async fn route<C>(&self, entity: EntityTriple, envelope: CommandEnvelope<C>)
        -> Result<(), EntityError>;

    /// Mark an entity as passivated (called by actor on shutdown).
    async fn mark_passivated(&self, entity: EntityTriple, version: u64);

    /// Remove active actor entry (called on actor failure or shutdown).
    fn remove_active(&self, entity: &EntityTriple);

    /// Check if an entity exists (passivated or active).
    fn exists(&self, entity: &EntityTriple) -> bool;
}
```

---

## 5. Mailbox Model

### 5.1 Mailbox Implementation

```rust
/// Wraps Tokio mpsc channel with passivation-aware semantics.
struct Mailbox<C> {
    sender: mpsc::Sender<CommandEnvelope<C>>,
}

impl<C> Mailbox<C> {
    fn new(capacity: usize) -> (Self, mpsc::Receiver<CommandEnvelope<C>>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Mailbox { sender: tx }, rx)
    }

    /// Send a command. Returns error if mailbox is full or channel closed.
    fn try_send(&self, envelope: CommandEnvelope<C>)
        -> Result<(), TrySendError<CommandEnvelope<C>>>
    {
        self.sender.try_send(envelope)
    }

    /// Check if the mailbox is closed (entity passivated/failed).
    fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    fn sender(&self) -> mpsc::Sender<CommandEnvelope<C>> {
        self.sender.clone()
    }
}
```

### 5.2 Mailbox Lifecycle

```text
┌─────────────┐
│  Created    │  When actor is spawned (ACTIVE or RECOVERING)
│  mpsc open  │
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  Active     │  Commands flow: try_send → recv → execute
│  receiving  │
└──────┬──────┘
       │
       ├── passivation trigger
       │
       ▼
┌─────────────┐
│  Frozen     │  PASSIVATING entered. Sender handle removed from registry.
│  (no new)   │  try_send from EntityRef detects stale handle → EntityPassivating
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  Draining   │  Existing commands processed in FIFO order.
│  (existing) │  mpsc receiver continues until channel empty.
└──────┬──────┘
       │
       ▼
┌─────────────┐
│  Closed     │  mpsc sender/receiver dropped. Entity is PASSIVATED.
│             │  Next command triggers reactivation → new mailbox.
└─────────────┘
```

### 5.3 Backpressure

- `try_send` returns `TrySendError::Full` when mailbox capacity reached
- Caller receives `EntityError::MailboxFull(capacity)` — must retry with backoff
- No unbounded buffering at any layer
- Capacity configurable via `EntityRuntimeBuilder::mailbox_capacity()`

---

## 6. Event Persistence Flow

### 6.1 Commit Pipeline

```text
┌──────────────┐
│  Handler     │  Produces Vec<E>
│  (pure fn)   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Version     │  Check expected_version against EventStore
│  Check       │  VersionConflict → error (no persist)
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Persist     │  EventStore::persist(entity_id, expected_version, events)
│  Atomic      │  Atomic commit. seq = current_version + 1
│  Commit      │  On success: version advanced.
└──────┬───────┘
       │ success
       ▼
┌──────────────┐
│  Apply       │  apply_event(state, event) → new_state
│  In-Memory   │  Sequential per event in list order.
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Snapshot    │  If snapshot_strategy.should_snapshot(new_version):
│  (optional)  │    SnapshotStore::store(entity_id, new_version, state)
│              │  Failure is logged, NOT propagated.
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Publish     │  EventPublisher::publish(events)
│  (async)     │  Best-effort. Failure logged, not propagated.
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Response    │  CommandResult sent to caller via oneshot
│  to Caller   │
└──────────────┘
```

### 6.2 Persistence Facade Detail

```rust
impl PersistenceFacade {
    async fn persist_events<E: Serialize + DomainEvent>(
        &self,
        entity_id: &EntityTriple,
        expected_version: Option<u64>,
        events: &[E],
        metadata: EventMetadata,
    ) -> Result<u64, PersistenceError> {
        let current_version = self.event_store
            .current_version(&entity_id.tenant_id, &entity_id.entity_type, &entity_id.entity_id)
            .await?;

        // Version check
        if let Some(expected) = expected_version {
            if current_version != expected {
                return Err(PersistenceError::Conflict);
            }
        }

        // Serialize and persist
        let stored = events.iter().map(|e| {
            StoredEvent {
                seq: 0, // assigned by store
                event: e.clone(),
                metadata: metadata.clone(),
                timestamp: Utc::now(), // set by store
                entity_type: entity_id.entity_type.to_string(),
                entity_id: entity_id.entity_id.0.clone(),
                tenant_id: entity_id.tenant_id.0.clone(),
            }
        }).collect::<Vec<_>>();

        let new_version = self.event_store
            .persist(&entity_id.tenant_id, &entity_id.entity_type, &entity_id.entity_id, &stored)
            .await?;

        Ok(new_version)
    }
}
```

---

## 7. System Architecture Diagram

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                        Application Layer                                │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │  PersistentEntity<C, E, S>  (user implements)                    │   │
│  │  - handle_command(state, command, ctx) → Result<Vec<E>, Error>   │   │
│  │  - apply_event(state, event) → new_state                         │   │
│  └──────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────┬───────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                      Entity Runtime (crate)                             │
│                                                                         │
│  ┌──────────┐   ┌────────────┐   ┌────────────┐   ┌────────────────┐  │
│  │ EntityRef │──►│  Registry  │──►│ Activation │──►│  Actor (Task)  │  │
│  │ (sender)  │   │ (routing)  │   │ (single-   │   │  Tokio spawn   │  │
│  │           │   │            │   │  flight)   │   │                │  │
│  └──────────┘   └────────────┘   └────────────┘   └────────┬───────┘  │
│                                                            │           │
│  ┌──────────┐                    ┌────────────┐            │           │
│  │ Scheduler│                    │ Supervisor │            │           │
│  │(budget + │                    │(failure +  │            │           │
│  │ FIFO)    │                    │ recovery)  │            │           │
│  └──────────┘                    └────────────┘            │           │
│                                                            ▼           │
│  ┌───────────────────────────────────────────────────────────────┐     │
│  │                  Actor Execution Loop                          │     │
│  │  ┌─────────┐  ┌──────────┐  ┌─────────┐  ┌───────────────┐   │     │
│  │  │ Mailbox │─►│ Handler  │─►│ Persist │─►│ Apply + Snap  │   │     │
│  │  │ (mpsc)  │  │ (pure)   │  │ (store) │  │ + Publish     │   │     │
│  │  └─────────┘  └──────────┘  └─────────┘  └───────────────┘   │     │
│  └───────────────────────────────────────────────────────────────┘     │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │  PersistenceFacade                                               │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐   │  │
│  │  │ EventStore   │  │ SnapshotStore│  │ EventPublisher      │   │  │
│  │  │ (append-only)│  │ (cached      │  │ (async, best-effort) │   │  │
│  │  │              │  │  state)      │  │                      │   │  │
│  │  └──────────────┘  └──────────────┘  └──────────────────────┘   │  │
│  └──────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                      Persistence Layer (ego-domain traits)              │
│                                                                         │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────────┐  │
│  │ PostgreSQLES     │  │ PostgreSQLSS      │  │ InMemoryES (test)   │  │
│  │ (production)     │  │ (production)      │  │ InMemorySS (test)   │  │
│  └──────────────────┘  └──────────────────┘  └──────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 8. Configuration & Defaults

```rust
struct RuntimeConfig {
    /// Bounded mailbox capacity per entity (default: 1000)
    mailbox_capacity: usize,
    /// Global concurrency limit for ACTIVE tasks (default: 10000)
    concurrency_budget: usize,
    /// Inactivity timeout before passivation (default: 5 min)
    passivation_timeout: Duration,
    /// Whether single-tenant mode (tenant_id = "")
    single_tenant_mode: bool,
}
```

---

## 9. Key Design Invariants

| Invariant | Enforced By |
|-----------|-------------|
| Single actor per entity triple at any time | Registry `active` DashMap + single-flight activation Mutex |
| Commands processed FIFO per entity | mpsc channel ordered delivery |
| No concurrent state mutation per entity | Single actor task owns state, sequential mailbox loop |
| Deterministic replay | Pure handler/applier functions, snapshot + event replay |
| Event commit is atomic | EventStore::persist is atomic. Snapshot/publish are post-commit |
| Committed events never invalidated | Append-only EventStore. No rollback path |
| Passivation is irreversible | LifecycleStateMachine forbids PASSIVATING → ACTIVE |
| CAS forbidden per constitution §5 | Activation uses `tokio::sync::Mutex`, not AtomicUsize CAS loops |
| Implementation types not leaked | Public API exposes only domain types in trait signatures |
