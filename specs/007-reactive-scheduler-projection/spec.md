# CORE-007 Reactive Scheduler & Deterministic Projection Engine

**Feature**: `core-007-reactive-scheduler-deterministic-projection-engine`
**Created**: 2026-06-08
**Status**: Draft

## Clarifications

### Session 2026-06-08

- Q: What is the default DropPolicy for the bounded event bus — lossy or lossless? → A: Hybrid — default is lossless (Block policy), but lossy mode (DropNewest) can be explicitly enabled per deployment. Lossless mode uses backpressure signaling to the Actor when the buffer approaches capacity, with a configurable high-water mark that triggers blocking only before overflow is imminent.
- Q: Should the replay_buffer be treated as a deterministic source of truth or a diagnostic window? → A: Diagnostic only — the replay buffer is best-effort, used for debugging and recent-history verification. It is NOT a source of truth for full state reconstruction (see Section 7.0 Canonical Determinism Definition).
- Q: Should event_id use UUID v4 (random) or deterministic hash (SHA-256)? → A: Deterministic hash (SHA-256 of canonical event payload) — event_id is an identity annotation layer, NOT part of the determinism proof. Determinism is a function of the event stream sequence, not of event_id values.
- Q: What observability signals should the Scheduler expose? → A: Core metrics bundle — event consumption rate (events/sec), total_events_consumed, detected_gaps, last_sequence_id, suggestion produced/consumed ratio.
- Q: What should CORE-007 do when it detects gaps in the event stream? → A: Option A — Observability Only (Passive Model). Gaps are detected and recorded; system continues normally; no recovery attempt. External recovery is an optional extension outside CORE-007 scope.
- Q: What is the formal invariant for observed stream dependency? → A: Observed stream is the ONLY source of determinism. Loss, truncation, or partial view are intrinsic system properties. Classification: hard invariant.
- Q: What is the formal global ordering model? → A: Per-actor ordering only is a hard invariant. No global ordering will ever exist. Classification: hard invariant.
- Q: What is the recovery boundary? → A: CORE-007 MUST remain non-self-healing. Recovery is strictly external responsibility. Classification: hard invariant.
- Q: How should semantic naming model risk be resolved? → A: Option A — keep current names; add a dedicated "Semantic Model Clarification" section documenting correct semantic reinterpretation. Classification: non-functional architectural risk only.

---

## 1. Objective

Design and formalize a reactive scheduling layer (CORE-007) that operates above CORE-006 runtime.

CORE-007 is **not** an execution system. It is a deterministic, event-driven projection engine that:

- Observes CORE-006 execution events
- Maintains a deterministic scheduling state
- Produces activation suggestions (not commands)
- Remains fully decoupled from execution authority

---

## 2. Core Principles

### P1 — Actor is Execution Authority (CORE-006 Invariant)

- Only the Actor executes commands
- The Scheduler MUST NEVER execute or block execution
- Command dispatch flows through the CORE-006 runtime, not through CORE-007

### P2 — Reactive-only Scheduler Model

- The Scheduler is purely event-driven
- The Scheduler does **not** poll any state
- The Scheduler reacts **only** to events emitted by CORE-006 actors

### P3 — Deterministic Projection

Given identical **observed** event streams:

- The resulting `SchedulerState` MUST be identical
- The `suggest_activation` output MUST be identical
- Determinism depends only on stream contents, not on event_id, not on loss configuration, and not on the replay buffer

### P4 — No Control Authority

The Scheduler MUST NOT:

- Block Actor execution
- Mutate Actor state
- Enforce scheduling decisions
- Act as a gatekeeper for command dispatch

---

## 3. System Boundary

### CORE-006 (Runtime Layer) — Responsible for:

- Command execution and validation
- Entity state transitions
- Domain event emission
- Mailbox management and bounded queuing
- Actor lifecycle (activation, passivation, recovery)

### CORE-007 (Reactive Layer) — Responsible for:

