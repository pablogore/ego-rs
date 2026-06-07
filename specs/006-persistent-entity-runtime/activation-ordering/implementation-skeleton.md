# Rust Runtime Implementation Skeleton (v2) — CORE-006 + Spec-007

**Date**: 2026-06-07  
**Status**: Design  
**Alignment**: CORE-006 spec (frozen) + Spec-007 Activation Ordering Model

---

## 1. Module Structure

```
ego-persistent-entity/
├── Cargo.toml
└── src/
    ├── lib.rs                    # Re-exports, public API surface
    ├── runtime.rs                # EntityRuntime<E> — top-level runtime handle
    ├── builder.rs                # EntityRuntimeBuilder<E>
    ├── entity_ref.rs             # EntityRef<C,E,S> — typed command sender + activation trigger
    ├── actor.rs                  # EntityActor<C,E,S> — Tokio task loop (recover → process → passivate)
    ├── registry.rs               # EntityRegistry — active/passivated/pending_activation maps
    ├── activation.rs             # SharedActivation — per-entity Mutex guard + watch channel
    ├── mailbox.rs                # Mailbox<C>, CommandEnvelope<C>, CommandErasedResult
    ├── persistent_entity.rs      # PersistentEntity trait (C, E, S generic)
    ├── publisher.rs              # EventPublisher<E> trait
    ├── persistence.rs            # PersistenceFacade<E> — Mutex-wrapped EventStore + Snapshot
    ├── recovery.rs               # StateRecovery trait
    ├── lifecycle.rs              # LifecycleStateMachine — 5 states, validated transitions
    ├── snapshot.rs               # SnapshotStrategy trait, SnapshotEveryN, NoSnapshot
    ├── command_context.rs        # CommandContext (correlation/causation/request-id + metadata)
    ├── error.rs                  # EntityError enum
    ├── scheduler.rs              # Scheduler (Semaphore-based concurrency gating)
    ├── supervisor.rs             # Supervisor — failure handling hooks
    └── testing.rs                # InMemoryEventStore, InMemorySnapshotStore, NoopPublisher
```

**Invariant**: Each source file maps to exactly one conceptual unit. No module exceeds 250 lines.

---

## 2. Core Structs

### 2.1 `EntityRegistry`

```rust
pub struct EntityRegistry {
    active: Arc<Mutex<HashMap<EntityTriple, ActorHandle>>>,
    passivated: Arc<Mutex<HashMap<EntityTriple, PassivationEntry>>>,
    pending_activations: Arc<Mutex<HashMap<EntityTriple, Arc<SharedActivation>>>>,
}
```

- **`active`**: Entities currently running an actor task. Insert after spawn, remove on passivation/failure.
- **`passivated`**: Entities known to have passivated, with last-known version. Used to decide activation vs. fresh start.
- **`pending_activations`**: Per-entity single-flight guards. Inserted by `get_or_create_activation()`, removed by the spawning caller after the actor is registered.

### 2.2 `ActorHandle`

```rust
pub struct ActorHandle {
    pub sender: Box<dyn Any + Send + Sync>,   // type-erased mpsc::Sender<CommandEnvelope<C>>
    pub join: tokio::task::JoinHandle<()>,     // actor task handle (fire-and-forget)
}
```

The `sender` field is the upcast of `mpsc::Sender<CommandEnvelope<C>>` stored as `Box<dyn Any>`. Downcast at retrieval time via `downcast_sender::<C>()`. This avoids making `EntityRegistry` or `ActorHandle` generic over `C`.

### 2.3 `Mailbox<C>`

```rust
pub struct Mailbox<C> {
    sender: mpsc::Sender<CommandEnvelope<C>>,
    capacity: usize,
}

impl<C> Mailbox<C> {
    pub fn new(capacity: usize) -> (Self, mpsc::Receiver<CommandEnvelope<C>>);
    pub fn sender(&self) -> mpsc::Sender<CommandEnvelope<C>>;
    pub fn try_send(&self, envelope: CommandEnvelope<C>)
        -> Result<(), TrySendError<C>>;
    pub fn is_closed(&self) -> bool;
    pub fn capacity(&self) -> usize;
}
```

