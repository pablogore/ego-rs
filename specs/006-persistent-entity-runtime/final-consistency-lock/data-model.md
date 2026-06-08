# Data Model: CORE-006 Persistent Entity Runtime

**Feature**: `006-persistent-entity-runtime/final-consistency-lock`
**Created**: 2026-06-07

## Core Types

### EntityId
```rust
/// Entity identity: (tenant_id, entity_type, entity_id)
pub struct EntityId {
    pub tenant_id: TenantId,
    pub entity_type: &'static str,
    pub entity_id: String,
}

pub type TenantId = String;
```

**Constraints**:
- `entity_type` is a static string identifying the PersistentEntity implementation
- `entity_id` is a caller-provided unique identifier within (tenant, type) scope
- The triple `(tenant_id, entity_type, entity_id)` is globally unique

### EntityTriple
```rust
/// Hashable key for entity registry lookups
#[derive(Hash, Eq, PartialEq, Clone)]
pub struct EntityTriple {
    pub tenant: TenantId,
    pub entity_type: &'static str,
    pub entity_id: String,
}
```

### ExecutionKey
```rust
/// Deterministic execution identity: hash(entity_id, command, state_version)
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct ExecutionKey([u8; 32]); // blake3 hash output

impl ExecutionKey {
    pub fn compute(
        entity_id: &EntityId,
        command_payload: &impl Serialize,
        state_version: u64,
    ) -> Self { ... }
}
```

**Constraints**:
- Deterministic: same `(entity_id, command, state_version)` → same `ExecutionKey`
- Scoped to Actor lifecycle window (reset on passivation)
- Not exposed in public API (EntityRef, CommandResult)
- Used by Actor for deduplication, not by Scheduler or Backend

### CommandEnvelope
```rust
pub struct CommandEnvelope<C> {
    pub entity_id: EntityId,
    pub command: C,
    pub context: CommandContext,
    pub expected_version: u64,
}
```

### CommandContext
```rust
pub struct CommandContext {
    pub tenant_id: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub approval_id: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}
```

## Trait Definitions

### PersistentEntity
```rust
/// Developer-defined entity behavior — pure, deterministic, stateless
#[async_trait]
pub trait PersistentEntity: Send + Sync + 'static {
    type Command: Send + 'static;
    type Event: DomainEvent + Send + 'static;
    type State: Clone + Send + Sync + 'static;
    type Error: std::error::Error + Send + 'static;

    /// Pure function: (state, command, context) -> (events | error)
    async fn handle_command(
        &self,
        state: &Self::State,
        command: Self::Command,
        context: &CommandContext,
    ) -> Result<Vec<Self::Event>, Self::Error>;

    /// Pure function: (state, event) -> state
    fn apply_event(&self, state: &Self::State, event: &Self::Event) -> Self::State;

    /// Initial state before any events
    fn initial_state(&self) -> Self::State;
}
```

**Constraints** (Handler Safety Contract):
- No I/O, random, clock, threads, or global state in handlers/appliers
- Deterministic: same (state, command, ctx) → same (events | error)
- Stateless between invocations (state is passed in/out)
- Identity-agnostic (no ExecutionKey awareness)

### DomainEvent
```rust
pub trait DomainEvent: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {
    fn event_type(&self) -> &'static str;
    fn aggregate_id(&self) -> Option<String>;
    fn tenant_id(&self) -> Option<String>;
}
```

### ExecutionBackend
```rust
/// Synchronous execution contract — pure computation, no async needed
pub trait ExecutionBackend: Send + Sync + Debug {
    fn execute<C, E, S>(
        &self,
        entity: &dyn PersistentEntity<Command = C, Event = E, State = S, Error = EntityError>,
        state: &S,
        command: C,
        context: &CommandContext,
    ) -> Result<(Vec<E>, S), EntityError>
    where
        C: Send + 'static,
        E: DomainEvent + Send + 'static,
        S: Clone + Send + Sync + 'static;
}
```

**Constraints**:
- Synchronous — no `.await`, no async machinery in the trait
- Backend-agnostic: Tokio, Yoke, WASM, custom all implement this trait
- No Actor state access, no Scheduler awareness, no EventStore access

### SchedulingPolicy
```rust
/// Policy engine: defines activation order, fairness, budget
pub trait SchedulingPolicy: Send + Sync + 'static {
    /// Select the next entity to activate from the pending set
    fn select_next(&self, pending: &HashSet<EntityTriple>, budget_available: usize) 
        -> Option<EntityTriple>;

    /// Check if a newly arrived entity should preempt current scheduling
    fn should_preempt(&self, entity: &EntityTriple, 
                       current: &EntityTriple) -> bool;

    /// Get the configured concurrency budget size
    fn budget_size(&self) -> usize;

    /// Get the fairness window (in scheduling decisions processed)
    fn fairness_window(&self) -> u64;
}
```

