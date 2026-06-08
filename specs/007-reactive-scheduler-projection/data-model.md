# Data Model: CORE-007 Reactive Scheduler

> Rust type definitions for the CORE-007 data model.
> See `contracts/scheduling-policy.md` for the policy trait contract.

---

## EntityTriple

Triple identifier for an actor/entity. Defined within CORE-007 scope (not yet promoted to `ego-domain`).

```rust
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct EntityTriple {
    pub tenant: String,
    pub entity_type: String,
    pub entity_id: String,
}
```

---

## SchedulerEvent

Events that the Scheduler consumes from CORE-006.

```rust
#[derive(Debug, Clone)]
pub enum SchedulerEvent {
    ExecutionCompleted {
        entity: EntityTriple,
        state_version: u64,
    },
    RecoveryCompleted {
        entity: EntityTriple,
        state_version: u64,
    },
}
```

**Note**: This is intentionally a subset of CORE-006 domain events. Only execution-relevant events are projected into SchedulerState. Domain events with no scheduling relevance are ignored.

---

## SchedulerEventEnvelope

Wrapper emitted from CORE-006 into the bounded event bus.

```rust
#[derive(Debug, Clone)]
pub struct SchedulerEventEnvelope {
    /// Deterministic SHA-256 hash of canonical payload.
    /// Identity annotation only — NOT part of determinism proof.
    pub event_id: [u8; 32],

    /// Monotonically increasing per-Actor stream.
    /// **Per-actor scoped.** This value carries ordering semantics
    /// only within its source entity's stream. Cross-entity
    /// comparison of this field is prohibited.
    pub sequence_id: u64,

    /// Classification of the event.
    pub event_type: EventType,

    /// Event-specific structured data.
    pub payload: SchedulerEvent,

    /// The Actor that emitted this event.
    pub source_actor: EntityTriple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    ExecutionCompleted,
    RecoveryCompleted,
}
```

---

## SchedulerState

Deterministic projection state reconstructed from the observed event stream. **Single-stream model**: SchedulerState tracks exactly one entity's projection at a time. When the observed stream switches entities, per-entity tracking resets. No cross-entity state is retained between projection cycles. **SchedulerState is a projection aggregation of independent per-entity streams — it is NOT a global timeline and MUST NOT be interpreted as imposing cross-entity ordering.**

**SchedulerState is a pure reducer output — a deterministic projection artifact (data structure), NOT a runtime engine.** `apply()` is a pure function: `(Event, SchedulerState) → SchedulerState`. It performs state transformation only — no I/O, no bus interaction, no policy evaluation, no entity switch detection, no reset logic beyond field updates. Entity switch detection (`current_entity != event.source_actor`) and per-entity field resets are performed by the Scheduler BEFORE calling `apply()`. Orchestration (bus drain, event loop, policy evaluation, suggestion output, per-entity reset coordination) belongs to the `Scheduler` struct, not SchedulerState.

```rust
#[derive(Debug, Clone)]
pub struct SchedulerState {
    /// Lifetime count of processed events across all entities.
    /// Aggregate diagnostic counter only — carries no ordering semantics.
    pub total_events_consumed: u64,

    /// Sequence ID of the most recent event consumed for the currently
    /// projected entity. Per-actor scoped within the current projection
    /// cycle. Resets when the observed stream switches entities.
    /// Cross-entity comparison of this field is prohibited.
    /// Never compared across entities.
    pub last_sequence_id: Option<u64>,

    /// Gap count for the currently projected entity's contiguous stream
    /// segment. Per-actor scoped within the current projection cycle.
    /// Resets when the observed stream switches entities.
    /// Gaps may arise from DropPolicy drops or from sequence discontinuities;
    /// the scheduler treats all gaps uniformly — no recovery, no reconciliation.
    pub detected_gaps: u64,

    /// Bounded diagnostic buffer of recent events (capacity: 1024).
    /// **Non-semantic, diagnostic-only.** Best-effort; ephemeral; lost on restart.
    /// MUST NOT be used as a source of truth, fallback store, reconstruction
    /// mechanism, or for any behavioral decision.
    /// ReplayBuffer differences MUST NOT affect SchedulerState equivalence.
    /// Two states with identical semantic fields are equivalent regardless of
    /// buffer contents.
    /// Used exclusively for debugging and recent-history inspection.
    /// Events from different entities coexist but carry no cross-entity ordering semantics.
    pub replay_buffer: VecDeque<(u64, SchedulerEventEnvelope)>,

    /// The most recent activation suggestion produced.
    pub last_suggestion: Option<EntityTriple>,

    /// Optional cryptographic snapshot hash for state integrity verification.
    /// Does not encode or preserve cross-entity ordering.
    pub state_hash: Option<[u8; 32]>,
}
```