### 2.4 `SharedActivation` (per-entity Mutex guard)

```rust
pub struct SharedActivation {
    pub lock: Mutex<()>,                           // per-entity serialization
    pub result_tx: watch::Sender<Option<EntityError>>,  // recovery outcome notification
    pub result_rx: watch::Receiver<Option<EntityError>>, // wait for recovery to finish
}
```

Constructed once per activation attempt. The `lock` ensures exactly one caller proceeds past the spawn decision. The `watch` channel lets concurrent waiters observe the recovery outcome (success or failure) without polling.

### 2.5 `EntityRuntime<E>`

```rust
pub struct EntityRuntime<E: DomainEvent> {
    pub registry: Arc<EntityRegistry>,
    pub scheduler: Arc<Scheduler>,
    pub persistence: Arc<PersistenceFacade<E>>,
    pub publisher: Arc<dyn EventPublisher<E>>,
    pub config: RuntimeConfig,
    pub snapshot_strategy: Arc<dyn SnapshotStrategy>,
}
```

---

## 3. Tokio Spawn Flow (fully defined)

### 3.1 Activation Entry Point: `EntityRef::send()`

```
   send(command, ctx, expected_version)
        │
        ▼
   registry.get_active_sender::<C>(&entity)
        │
        ├── Some(tx) ──────────────────────────────► tx.send(envelope).await
        │                                               │
        │                                           wait on response_rx
        │                                               │
        │                                           return downcast_result()
        │
        └── None ──► activate_and_send(envelope, response_rx)
```

### 3.2 Spawn Decision: `EntityRef::activate_and_send()`

```
   activation = registry.get_or_create_activation(entity.clone())
        │
   guard = activation.lock.lock().await       // ← Mutex ACQUIRED
        │
   sender = registry.get_active_sender::<C>(&entity).await
        │
        ├── Some(tx) ──► drop(guard)          // ← Mutex RELEASED
        │                   │
        │                   tx.send(envelope).await
        │                   │
        │                   return downcast_result()
        │
        └── None ──► (mailbox_tx, mailbox_rx) = mpsc::channel(capacity)
                        │
                    tokio::spawn(async move {      // ← ACTOR SPAWNED
                        let actor = EntityActor { mailbox: mailbox_rx, ... };
                        actor.run().await;
                    })
                        │
                    registry.insert_active(entity, ActorHandle::new(mailbox_tx, join))
                    registry.remove_passivated(&entity)
                    drop(guard)                     // ← Mutex RELEASED
                    registry.remove_activation(&entity)
                        │
                    mailbox_tx.send(envelope).await  // ← FIRST COMMAND SENT
                        │
                    wait on response_rx
                        │
                    return downcast_result()
```

### 3.3 Critical Ordering Invariant

| Step | Action | Mutex State | Mailbox State | Registry State |
|------|--------|-------------|---------------|----------------|
| 1 | `get_or_create_activation()` | - | - | pending_activations has entry |
| 2 | `lock.lock().await` | **HELD** | - | - |
| 3 | Re-check `get_active_sender()` | **HELD** | - | active queried |
| 4 | `mpsc::channel()` created | **HELD** | **EXISTS** (empty) | - |
| 5 | `tokio::spawn(actor)` | **HELD** | mailbox_rx passed to actor | - |
| 6 | `insert_active()` | **HELD** | mailbox_tx stored | **active has entry** |
| 7 | `drop(guard)` | **RELEASED** | - | - |
| 8 | `remove_activation()` | - | - | pending_activations entry removed |
| 9 | First command sent | - | **1 message queued** | - |
| 10 | Actor begins `recover_state()` | - | More commands may arrive | active remains |
| 11 | Recovery completes | - | Commands queued during recovery | active remains |
| 12 | `process_commands()` starts | - | Queue drained FIFO | active remains |

---

## 4. Mutex-Based Activation Guard — Ownership Model

### 4.1 Guard Lifetime

