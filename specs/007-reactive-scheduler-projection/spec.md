# CORE-007 Reactive Scheduler & Deterministic Projection Engine

**Feature**: `core-007-reactive-scheduler-deterministic-projection-engine`
**Status**: Draft

## Clarifications

### Session 2026-06-08

- Q: How should determinism factors be categorized (DropPolicy vs concurrency vs others)? → A: Merge DropPolicy into the §4 canonical rule text: "Determinism is SchedulerState = f(observed_stream) where observed_stream ≡ all events surviving DropPolicy." Keep all other factors as "not inputs to f." No tier restructuring.
- Q: How should "completeness" be handled in the spec? → A: Remove "completeness" entirely. Replace with explicit positive framing: the observed stream is authoritative. The system has no concept of a "full stream" of all emitted events; only the post-DropPolicy stream exists from the scheduler's perspective.
- Q: (user-directed) ReplayBuffer, ordering, advisory-only, and concurrency semantics → A: Four-fold: (1) ReplayBuffer is strictly diagnostic-only — must never be used for state reconstruction, determinism validation, or recovery; differences in buffer content MUST NOT affect state equivalence. (2) Ordering is strictly per-entity — no cross-entity ordering exists or is inferred; each entity stream is fully isolated; sequence_id MUST NOT be compared across entities. (3) Scheduler outputs are strictly advisory — suggest_activation is NOT a command; execution authority belongs exclusively to CORE-006; Scheduler must not influence execution directly or indirectly. (4) Concurrency is an implementation detail only — correctness is defined over sequential application of the observed event stream; internal execution order may vary without affecting final state; the system MUST be equivalent to a single-threaded deterministic execution.
- Q: Is SchedulerState per-entity map-based or single-stream? → A: Single-stream. Flat fields track the currently projected entity. last_sequence_id and detected_gaps are per-actor scoped within the current projection context. SchedulerState represents projection of one entity stream at a time; state resets when the observed stream switches entities. No cross-entity state is retained between projection cycles.
- Q: What is the event bus ownership model? → A: Single consumer, multi-producer, Scheduler-owns-bus. Scheduler creates channel via event_bus_channel(), holds SchedulerEventReceiver for its lifetime (single consumer). SchedulerEventSender is Clone — clones distributed to CORE-006 actors (multi-producer). Dropping Scheduler closes channel; senders get SendError.
- Q: (user-directed) Which SchedulerState fields may SchedulingPolicy access? → A: Policy MUST depend only on allowed semantic fields: `total_events_consumed` and `last_suggestion`. Forbidden fields: `replay_buffer` (diagnostic-only), `detected_gaps` (gap tracking, not decision-relevant), `last_sequence_id` (per-actor scoped, resets on entity switch), `state_hash` (integrity, not decision-relevant). Policy is a pure function over valid inputs only.
- Q: Should gap types be distinguished or treated uniformly? → A: Uniform treatment (no distinction). All gaps are treated identically — single `detected_gaps` counter, no per-cause classification. The scheduler cannot and does not distinguish DropPolicy loss from sequence discontinuity; such distinction would require knowledge of events outside the observed stream boundary.
- Q: Is RoundRobin fairness event-driven or entity-driven? → A: Event-driven. Cursor = `total_events_consumed % pending.len()`. Every consumed event advances the cursor regardless of which entity emitted it. Under skewed event distributions, high-event-rate entities dominate cursor positions. This is deterministic and predictable — not a defect. Advisory-only output (I3) means consumer may ignore suggestion.
- Q: (user-directed) What is SchedulerState's role — pure reducer or runtime engine? → A: SchedulerState is a PURE REDUCER OUTPUT — a deterministic projection artifact (data structure), NOT a runtime engine. apply() is a pure function (Event × SchedulerState → SchedulerState). All orchestration logic (bus drain, event loop, policy evaluation, suggestion output) lives in Scheduler, not SchedulerState.
- Q: (user-directed) Event bus behavior under high concurrency and DropPolicy → A: `try_send` is fire-and-forget — SendError is final (no retry). Each `try_send` is atomic per-event (no batch send). Ordering guarantees apply only to successfully enqueued events. DropPolicy applies strictly at enqueue time, not post-send. No retry orchestration exists in Scheduler.
- Q: (user-directed) Final pre-implementation consistency — 8 fixes applied → A: (1) SchedulerState pure reducer only — apply() does NO entity switch detection or reset logic beyond field updates. (2) Entity switch detection explicitly Scheduler-owned: `current_active_entity != event.source_actor` check before apply(). (3) RoundRobin uses BTreeSet (deterministic iteration); HashSet forbidden. (4) Bus semantics: concurrency does not define behavior, only arrival order after DropPolicy defines observed_stream. (5) ReplayBuffer write-only from Scheduler, read-only for diagnostics. (6) Gap detection triggered in Scheduler, SchedulerState stores counters only, no causal inference. (7) Policy inputs sealed — total_events_consumed and last_suggestion only. (8) No implicit assumptions — observed_stream is sole reality, DropPolicy defines stream boundary.
- Q: (user-directed) Scheduler pipeline decomposition → A: Scheduler refactored into a thin orchestration pipeline of 6 pure internal components: EventIngestor (drain only), EntityRouter (entity switch detection only), StateReducer (wraps SchedulerState::apply, pure), GapDetector (structural only: sequence_id != last + 1), PolicyEvaluator (calls suggest_activation only), SuggestionEmitter (writes last_suggestion only). Scheduler itself becomes composition-only — no business logic. All invariants I1-I7 unchanged. Zero behavior change.
- Q: (user-directed) Pipeline drift guard → A: Scheduler is a FIXED orchestration shell — it MUST NOT evolve. Forbidden: entity switch logic, gap detection, policy evaluation, conditional branching, derived values, state interpretation, module logic duplication. Allowed: function composition, data passing, execution order. Each responsibility lives in exactly one pipeline module. If logic appears in Scheduler beyond function calls: STOP and refactor immediately.