**Invariants**:
- **I1**: `SchedulerState` is a pure function of the observed event stream (post-DropPolicy). Given identical streams, two instances MUST produce identical state. Internal execution order, concurrency scheduling, replay buffer, event_id, and system load are not inputs to f.
- **I2**: Sequence identifiers are scoped per-entity. Never compared across entity boundaries. No cross-entity ordering exists or is inferred. Each entity stream is fully isolated.
- **I3**: Scheduler output is purely advisory. `suggest_activation` is NOT a command. Scheduler MUST NOT influence execution directly or indirectly. Execution authority belongs exclusively to CORE-006.
- **I4**: `replay_buffer` is non-semantic diagnostic. Never used for reconstruction, determinism validation, or recovery. ReplayBuffer differences MUST NOT affect SchedulerState equivalence. Two states with identical semantic fields but different buffers are equivalent.
- **I5**: DropPolicy is fully deterministic. Drop outcomes depend only on event arrival order, buffer capacity, and policy type. Load/concurrency affect timing only — never which events are dropped.

---

## Suggestion

An advisory activation recommendation produced by the SchedulingPolicy.

```rust
#[derive(Debug, Clone)]
pub struct Suggestion {
    /// The entity the scheduler recommends activating.
    pub target: EntityTriple,

    /// Monotonic suggestion ID for deduplication.
    pub suggestion_id: u64,

    /// Timestamp (logical, not wall-clock) for ordering.
    pub logical_time: u64,
}
```

---

## DropPolicy

Controls behavior when the bounded event bus reaches capacity. **All variants are fully deterministic**: given identical event arrival order, buffer capacity, and policy config, the same events are dropped on every execution. Load affects *whether* drops occur — never *which* events are dropped.

**Bus ownership**: Single-consumer, multi-producer. The Scheduler creates the channel and owns the receiver exclusively. Senders are cloned for CORE-006 actors. Dropping the Scheduler closes the channel.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropPolicy {
    /// Default — sender blocks until buffer space is available.
    /// High-water mark (90%) triggers blocking before overflow.
    /// Fully deterministic: senders block and unblock in arrival order.
    Block,

    /// Opt-in — incoming event is silently dropped.
    /// Actor execution is never blocked.
    /// Fully deterministic: the event dropped is the one whose send reached
    /// a full buffer; this is determined by event arrival order, not by OS scheduling.
    DropNewest,

    /// Opt-in — oldest buffered event is evicted; newest is accepted.
    /// Fully deterministic: the buffer's FIFO order is maintained; the oldest
    /// event in the queue is evicted regardless of concurrency conditions.
    DropOldest,
}
```

---

## GapInfo

Structured gap record emitted for observability. **Per-actor scoped** — gap detection operates independently within each entity stream. Gap records from different entities carry no cross-entity ordering semantics.

```rust
#[derive(Debug, Clone)]
pub struct GapInfo {
    /// Start of the gap range (exclusive of last consumed).
    pub start_seq: u64,

    /// End of the gap range (inclusive of the gap boundary).
    pub end_seq: u64,

    /// Actor stream where gap was detected.
    pub source_actor: EntityTriple,
}
```

---

## BusItem

An item dequeued from the event bus.

```rust
#[derive(Debug, Clone)]
pub struct BusItem {
    pub sequence: u64,
    pub event: SchedulerEventEnvelope,
}
```

---

## Relation Diagram

```
CORE-006 Runtime
    │
    │ emit SchedulerEventEnvelope
    ▼
┌─────────────────────┐
│  Bounded Event Bus  │  capacity: 4096 (default), DropPolicy configurable
│  (tokio::sync::     │
│   mpsc channel)     │
└─────────┬───────────┘
          │ consume
          ▼
┌─────────────────────┐
│  Scheduler          │
│  ┌───────────────┐  │
│  │ SchedulerState│  │ ← pure function of observed event stream
│  │ (ephemeral)   │  │
│  └───────┬───────┘  │
│          │           │
│  ┌───────▼───────┐  │
│  │ Scheduling    │  │ ← pure function: (state, pending) → Suggestion
│  │ Policy        │  │
│  └───────┬───────┘  │
└──────────┼──────────┘
           │ Suggestion (advisory)
           ▼
    CORE-006 Runtime
    (may accept or ignore)
```
