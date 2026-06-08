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

Deterministic projection state reconstructed from the observed event stream.

```rust
#[derive(Debug, Clone)]
pub struct SchedulerState {
    /// Lifetime count of processed events.
    pub total_events_consumed: u64,

    /// Sequence ID of the most recent event consumed.
    pub last_sequence_id: Option<u64>,

    /// Cumulative count of detected sequence gaps.
    pub detected_gaps: u64,

    /// Bounded diagnostic buffer of recent events (capacity: 1024).
    /// Best-effort, used for debugging and verification only.
    /// NOT a source of truth for full state reconstruction.
    pub replay_buffer: VecDeque<(u64, SchedulerEventEnvelope)>,

    /// The most recent activation suggestion produced.
    pub last_suggestion: Option<EntityTriple>,

    /// Optional cryptographic snapshot hash for state integrity verification.
    pub state_hash: Option<[u8; 32]>,
}
```

**Determinism invariant**: `SchedulerState` is a pure function of the observed event stream. Given identical observed streams, two Scheduler instances MUST produce identical state.

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

Controls behavior when the bounded event bus reaches capacity.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropPolicy {
    /// Default — sender blocks until buffer space is available.
    /// High-water mark (90%) triggers blocking before overflow.
    Block,

    /// Opt-in — incoming event is silently dropped.
    /// Actor execution is never blocked.
    DropNewest,

    /// Opt-in — oldest buffered event is evicted; newest is accepted.
    DropOldest,
}
```

---

## GapInfo

Structured gap record emitted for observability.

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