```
  ┌─────────────────────────────────────────────────────────────┐
  │ EntityRef::activate_and_send() scope                        │
  │                                                             │
  │  let guard = activation.lock.lock().await;     ← ACQUIRE   │
  │  ... spawn decision ...                                    │
  │  ... mailbox creation ...                                  │
  │  ... tokio::spawn ...                                      │
  │  ... insert_active ...                                     │
  │  drop(guard);                                 ← RELEASE    │
  │  ... remove_activation ...                                 │
  │  ... send first command ...                                │
  └─────────────────────────────────────────────────────────────┘
```

### 4.2 Concurrent Behavior

```
  ┌──────────────┐                    ┌──────────────┐
  │ Caller A      │                    │ Caller B      │
  │ (first)       │                    │ (concurrent)  │
  ├──────────────┤                    ├──────────────┤
  │ get_or_create │                    │ get_or_create  │
  │ activation()  │                    │ activation()   │
  │      │       │                    │      │        │
  │ lock.lock()  │                    │ lock.lock()    │
  │ (acquires)   │                    │ (blocks)       │
  │      │       │                    │      │        │
  │ spawn actor  │                    │      │        │
  │ insert_active│                    │      │        │
  │ drop(guard)  │                    │      │        │
  │      │       │                    │      │        │
  │              │                    │ (wakes up)     │
  │              │                    │ re-check active│
  │              │                    │ found sender   │
  │              │                    │ send to mailbox│
  └──────────────┘                    └──────────────┘
```

### 4.3 Watch Channel (recovery outcome)

After `drop(guard)`, waiting callers (who arrived after the mutex was released) can optionally use `activation.result_rx` to wait for recovery outcome:

```rust
// Caller B (arrives after mutex released, finds active sender)
let sender = registry.get_active_sender::<C>(&entity).await.unwrap();
sender.send(envelope).await?;

// Optionally wait for recovery to complete before reading response
if let Err(e) = activation.result_rx.changed().await {
    // recovery channel closed — actor panicked
}
// Now read response_rx
```

---

## 5. Mailbox Attach Timing

### 5.1 Precise Timeline

```
time ─────────────────────────────────────────────────────────────►

Event:              Activation begins      Actor spawned       Recovery starts    Commands processed
                         │                      │                    │                    │
                         ▼                      ▼                    ▼                    ▼
                     ┌────────┐            ┌────────┐          ┌────────┐           ┌────────┐
                     │ Mutex  │            │ mpsc   │          │ Actor  │           │ Mailbox│
                     │ Locked │            │ Channel│          │ Run()  │           │ Drain  │
                     └────────┘            └────────┘          └────────┘           └────────┘
                         │                      │                    │                    ▲
                         │                      │                    │                    │
                    ┌────┴────┐            ┌─────┴─────┐       ┌────┴────┐               │
                    │ mpsc    │            │ tokio     │       │ recovery│               │
                    │ channel │            │ .spawn()  │       │ state() │               │
                    │ created │            └───────────┘       └─────────┘               │
                    │         │                                      │                   │
                    │ can     │                                      │                   │
                    │ accept  │                                      ▼                   │
                    │ cmds    │                                 ┌─────────┐              │
                    └─────────┘                                 │ .await  │              │
                         │                                      │ process │              │
                         │                                      │ commands│              │
                         ▼                                      ▼ ────────┘              │
                    ┌────────────┐                         ┌──────────────┐              │
                    │ First cmd  │                         │ Queue drained│              │
                    │ sent via tx│                         │ FIFO         │              │
                    └────────────┘                         └──────────────┘              │
                         │                                                               │
                         └───────────────────────────────────────────────────────────────┘
                                              Commands queue in channel buffer
                                              during recovery, untouched by actor
```

### 5.2 Key Invariant

**The `mpsc::Sender` is registered in the active registry BEFORE the mutex is released**, and the first command is sent AFTER the mutex is released but BEFORE the actor enters `recover_state()`. Commands that arrive during recovery are buffered in the channel. The actor's `process_commands()` loop only starts draining after `recover_state()` returns.

---

## 6. Recovery Executor Loop (pseudocode)