- Consuming CORE-006 execution events via bounded event bus
- Reconstructing scheduling state from event stream
- Running deterministic policy evaluation
- Producing activation suggestions (advisory only)

### 3.3 Observability Signals

The Scheduler MUST expose the following core metrics for operational monitoring:

| Metric | Type | Description |
|--------|------|-------------|
| `scheduler.events.consumed.total` | Counter (u64) | Lifetime count of consumed events |
| `scheduler.events.consumed.rate` | Gauge (f64) | Event consumption rate (events/sec) |
| `scheduler.events.gaps.total` | Counter (u64) | Cumulative detected gaps |
| `scheduler.events.last_sequence_id` | Gauge (i64) | Sequence ID of the most recent consumed event |
| `scheduler.suggestions.produced` | Counter (u64) | Number of activation suggestions emitted |
| `scheduler.suggestions.consumed` | Counter (u64) | Number of suggestions accepted by the runtime |

---

## 4. Core Data Model

### 4.1 SchedulerEventEnvelope

Each event flowing from CORE-006 to CORE-007 MUST include:

| Field | Type | Description |
|-------|------|-------------|
| `event_id` | SHA-256 hash of canonical payload | Deterministic identity annotation for the event. Enables idempotent event reference and cross-referencing. NOT part of the determinism proof — determinism depends on event stream sequence, not event_id values |
| `sequence_id` | u64 (monotonic per Actor) | Ordering identifier, no gaps within an Actor stream |
| `event_type` | Enum | Classification of the event (ExecutionCompleted, RecoveryCompleted, etc.) |
| `payload` | Structured data | Event-specific fields (entity triple, state version, timestamps) |
| `source_actor` | EntityTriple | The Actor that emitted this event |

### 4.2 SchedulerState

The Scheduler maintains a deterministic projection state:

| Field | Type | Description |
|-------|------|-------------|
| `total_events_consumed` | u64 | Lifetime count of processed events |
| `last_sequence_id` | Option\<u64\> | Sequence ID of the most recent event consumed |
| `detected_gaps` | u64 | Cumulative count of detected sequence gaps |
| `replay_buffer` | VecDeque\<(u64, SchedulerEvent)\> | Bounded diagnostic buffer of recent events (capacity: 1024). Best-effort, used for debugging and verification only. NOT a source of truth for full state reconstruction |
| `last_suggestion` | Option\<EntityTriple\> | The most recent activation suggestion produced |
| `state_hash` | Option\<[u8; 32]\> | Optional cryptographic snapshot hash for state integrity verification |

### 4.3 SchedulingPolicy

A pure function:

```
suggest_activation(state: SchedulerState, pending_entities: Set<EntityTriple>) -> Option<EntityTriple>
```

**Constraints**:

- No side effects — pure computation only
- No time dependency — output depends solely on inputs
- MUST be deterministic — identical inputs produce identical outputs
- MUST complete within bounded time

---

## 5. Event Flow Model

### 5.1 Core Actor Flow (CORE-006)

```
Command received ──► Actor executes ──► State updated ──► Event emitted
```

This is the CORE-006 execution path. CORE-007 does not modify this flow.

### 5.2 Reactive Event Flow (CORE-007)

```
Event received ──► Scheduler updates SchedulerState ──► Policy evaluation ──► Suggestion emitted (advisory)
```

This flow is decoupled from Actor execution. The Scheduler processes events asynchronously and produces suggestions that the runtime may choose to accept or ignore.

### 5.3 Combined Flow