**Constraints**:
- Stateless policy evaluation (state stored externally in Scheduler)
- Deterministic: same inputs → same `select_next` output
- Does not depend on ExecutionBackend or wall-clock

## Lifecycle States

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorLifecycleState {
    /// Loading snapshot + replaying events. Exempt from budget.
    Recovering,
    /// Processing mailbox commands. Consumes budget slot.
    Active,
    /// Draining mailbox before shutdown. Exempt from budget after drain starts.
    Passivating,
    /// No in-memory state, no task. Exists in passivation registry only.
    Passivated,
    /// Irrecoverable error. On-demand recovery.
    Failed,
}
```

**State transitions**:
- `Recovering → Active` (automatic on recovery completion)
- `Active → Passivating` (passivation policy trigger)
- `Passivating → Passivated` (mailbox drained, state serialized)
- `Passivated → Recovering` (command arrival triggers reactivation)
- `Active → Failed` (irrecoverable error)
- `Failed → Recovering` (on-demand admin trigger)

## Entity State & Versioning

```rust
/// Per-entity runtime state tracked by EntityActor
struct EntityRuntimeState<S> {
    state: S,                          // current entity state
    version: u64,                       // committed event count (0 = no events)
    lifecycle: ActorLifecycleState,
    seen_execution_keys: HashSet<ExecutionKey>,  // dedup per lifecycle window
}
```

**Version rules**:
- Starts at 0 (no events)
- Incremented atomically with event commit
- Zero-event commands do NOT advance version
- Version gaps forbidden: (seq N) → (seq N+1) only
- Snapshot at version V = "state after applying events 1..V"

## Scheduler Internal State

```rust
struct SchedulerState {
    /// Entities waiting for activation (ordered by arrival)
    activation_queue: VecDeque<EntityTriple>,
    /// Number of scheduling decisions since each entity was last activated
    fairness_tracker: HashMap<EntityTriple, u64>,
    /// Number of currently active (budget-consuming) entities
    active_count: usize,
    /// Policy
    policy: Box<dyn SchedulingPolicy>,
}
```

## EntityActor Internal State

```rust
pub struct EntityActor<C, E, S> {
    /// Entity identity
    entity_id: EntityId,
    /// Lifecycle state machine
    lifecycle: LifecycleStateMachine,
    /// Current state + version
    state: EntityRuntimeState<S>,
    /// Mailbox receiver (owned by this task)
    mailbox: BoundedMailboxReceiver<CommandEnvelope<C>>,
    /// Execution backend
    backend: Arc<dyn ExecutionBackend>,
    /// Persistence facade
    persistence: Arc<dyn PersistenceFacade<E>>,
    /// Entity trait implementation
    handler: Arc<dyn PersistentEntity<...>>,
    /// Slot permit (drops on task completion → frees budget slot)
    _budget_guard: Option<OwnedSemaphorePermit>,
}
```

## Entity Types

- **EntityId**: Three-part identity: `(tenant_id, entity_type, entity_id)`. Globally unique per entity.
- **EntityTriple**: Hashable, equality-comparable key for entity registry lookups.
- **ExecutionKey**: Deterministic hash of `(entity_id, command_payload, state_version)`. Identifies a specific execution occurrence. Actor-scoped deduplication window.
- **CommandEnvelope<C>**: Wraps a command with its EntityId, CommandContext, and expected version for the mailbox.
- **CommandContext**: Runtime metadata per command: tenant, correlation, causation, approval, extensible metadata map.
- **PersistentEntity<C,E,S>**: Developer trait defining `handle_command` (pure handler), `apply_event` (pure applier), `initial_state`.
- **ExecutionBackend**: Synchronous trait for executing ExecutionUnit computation. Process-wide singleton. Backend-agnostic.
- **SchedulingPolicy**: Stateless trait defining activation order, fairness rules, budget size. Deterministic.
- **EntityActor<C,E,S>**: Per-entity async task. Owns state, mailbox receiver, lifecycle FSM. Processes commands sequentially.
- **LifecycleStateMachine**: Tracks and enforces valid state transitions: Recovering → Active → Passivating → Passivated, Failed from any.
- **BoundedMailbox**: `tokio::sync::mpsc` channel with `try_send` for bounded backpressure.
- **EntityRegistry**: Concurrent map of active entities (sender handles), passivated entities (version tracking), and pending activations.
- **SharedActivation**: Per-entity `Arc<Mutex<Option<()>>>` guard for single-flight spawn, budget enforcement point.
- **PersistenceFacade<E>**: Abstracts EventStore + SnapshotStore for the Actor. Delegates to domain `EventStore` trait.