```rust
// === EntityActor::run() ===
pub async fn run(&mut self) {
    // ───── Phase 1: Recover ─────
    self.recover_state().await;

    if self.lifecycle.state() == LifecycleState::Failed {
        // Recovery failed — clean up and exit
        self.registry.remove_active(&self.entity_id).await;
        return;
    }

    // ───── Phase 2: Process Commands ─────
    self.process_commands().await;

    // ───── Phase 3: Passivate ─────
    self.passivate().await;
}

// ───── Phase 1 Detail ─────
async fn recover_state(&mut self) {
    let result = self.persistence.load_for_recovery(
        &self.entity_id.aggregate_id(),
        Some(&self.entity_id.tenant_id),
    );

    let (snap_data, stored_events) = match result {
        Ok(data) => data,
        Err(e) => {
            let _ = self.lifecycle.transition_to(LifecycleState::Failed);
            log::error!("Recovery failed: {}", e);
            return;
        }
    };

    // 1. Load snapshot (if any)
    let (mut state, mut version) = match snap_data {
        Some(snap) => {
            let s = serde_json::from_slice(&snap.data)
                .unwrap_or_else(|_| self.entity_handler.initial_state());
            (s, snap.version)
        }
        None => (self.entity_handler.initial_state(), 0),
    };

    // 2. Replay events in order (deterministic)
    for stored in &stored_events {
        let new_state = self.entity_handler.apply_event(
            &state,
            stored.event.clone(),
        ).await;
        state = new_state;
        version += 1;
    }

    // 3. Transition to Active
    self.state = Some(state);
    self.version = version;
    let _ = self.lifecycle.transition_to(LifecycleState::Active);
}

// ───── Phase 2 Detail ─────
async fn process_commands(&mut self) {
    let timeout = self.config.passivation_timeout;

    loop {
        tokio::select! {
            biased;  // prefer commands over timeout

            Some(envelope) = self.mailbox.recv() => {
                self.execute_command(envelope).await;

                if self.lifecycle.should_passivate(timeout) {
                    break;
                }
            }

            _ = tokio::time::sleep(timeout) => {
                if self.lifecycle.should_passivate(timeout) {
                    break;
                }
            }
        }
    }
}

// ───── Phase 3 Detail ─────
async fn passivate(&mut self) {
    let _ = self.lifecycle.transition_to(LifecycleState::Passivating);

    // Drain remaining commands
    while let Some(envelope) = self.mailbox.recv().await {
        self.execute_command(envelope).await;
    }

    // Final snapshot
    if let Some(state) = &self.state {
        let _ = self.persistence.store_snapshot(
            &self.entity_id.aggregate_id(),
            Some(&self.entity_id.tenant_id),
            self.version,
            &serde_json::to_value(state).unwrap_or_default(),
        );
    }

    // Unregister
    self.registry
        .mark_passivated(self.entity_id.clone(), self.version)
        .await;

    let _ = self.lifecycle.transition_to(LifecycleState::Passivated);
}
```

### 6.1 Recovery State Machine Transitions

```
        ┌────────────────┐
        │  NEW Actor     │  (constructed with state=None, version=0, lifecycle=RECOVERING)
        └───────┬────────┘
                │
                ▼
        ┌────────────────┐
        │ recover_state  │  snapshot load + event replay
        │ ()             │
        └───────┬────────┘
                │
        ┌───────┴───────┐
        │               │
        ▼               ▼
  ┌──────────┐   ┌──────────┐
  │ ACTIVE   │   │ FAILED   │  (recovery error — snapshot corrupt,
  │          │   │          │   deserialization fails, event store error)
  └────┬─────┘   └──────────┘
       │               │
       │               └────────► registry.remove_active()
       │
       │  timeout / passivation
       ▼
  ┌──────────┐
  │PASSIVATING│  drain remaining commands
  └────┬─────┘
       │
       ▼
  ┌──────────┐
  │PASSIVATED│  store final snapshot, mark_passivated()
  └──────────┘
```

---

## 7. Full Execution Timeline Diagram