```
┌─────────────────────────────────────────────────────────────────┐
│  CORE-006 Runtime                                               │
│  ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────────┐ │
│  │ Command  │──▶│  Actor   │──▶│  State   │──▶│   Event Bus  │─┼──▶ domain events
│  │ Receive  │   │ Execute  │   │  Update  │   │  (bounded)   │ │
│  └──────────┘   └──────────┘   └──────────┘   └──────┬───────┘ │
│                                                       │         │
└───────────────────────────────────────────────────────┼─────────┘
                                                        │
                                                        ▼
                                        ┌───────────────────────────┐
                                        │  CORE-007 Reactive Layer  │
                                        │  ┌─────────────────────┐  │
                                        │  │  SchedulerState     │  │
                                        │  │  (deterministic     │  │
                                        │  │   projection)       │  │
                                        │  └─────────┬───────────┘  │
                                        │            ▼              │
                                        │  ┌─────────────────────┐  │
                                        │  │  SchedulingPolicy   │  │
                                        │  │  (pure function)    │  │
                                        │  └─────────┬───────────┘  │
                                        │            ▼              │
                                        │  ┌─────────────────────┐  │
                                        │  │  Suggestion         │  │
                                        │  │  (advisory only)    │  │
                                        │  └─────────────────────┘  │
                                        └───────────────────────────┘
```

---

## 6. Backpressure Model

### 6.1 Bounded Capacity

The event bus connecting CORE-006 to CORE-007 MUST have bounded capacity. Default: 4096 events.

### 6.2 Drop Policy

The event bus supports a configurable drop policy. **Default is lossless (`Block`).** A lossy mode (`DropNewest`) can be explicitly enabled per deployment.

| Policy | Behavior | Use Case |
|--------|----------|----------|
| `Block` **(default)** | The sender blocks until buffer space is available. A configurable high-water mark (default: 90%) triggers blocking before overflow. | Deployments prioritizing completeness of the observed event stream |
| `DropNewest` (opt-in) | The incoming event is silently dropped; counter increments. Actor execution is never blocked. | High-throughput deployments that accept degraded suggestion quality for zero Actor impact |
| `DropOldest` (opt-in) | The oldest buffered event is evicted; the newest is accepted | Systems that favor completeness over recency |

### 6.3 Gap Detection

When events are dropped, regardless of policy:

- `detected_gaps` counter MUST increment
- The gap range MUST be logged at debug level
- The Actor execution path MUST NOT be affected

---

## 7. Determinism Model (Canonical Definition)

CORE-007's determinism guarantee is defined by a single unified model that cleanly separates event identity, event loss, and replay semantics from the core determinism claim.

### 7.0 Canonical Determinism Definition

**HARD INVARIANT**: Determinism = a pure function of the **observed event stream** only. The observed stream is the ONE AND ONLY source of determinism.

```
invariant determinism_property:
  given identical observed streams E1 and E2
    → SchedulerState(E1) == SchedulerState(E2)
    → suggest_activation(SchedulerState(E1)) == suggest_activation(SchedulerState(E2))
```

**Corollary**: Loss, truncation, or partial view are intrinsic system properties. Determinism is defined over whatever event sequence the Scheduler actually observes; completeness is a separate concern.

The following are **independent** of the determinism proof:

| Dimension | Relationship to Determinism |
|-----------|----------------------------|
| **Event loss** | Loss affects **completeness**, NOT determinism. Two Schedulers observing the same subsequence will produce identical state for that subsequence, regardless of whether events were lost upstream |
| **Event identity (event_id)** | event_id is an **identity annotation layer**, not part of the determinism function. Determinism depends on event payloads and sequence order, not on hash values |
| **Replay buffer** | The buffer is a **diagnostic snapshot window**, not a determinism source. The determinism guarantee is proven via the event stream projection function, not via buffer contents |

### 7.1 Event Stream → SchedulerState

Given two identical observed event streams (same events, same sequence, same order), the SchedulerState MUST be identical regardless of:

- Wall-clock timing differences
- System load variations
- Processing order of concurrent events from different Actors

### 7.2 SchedulerState → Suggestion

Given two identical SchedulerState snapshots, `suggest_activation` MUST produce identical output.

### 7.3 Replay Invariant (Diagnostic Only)

Replaying the last N events (from the replay buffer) on a fresh SchedulerState MUST reconstruct the same state for those N events as the original processing produced. This holds for any N up to the replay buffer capacity. This is a **diagnostic invariant** — it verifies that the projection function is deterministic for the buffer contents but does NOT guarantee full-state reconstruction from process start. The replay buffer is NOT part of the determinism proof.