---

## 1. Objective

CORE-007 observes CORE-006 execution events via a bounded bus, maintains a deterministic projection state, and produces **advisory** activation suggestions via a pure SchedulingPolicy function.

Core properties:
- Reactive-only (no polling)
- Advisory output (no execution authority)
- Non-self-healing (recovery is external)
- Determined solely by the observed event stream

---

## 2. Data Model

### 2.1 SchedulerEventEnvelope

Events flowing from CORE-006 to CORE-007:

| Field | Type | Description |
|-------|------|-------------|
| `event_id` | `[u8; 32]` | SHA-256 of canonical payload. Identity annotation — not part of determinism |
| `sequence_id` | `u64` | Monotonic per Actor. **Per-actor scoped** — never compared across entities. No cross-entity ordering exists or is inferred. Each entity stream is fully isolated |
| `event_type` | `EventType` | `ExecutionCompleted` or `RecoveryCompleted` |
| `payload` | `SchedulerEvent` | Event-specific fields (entity, state_version) |
| `source_actor` | `EntityTriple` | The emitting Actor |

### 2.2 SchedulerState

Deterministic projection maintained by the Scheduler. **Single-stream model**: SchedulerState tracks exactly one entity's projection at a time. `last_sequence_id` and `detected_gaps` are per-actor scoped within the current projection context — they reflect the currently projected entity, not an aggregate of all entities. When the observed stream switches entities, per-entity tracking resets for the new entity. No cross-entity state is retained between projection cycles.

**SchedulerState is a pure reducer output — a deterministic projection artifact (data structure), NOT a runtime engine.** The `apply()` method is a pure function: `(Event, SchedulerState) → SchedulerState`. It performs state transformation only — no I/O, no bus interaction, no policy evaluation, no scheduling decisions, no entity switch detection, no reset logic beyond field updates. All orchestration logic (bus drain, event loop, policy evaluation, suggestion output, per-entity reset coordination) belongs to the `Scheduler` struct (`scheduler.rs`), not SchedulerState.

