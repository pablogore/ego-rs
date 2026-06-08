# CORE-007 SchedulerState

## State Machine

```
[Initial] ──► event consumed ──► [Projecting] ──► suggest_activation ──► [Ready]
     ▲                                                                │
     └───────────────────── reset ────────────────────────────────────┘
```

## State Fields

### `total_events_consumed: u64`
Monotonically increasing lifetime counter. Wraps on overflow.

### `last_sequence_id: Option<u64>`
The sequence_id of the most recently consumed event. Used for gap detection.

### `detected_gaps: u64`
Cumulative count of detected sequence gaps. Incremented each time a consumed event's sequence_id is not exactly `last_sequence_id + 1`.

### `replay_buffer: VecDeque<(u64, SchedulerEvent)>`
Bounded diagnostic FIFO buffer of the last N consumed events. Default capacity: 1024. Best-effort, used for debugging and recent-history verification. NOT a source of truth for full state reconstruction.

### `last_suggestion: Option<EntityTriple>`
The most recent output of `suggest_activation`. Purely informational.

### `state_hash: Option<[u8; 32]>`
Optional SHA-256 hash of the serialized SchedulerState. Enables external integrity verification.

## Determinism Proof

Given **observed** event stream E = [e1, e2, ..., en]:

1. Initial state S0 is fixed
2. Each event ei updates state deterministically: S_i = apply(S_{i-1}, ei)
3. Sn = apply(apply(...apply(S0, e1), e2), ..., en)
4. Sn depends only on E and S0, not on timing, load, event_id values, loss configuration, or replay buffer contents