---

## 8. Gap Handling Model

**Gap Behavior Model**: Observability Only (Passive Model) — gaps are detected, recorded, and exposed via metrics. The system never attempts recovery or emits recovery signals. External gap recovery is strictly outside CORE-007 scope. This is an architectural invariant — see Section 14.3.

### 8.1 Detection

The Scheduler MUST:

- Detect missing `sequence_id` values in the consumed event stream
- Track gap ranges (start_seq, end_seq) for observability
- Expose metrics via the core bundle defined in Section 3.3 (`scheduler.events.gaps.total`, `scheduler.events.last_sequence_id`)

### 8.2 Recovery (Hard Invariant)

The Scheduler MUST **not** attempt automatic recovery from gaps, emit structured recovery signals, or participate in recovery orchestration. Gap recovery is strictly an external responsibility that operates outside the CORE-007 reactive loop. This is a hard architectural invariant — not a configurable behavior and not an extension point within CORE-007.

### 8.3 Behavior Under Gaps

When gaps are detected, the Scheduler:

- Continues operating normally (does not pause or degrade)
- Uses available state to produce best-effort suggestions
- Logs gap information for external monitoring

---

## 9. Non-Goals

CORE-007 MUST NOT:

- Execute commands in place of Actors
- Control the Actor lifecycle (activation, passivation, recovery)
- Enforce scheduling decisions (suggestions are advisory)
- Own or manage mailboxes
- Replace any part of the CORE-006 runtime
- Provide persistence for scheduling state (state is ephemeral and reconstructable)
- Act as a gatekeeper for command dispatch
- Implement automatic gap recovery
- Emit structured recovery signals (observability is the boundary; recovery orchestration is strictly external)

---

## 10. User Scenarios & Testing

### User Story 1 — Reactive Scheduling (Priority: P1)

A runtime operator observes that the Scheduler produces activation suggestions based on observed execution events. The suggestions follow the configured policy and are reproducible given the same event sequence.

**Acceptance Scenarios**:

1. **Given** an active CORE-006 runtime with entities processing commands, **When** the Actor emits `ExecutionCompleted` events, **Then** the Scheduler consumes them and updates SchedulerState.
2. **Given** a SchedulerState that has consumed events, **When** `suggest_activation` is called, **Then** it returns a valid `EntityTriple` or `None`.
3. **Given** identical event streams fed to two Scheduler instances, **When** both process all events, **Then** their SchedulerState is identical.

### User Story 2 — Backpressure Under Load (Priority: P2)

Under load, the event bus applies the configured drop policy. The default lossless mode provides backpressure; the opt-in lossy mode drops events without blocking Actors. The Scheduler detects and reports gaps in both modes.

**Acceptance Scenarios**:

1. **Given** an event bus with capacity 10 and `DropNewest` policy, **When** 100 events are emitted faster than the Scheduler consumes them, **Then** the Scheduler observes `detected_gaps > 0`.
2. **Given** a `DropNewest` policy, **When** the buffer is full, **Then** new events are dropped and the Actor execution is not blocked.
3. **Given** the default `Block` policy, **When** the buffer exceeds the high-water mark (90%), **Then** the sender blocks until buffer space is available, and no events are lost.

### User Story 3 — Diagnostic Replay Verification (Priority: P3)

An operator replays recent events from the replay buffer to verify the last N events' correctness. Full state reconstruction uses the CORE-006 persistence layer, not the buffer.

**Acceptance Scenarios**:

1. **Given** a populated replay buffer, **When** the last N events are replayed on a fresh SchedulerState, **Then** the resulting state for those N events matches the original.
2. **Given** a replay buffer bounded to 1024 events, **When** more than 1024 events are consumed, **Then** only the most recent 1024 are retained (buffer is diagnostic, not authoritative).

### User Story 4 — Gap Detection and Monitoring (Priority: P3)