**Entity switch detection is Scheduler-owned**: The Scheduler detects entity switches via `current_active_entity != event.source_actor` BEFORE calling `apply()`. On entity switch, the Scheduler resets per-entity scoped fields externally (sets `last_sequence_id` to the new entity's sequence, resets `detected_gaps`). `apply()` receives the already-reset state — SchedulerState MUST NOT self-detect entity changes or self-trigger resets.

Fields split into two independent categories:

**Semantic fields** (participate in determinism and `suggest_activation`):

| Field | Type | Description |
|-------|------|-------------|
| `total_events_consumed` | `u64` | Lifetime count across all entities. Aggregate diagnostic — no ordering semantics |
| `last_sequence_id` | `Option<u64>` | Most recent `sequence_id` of the currently projected entity. Per-actor scoped within the current projection cycle. Resets when the observed stream switches to a different entity. Never compared across entities |
| `detected_gaps` | `u64` | Gap count for the currently projected entity's contiguous stream segment. Per-actor scoped within the current projection cycle. Resets when the observed stream switches entities |
| `last_suggestion` | `Option<EntityTriple>` | Most recent advisory suggestion |
| `state_hash` | `Option<[u8; 32]>` | Optional snapshot hash for integrity |

**Diagnostic field** (non-semantic — no behavioral role):

| Field | Type | Description |
|-------|------|-------------|
| `replay_buffer` | `VecDeque<(u64, SchedulerEvent)>` | Bounded (1024), ephemeral, lost on restart. Diagnostic-only: debugging, post-mortem. **Never used for reconstruction, determinism validation, or recovery** |

### 2.3 SchedulingPolicy

Pure function:

```
suggest_activation(state: &SchedulerState, pending: &BTreeSet<EntityTriple>) -> Option<EntityTriple>
```

Constraints:
- Pure — no side effects, no wall-clock dependency
- Deterministic — identical inputs produce identical output. **RoundRobin MUST operate on a deterministic ordered collection** (`BTreeSet<EntityTriple>` or `Vec<EntityTriple>` sorted before use). Iteration over `HashSet` is forbidden for scheduling decisions — `HashSet` iteration order is non-deterministic in Rust
- Bounded time — O(pending) or better
- `pending` is an **unordered set** — policies select by entity identity, never by cross-entity `sequence_id`. No cross-entity ordering exists or is inferred. The collection implementation MUST provide deterministic iteration order for the same set of entities
- Output is **advisory only** — `suggest_activation` is NOT a command; the Scheduler MUST NOT influence execution directly or indirectly. Execution authority belongs exclusively to CORE-006
- **Field access scope** — Policy MAY only read from `state`:
  - ✅ `total_events_consumed` (aggregate counter, valid for cursor-based policies like RoundRobin)
  - ✅ `last_suggestion` (previous suggestion, valid for deduplication or rotation)
  - ❌ `replay_buffer` (diagnostic-only, no behavioral role — I4)
  - ❌ `detected_gaps` (gap tracking, not decision-relevant)
  - ❌ `last_sequence_id` (per-actor scoped, resets on entity switch — not globally meaningful)
  - ❌ `state_hash` (integrity, not decision-relevant)
- Policy MUST NOT depend on any forbidden field. Violation breaks determinism purity
- **Fairness model**: RoundRobin is event-driven — the cursor advances on every consumed event (`total_events_consumed % pending.len()`), not per suggestion emitted. Under skewed event distributions (one entity emits many more events than others), high-event-rate entities occupy more cursor positions. This is deterministic and predictable, not a fairness defect. The advisory-only output (I3) means the consumer may accept or ignore any suggestion. Alternative fairness models (entity-driven, weighted) can be implemented as custom `SchedulingPolicy` implementations within the same field-access constraints

---

## 3. Event Flow

```
CORE-006: Command → Actor executes → State updated → Event emitted
                     CORE-007 reads via bounded bus (never modifies CORE-006)
CORE-007: Event received → SchedulerState updated → Policy evaluated → Suggestion (advisory)
```

Execution authority belongs exclusively to CORE-006. CORE-007's `suggest_activation` output is strictly advisory — it is never a command, and Scheduler MUST NOT influence execution directly or indirectly.

---

## 4. Determinism

**CANONICAL RULE**:

```
Determinism = SchedulerState = f(observed_stream)
where observed_stream ≡ all events that survive DropPolicy
```

```
Given identical observed streams E1 and E2 (post-DropPolicy):
  → SchedulerState(E1) == SchedulerState(E2)
  → suggest_activation(SchedulerState(E1)) == suggest_activation(SchedulerState(E2))
```

The observed stream — defined as the event sequence after DropPolicy has been applied — is the **sole** input to determinism. DropPolicy is **part of the stream definition**: it determines which events comprise the observed stream, and is not a factor that violates or conditions determinism.

**Factors that are NOT inputs to f** (do not affect SchedulerState):
- Internal execution order and concurrency scheduling — concurrency is an implementation detail only. Correctness is defined over sequential application of the observed event stream. Internal execution order MAY vary without affecting final state. The system MUST be equivalent to a single-threaded deterministic execution
- `event_id` values (identity annotation, not behavioral)
- Replay buffer — diagnostic-only; never used for state reconstruction, determinism validation, or recovery. ReplayBuffer differences MUST NOT affect equivalence of SchedulerState
- Event loss — DropPolicy-dropped events define the stream boundary; the scheduler has no concept of, and no access to, pre-DropPolicy events. The observed stream is authoritative. Loss is not a correctness defect
- System load, CPU contention, wall-clock timing (may affect *when* events are processed, never *what* SchedulerState results)

**No hidden "full stream" assumption**: The scheduler receives and processes only what the bus delivers after DropPolicy. Events that never arrive (due to DropPolicy or any other cause) are outside the system boundary. The scheduler is neither aware of nor dependent on them. There is no ideal "complete" stream against which the observed stream is measured.

**No ReplayBuffer as truth source**: The replay buffer is strictly diagnostic — a bounded, ephemeral window for debugging and post-mortem analysis only. It carries zero semantic weight. Two SchedulerState instances with identical semantic fields are equivalent regardless of ReplayBuffer content. ReplayBuffer has no code path to state reconstruction, determinism validation, or recovery.

**Concurrency is an implementation detail**: Correctness is defined over sequential application of the observed event stream. Any concurrent or parallel processing MUST produce the same SchedulerState as sequential processing. Internal execution order may vary; the system MUST be equivalent to a single-threaded deterministic execution. No dependency on async runtime ordering semantics (Tokio or equivalent).

---

## 5. Backpressure & DropPolicy

Event bus: bounded capacity (default 4096). Configurable drop policy:

| Policy | Behavior |
|--------|----------|
| `Block` (default) | Sender blocks until space available. High-water mark at 90%. |
| `DropNewest` | Incoming event silently dropped; counter incremented |
| `DropOldest` | Oldest buffered event evicted; newest accepted |

**DropPolicy is fully deterministic**: Given identical event arrival order, buffer capacity, and policy config, the same events are dropped on every execution. Load, CPU, concurrency timing affect *whether* drops occur — never *which* events are dropped.

### 5.1 Bus Ownership & Lifecycle

- **Channel creation**: The Scheduler creates the bounded channel via `event_bus_channel()`, receiving the `(SchedulerEventSender, SchedulerEventReceiver)` pair
- **Single consumer**: The Scheduler owns the `SchedulerEventReceiver` exclusively for its entire lifetime. No other component may receive from the bus. This prevents double consumption
- **Multi-producer**: `SchedulerEventSender` is `Clone`. Clones are distributed to CORE-006 actors. Each actor independently sends events via `try_send()`. The channel remains open as long as the Scheduler holds the receiver — dropping all sender clones does not close the channel
- **Shutdown**: When the Scheduler is dropped, the receiver is dropped, closing the channel. Subsequent `try_send()` calls return `SendError`. Senders detect closure and stop emitting. No explicit shutdown coordinator is required
- **No fan-out**: There is exactly one event bus and one consumer. Multiple Scheduler instances (if any) each have their own independent bus

### 5.2 Send Semantics

- **Fire-and-forget**: `try_send()` is non-blocking. `SendError` is final — no retry mechanism exists in Scheduler or bus. The caller (CORE-006 actor) is responsible for any retry logic, but retry is not required or expected
- **Atomic per-event**: Each `try_send()` call is atomic — either the event is fully enqueued or it is not. No partial or batch send. Ordering is per-send: if `try_send(A)` succeeds before `try_send(B)` is called, A appears before B in the channel
- **Ordering scope**: Ordering guarantees apply only to successfully enqueued events. Events dropped by `DropNewest` or `DropOldest` are never observed by the Scheduler and carry no ordering semantics
- **DropPolicy at enqueue time**: DropPolicy is evaluated strictly at the moment `try_send()` is called. If the buffer is full, the policy determines whether the incoming event is dropped (`DropNewest`), an old event is evicted (`DropOldest`), or the sender blocks (`Block`). No post-send adjustment or retry orchestration exists
- **No retry orchestration**: The Scheduler never orchestrates retries. It only drains events from the receiver. The bus has no retry buffer, dead-letter queue, or backpressure propagation beyond channel closure

---

## 6. Gap Handling

- Gaps detected when consumed `sequence_id` != last `+ 1` within the currently projected entity's contiguous stream segment
- When the observed stream switches entities, `last_sequence_id` resets — gap tracking starts fresh for the new entity. No cross-entity gap inference
- Gaps may arise from DropPolicy drops or from sequence discontinuities; the scheduler treats all gaps uniformly — no gap-type classification, no per-cause metrics, no attribution. The scheduler cannot distinguish gap causes: such distinction would require knowledge of events outside the observed stream boundary. All gaps result in identical behavior: increment `detected_gaps`, log, continue
- On gap: increment `detected_gaps`, log at debug, continue normally
- No recovery attempted. No recovery signals emitted. Recovery is strictly external.
- Event loss (dropped events, missing sequence numbers) does NOT break determinism — it is part of the observed stream definition

---

## 7. User Scenarios

### US1 — Reactive Scheduling (P1) 🎯 MVP
Events consumed → state updated → policy evaluated → advisory suggestion produced.
- Determinism: two instances fed identical streams → identical state

### US2 — Control Plane Isolation (P1)
CORE-006 execution path is independent of CORE-007 output. Suggestions are advisory only.

### US3 — Per-Actor Ordering Only (P1)
`sequence_id` values never compared across entities. `pending` is unordered. Policy selection by entity identity, not cross-entity sequence.

### US4 — Backpressure (P2)
DropNewest drops without blocking Actor. Block prevents loss. DropPattern deterministic under identical arrival order.

### US5 — Diagnostic Replay (P3)
Replay buffer bounded to 1024. Diagnostic inspection only — never reconstruction.

### US6 — Gap Detection (P3)
Gaps detected per-actor. Metrics exposed. System continues under gaps.

---

## 8. Success Criteria

CORE-007 is complete when:
1. Two instances fed identical observed streams produce identical semantic state
2. `suggest_activation` is a pure, deterministic function (property-tested, 1000 inputs)
3. DropPattern matches identical arrival order under any load
4. Zero polling loops exist (code review)
5. Zero cross-entity `sequence_id` comparisons exist
6. Replay buffer has zero code paths for reconstruction/recovery
7. Concurrent drain = sequential drain equivalence
8. CORE-006 files are unmodified (`git diff` empty)

---

## 9. Consolidated Invariants

| # | Invariant | Where Enforced |
|---|-----------|---------------|
| I1 | **Determinism**: SchedulerState = f(observed_stream) only, where observed_stream ≡ events surviving DropPolicy. Internal execution order, concurrency scheduling, replay buffer, event_id, system load are not inputs to f. SchedulerState is a pure reducer (data structure); `apply()` is a pure function (Event × S → S). Orchestration lives in Scheduler, not SchedulerState | §2.2, §4, SchedulerState::apply |
| I2 | **Per-entity ordering**: `sequence_id` scoped to entity stream. Never compared across entities. No cross-entity ordering exists or is inferred. Each entity stream is fully isolated. `pending` is unordered. SchedulerState tracks one entity at a time; state resets on entity switch | §2.1, §2.2, §2.3, scheduler drain loop |
| I3 | **No execution authority**: CORE-007 output is purely advisory. `suggest_activation` is NOT a command. Scheduler MUST NOT influence execution directly or indirectly. Execution authority belongs exclusively to CORE-006 | §2.3, §3, trait contract |
| I4 | **ReplayBuffer is non-semantic**: Diagnostic-only. Never used for reconstruction, determinism validation, or recovery. ReplayBuffer differences MUST NOT affect SchedulerState equivalence. Two states with different buffers are semantically equivalent | §2.2, visibility restricted |
| I5 | **Deterministic DropPolicy**: Drop outcomes depend only on event arrival order + buffer capacity + policy type. Load/concurrency affect timing only | §5, `try_send` impl |
| I6 | **Single-consumer bus**: Scheduler owns SchedulerEventReceiver exclusively (no double consumption). SchedulerEventSender is Clone (multi-producer). Dropping Scheduler closes channel. One consumer per bus — no fan-out | §5.1, event bus factory |
| I7 | **Policy field access**: `suggest_activation` MAY only read `total_events_consumed` and `last_suggestion` from `state`. MUST NOT read `replay_buffer`, `detected_gaps`, `last_sequence_id`, or `state_hash`. Policy is a pure function over allowed inputs only | §2.3, trait contract |