```
┌─────────────────────────────────────────────────────────────────────────────────────┐
│ ACTIVATION (caller context)                                                         │
│                                                                                     │
│  1. registry.get_or_create_activation(entity)                                       │
│       → creates SharedActivation { lock: Mutex, result_tx, result_rx }              │
│                                                                                     │
│  2. activation.lock.lock().await  ← MUTEX ACQUIRED                                  │
│                                                                                     │
│  3. Re-check: registry.get_active_sender(entity)                                    │
│       → found? Yes → drop(guard), redirect to existing mailbox                      │
│       → NOT found → proceed to spawn                                                │
│                                                                                     │
│  4. let (mailbox_tx, mailbox_rx) = mpsc::channel(capacity)                          │
│                                                                                     │
│  5. tokio::spawn(async move { EntityActor { mailbox: mailbox_rx, ... }.run().await })│
│                                                                                     │
│  6. registry.insert_active(entity, ActorHandle::new(mailbox_tx, join))              │
│       → active map now has entry → subsequent lookups find it                      │
│                                                                                     │
│  7. drop(guard)  ← MUTEX RELEASED                                                   │
│                                                                                     │
│  8. registry.remove_activation(&entity)  → pending_activations cleaned up           │
│                                                                                     │
│  9. mailbox_tx.send(envelope).await  → FIRST COMMAND IN MAILBOX                     │
│       (actor task may still be in recover_state())                                  │
│                                                                                     │
│ 10. Wait on response_rx for first command                                           │
└─────────────────────────────────────────────────────────────────────────────────────┘
                                                                                              
┌─────────────────────────────────────────────────────────────────────────────────────┐
│ ACTOR TASK (spawned context)                                                        │
│                                                                                     │
│  spawn → EntityActor::run() is called                                               │
│                                                                                     │
│  [RECOVERING]                                                                       │
│     │                                                                              │
│     ├─ recover_state():                                                            │
│     │   ├─ persistence.load_for_recovery()  → (snapshot, events)                   │
│     │   ├─ serde_json::from_slice(snapshot) → state S                              │
│     │   ├─ for each event: apply_event(&state, event).await → new_state            │
│     │   └─ self.state = Some(state); self.version = count                          │
│     │                                                                              │
│     ├─ success? → transition(ACTIVE)                                               │
│     │   └─ (commands already queued in mailbox during replay)                      │
│     │                                                                              │
│     └─ failure? → transition(FAILED); registry.remove_active(); return             │
│                                                                                     │
│  [ACTIVE]                                                                           │
│     │                                                                              │
│     ├─ process_commands():                                                         │
│     │   ├─ loop { select! {                                                        │
│     │   │     cmd = mailbox.recv() → execute_command(cmd)                          │
│     │   │     timeout              → should_passivate?                             │
│     │   │   }}                                                                     │
│     │   └─ execute_command:                                                        │
│     │       ├─ handler.handle_command(&state, cmd, ctx).await → events             │
│     │       ├─ persistence.persist_events(...) → new_version                       │
│     │       ├─ reload state + replay persisted events                              │
│     │       ├─ snapshot_strategy.should_snapshot → store snapshot                  │
│     │       └─ publisher.publish(events).await                                     │
│     │                                                                              │
│     └─ passivation timeout reached                                                 │
│                                                                                     │
│  [PASSIVATING → PASSIVATED]                                                        │
│     ├─ drain remaining commands from mailbox                                       │
│     ├─ store final snapshot                                                        │
│     ├─ registry.mark_passivated(entity, version)                                   │
│     └─ actor task ends                                                             │
└─────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 8. Concurrency Safety Argument

| Risk | Mechanism | Mitigation |
|------|-----------|------------|
| Double spawn | Activation mutex | Only one caller holds lock; others block and re-check |
| Message loss during activation | mpsc channel created before spawn | Sender registered before mutex released; first command sent after |
| Stale mailbox after actor death | `mpsc::Sender::is_closed()` | `send()` returns error → caller treats as entity no longer active → triggers fresh activation |
| Partial recovery visible | Recovery completes before `process_commands()` | Commands queued in channel buffer; no `.recv()` until recovery done |
| Activation guard leak | `drop(guard)` in same scope as acquire | Rust's drop guarantees release even on panic |
| pending_activations leak | `remove_activation()` after `drop(guard)` | Both always execute in the spawning path |
| Concurrent passivation + activation | Passivation calls `remove_active()` then `mark_passivated()` | Activation re-checks active after acquiring mutex; atomic window closed |
| Version conflict on persist | Optimistic concurrency check | `EventStore::append(expected_version)` fails if concurrent write happened |