An operator monitors system metrics and observes gap information when events are dropped.

**Acceptance Scenarios**:

1. **Given** a running Scheduler, **When** events are dropped, **Then** `detected_gaps` increments and gap range information is logged.
2. **Given** a Scheduler with gaps, **When** `suggest_activation` is called, **Then** it still produces a suggestion based on available state.

---

## 11. Success Criteria

| Criterion | Metric | Verification |
|-----------|--------|--------------|
| Fully event-driven | The Scheduler processes events only; no polling exists | Code review confirms zero polling loops or timer-based reads |
| Deterministic projection | Identical observed event streams produce identical SchedulerState | Automated test with two Scheduler instances fed same event sequence; test passes regardless of loss configuration, event_id strategy, or replay buffer state |
| Bounded memory | Event bus capacity is configured and enforced (both default Block and opt-in DropNewest) | Test with overflow in DropNewest mode: events are dropped, memory does not grow unbounded. Block mode: sender pauses, buffer never exceeds capacity |
| Actor isolation | Actor execution throughput is identical with and without Scheduler load | Measure command throughput with/without event bus producer load |
| Stable suggestions | Same SchedulerState + same pending set = same suggestion | Property-based test across 1000 random inputs |
| CORE-006 unchanged | No modifications required in CORE-006 execution path | Diff analysis: CORE-006 files are unmodified |

---

## 12. Dependencies

| Dependency | Type | Description |
|------------|------|-------------|
| CORE-006 Runtime | Hard | CORE-007 consumes events emitted by CORE-006 actors |
| SchedulerEventEnvelope format | Contract | The event envelope contract defines the interface between CORE-006 emission and CORE-007 consumption |
| Bounded event bus | Infrastructure | The channel/bus connecting the layers must support bounded capacity and configurable drop policy |

---

## 13. Assumptions

1. **HARD INVARIANT**: Events are ordered per-Actor stream only. No global ordering exists across Actors at the CORE-007 level and none will be introduced. Cross-Actor ordering is not guaranteed
2. The replay buffer is ephemeral — lost on process restart; only the last N events are available for replay
3. **HARD INVARIANT**: CORE-007 is non-self-healing. Gap recovery is strictly external (see Section 14.3)
4. The Scheduler runs in the same process as the CORE-006 runtime (not a separate service)

---

## 14. Architectural Boundary Model

This section defines the definitive architectural boundary model for CORE-007. Each dimension is classified as **hard invariant** — intentional, non-negotiable design property — not a gap, risk, or extension point.

### 14.1 Observed Stream Dependency — HARD INVARIANT

**Statement**: Determinism is defined exclusively over the observed event stream. The Scheduler makes no claim about completeness, and loss does not degrade the deterministic guarantee.

**Classification**: Hard invariant (not configurable, not an extension point, not a gap).

**Implications**:
- No "degraded determinism mode" exists — determinism is always unconditional over whatever events are observed
- Completeness tracking (gaps, metrics) is orthogonal to the determinism proof
- Future extensions that introduce replay-based state recovery operate outside CORE-007's determinism model

### 14.2 Global Ordering Model — HARD INVARIANT

**Statement**: CORE-007 operates under per-Actor ordering only. No global ordering will ever exist within CORE-007.

**Classification**: Hard invariant (not an extension point, not a future CORE-007 feature).

**Implications**:
- The Scheduler processes events from different Actors in an unspecified order
- `suggest_activation` is defined over the per-Actor partial order, not a total order
- Cross-Actor causal ordering, if needed, must be provided by a separate layer above CORE-007
- This invariant simplifies the state projection function (no vector clocks, no causal tracking)

### 14.3 Recovery Boundary — HARD INVARIANT

**Statement**: CORE-007 is non-self-healing. Recovery from gaps, loss, or state divergence is strictly an external responsibility.

**Classification**: Hard invariant (not an extension point within CORE-007; external extensions are permitted).

