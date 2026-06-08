# CORE-007 SchedulerState

## State Categories

SchedulerState divides into two independent categories:

- **Projection State (Semantic)**: Fields that participate in the determinism proof and drive `suggest_activation`. These are the only fields that matter for system behavior.
- **Diagnostic State (Non-Semantic)**: Fields that exist purely for observability, debugging, and inspection. These fields participate in NO system behavior, invariant, or decision.

## State Machine

```
[Initial] ──► event consumed ──► [Projecting] ──► suggest_activation ──► [Ready]
     ▲                                                                │
     └───────────────────── reset ────────────────────────────────────┘
```

## Semantic State Fields

### `total_events_consumed: u64`
Monotonically increasing lifetime counter. Wraps on overflow.

### `last_sequence_id: Option<u64>`
The sequence_id of the most recently consumed event. **Per-actor scoped.** Used for gap detection within a single entity stream. Cross-entity comparison of this field is prohibited per the no global ordering invariant.

### `detected_gaps: u64`
Cumulative count of detected sequence gaps. Incremented each time a consumed event's sequence_id is not exactly `last_sequence_id + 1`. **Per-actor scoped.**

### `last_suggestion: Option<EntityTriple>`
The most recent output of `suggest_activation`. Purely informational.

### `state_hash: Option<[u8; 32]>`
Optional SHA-256 hash of the serialized SchedulerState (semantic fields only). Enables external integrity verification.

## Diagnostic State Fields (Non-Semantic)

### `replay_buffer: VecDeque<(u64, SchedulerEvent)>`

**HARD INVARIANT**: Non-semantic diagnostic structure only. MUST NOT be used as a source of truth, fallback store, reconstruction mechanism, or for any behavioral decision.

Bounded diagnostic FIFO buffer of the last N consumed events. Default capacity: 1024. Best-effort; ephemeral; lost on process restart.

**Permitted use**: Debugging, post-mortem analysis, recent-history inspection.
**Prohibited use**: State reconstruction, determinism validation, recovery logic, gap filling, seeding scheduler state after restart, influencing `suggest_activation`.

## Determinism Proof

Given **observed** event stream E = [e1, e2, ..., en]:

1. Initial state S0 is fixed (semantic fields only)
2. Each event ei updates semantic state deterministically via pure reducer: `S_i = apply(S_{i-1}, ei)`
3. Sn = `apply(apply(...apply(S0, e1), e2), ..., en)`
4. Sn depends only on E and S0, not on timing, load, event_id values, loss configuration, replay buffer contents, or concurrency execution order
5. **Concurrency linearization**: Any concurrent execution that consumes E MUST produce Sn. Concurrent drain interleaving, Tokio task scheduling order, and mutex acquisition timing affect only performance — never logical state
6. **Two SchedulerStates with identical semantic fields but different replay buffer contents are deterministically equivalent**