**Implications**:
- The Scheduler exposes gap observability (metrics, logs) for external consumption
- The Scheduler never initiates, requests, or participates in recovery operations
- The replay buffer exists for diagnostic verification only — not for recovery seeding
- External recovery (e.g., CORE-008+) may reconstruct SchedulerState from persisted CORE-006 events, but this operates outside CORE-007 boundaries

---

## 15. Semantic Dual Layer Model

**Classification**: Non-functional architectural interpretation model only. No determinism or invariants are modified by this section.

CORE-007 MUST be interpreted under two independent semantic layers:

| Layer | Role | Normativity |
|-------|------|-------------|
| **Semantic Layer** (Canonical) | Defines system behavior, invariants, and architecture | **NORMATIVE** — single source of truth |
| **Lexical Layer** (Historical) | Provides names used in code and documentation | **NON-NORMATIVE** — does not define behavior |

### 15.1 Semantic Layer — Canonical (NORMATIVE)

This is the only layer that defines system behavior. It includes:

- Determinism Model (Section 7.0): `function(observed_event_stream)` — hard invariant
- SchedulerState projection rules (Section 4.2): pure function, no side effects
- SchedulingPolicy contract (Section 4.3): pure function, bounded time, deterministic
- Event Flow semantics (Section 5): reactive-only, no polling, advisory output
- Gap handling invariants (Section 8): passive detection, no recovery
- Backpressure semantics (Section 6): bounded bus, hybrid Block/Drop policy
- Architectural boundaries (Sections 2, 14): P1–P4, three hard invariants

**Rule**: Only this layer defines system truth. It is immutable and independent of names.

### 15.2 Lexical Layer — Historical (NON-NORMATIVE)

This layer provides the names used in code and documentation. It carries forward legacy scheduling terminology but does NOT define behavior.

| Term | Lexical Form | Semantic Interpretation (What it IS) |
|------|-------------|--------------------------------------|
| **Scheduler** | Code name for the projection engine | A **deterministic event stream projection engine** that maintains state and produces advisory suggestions. Never executes, blocks, or enforces |
| **SchedulingPolicy** | Code name for the activation function | A **pure function** that, given projected state and pending entities, returns an advisory suggestion. The runtime may accept or ignore |
| **ReplayBuffer** | Code name for the diagnostic window | A **bounded diagnostic window** over recent events. NOT a source of truth for state reconstruction (see Section 7.0, Section 14.1) |
| **GapDetection** | Code name for the passive monitor | A **passive observability monitor** that detects missing sequence IDs, records them, and continues. No recovery action is ever initiated (see Section 14.3) |

**Rules**:
- This layer does NOT define behavior
- This layer cannot modify invariants
- This layer exists solely for code compatibility and continuity

### 15.3 Semantic Priority Rule (Hard Interpretation Invariant)

> **En caso de conflicto entre niveles, la capa Semántica (15.1) prevalece sobre la capa Léxica (15.2).**

> In case of conflict between layers, the Semantic Layer (15.1) prevails over the Lexical Layer (15.2).

This is an architectural invariant about interpretation priority: no lexical name can modify, override, or contradict the behavior defined by the semantic layer.

**Corollary**: Any reader encountering a term from the Lexical Layer MUST interpret it through the Semantic Layer definition. The name is a label, not a specification.

### 15.4 Invariant Preservation

This entire section is documentation-only. It does not change:

- The hard invariants defined in Section 14
- The determinism model defined in Section 7.0
- The system boundaries defined in Section 3
- The non-goals defined in Section 9
- The user stories or success criteria defined in Sections 10–11

### 15.5 Rationale for Name Retention

| Factor | Assessment |
|--------|------------|
| **Code churn risk** | Full rename would touch 7+ documents, all contracts, all tasks, all planned source files |
| **Architectural clarity** | The dual-layer model makes interpretation explicit without renaming |
| **Downstream alignment** | CORE-006 already uses "Scheduler" — aligned naming reduces cross-layer confusion |
| **Reader burden** | A single clarification section is lower cost than learning an entirely new vocabulary |
