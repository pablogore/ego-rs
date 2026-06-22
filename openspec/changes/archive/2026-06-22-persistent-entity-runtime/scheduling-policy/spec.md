# Scheduling Policy Model: CORE-006 Persistent Entity Runtime

**Feature**: `006-persistent-entity-runtime/scheduling-policy`
**Created**: 2026-06-07
**Status**: Draft

**Parent Spec**: [../spec.md](../spec.md) (CORE-006 Canonical, Section 10: Known Architecture Debt — Gap #5)

**Input**: Define the final deterministic Scheduling Policy model for CORE-006 Persistent Entity Runtime, consolidating all architectural decisions resolved during the pre-scheduling consistency audit.

**Prerequisite Audit**: [../scheduling-consistency-audit/spec.md](../scheduling-consistency-audit/spec.md) — All 5 integration-level ambiguities resolved. System confirmed structurally stable.

---

## Clarifications

### Session 2026-06-07

- Q: Where does concurrency budget enforcement belong? → A: Actor/Activation Guard — Scheduler defines policy; Actor enforces budget at activation guard before mailbox creation.
- Q: Who owns the activation guard under contention? → A: The Actor owns the guard; Scheduler is guard-agnostic (policy vs enforcement separation).
- Q: Does ExecutionBackend manage concurrency limits or only execute already-decided units? → A: Backend only executes already-decided units. All concurrency decisions are upstream.
- Q: Are RECOVERING entities subject to the concurrency budget? → A: RECOVERING is exempt from budget; commands queue at Actor mailbox during recovery.
- Q: Does zero-event command deduplication apply? → A: Zero-event commands are exempt from ExecutionKey deduplication; always re-execute (deterministic output).

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Deterministic Scheduling Across Entities (Priority: P1)

As a runtime implementer, I need a deterministic scheduling policy so that the same input stream always produces the same entity activation order, enabling replay verification and debugging.

**Why this priority**: Without deterministic scheduling, replay cannot be trusted to reproduce live behavior, breaking the event sourcing model.

**Independent Test**: Can be tested by feeding the same entity stream through two separate runtime instances and verifying identical per-entity processing order.

**Acceptance Scenarios**:

1. **Given** an input stream of commands targeting entities A, B, C in order, **When** the Scheduler processes them, **Then** the activation order is deterministic and reproducible on every run with the same input.
2. **Given** two runtime instances processing identical input streams, **When** comparing execution traces, **Then** the per-entity command processing order matches exactly.
3. **Given** a replay of a historical event stream, **When** the Scheduler makes activation decisions during replay, **Then** the scheduling order matches the live execution scheduling order.

---

### User Story 2 — Fairness Across Entities Under Load (Priority: P1)

As a runtime implementer, I need the scheduling policy to guarantee cross-entity fairness so that no single entity monopolizes execution slots under sustained load.

**Why this priority**: Without fairness guarantees, a high-frequency entity can starve all other entities, causing unbounded processing delays.

**Independent Test**: Can be tested by sending continuous commands to entity A while sending sporadic commands to entity B, verifying that B's commands are eventually processed within a bounded time window.

**Acceptance Scenarios**:

1. **Given** entity A receiving 1000 commands/second and entity B receiving 1 command, **When** the system processes under the concurrency budget, **Then** entity B's command is processed within a bounded window (not starved).
2. **Given** N active entities all competing for budget slots, **When** the system is under sustained load, **Then** every entity receives at least one execution slot within a configurable fairness window.
3. **Given** a concurrency budget of K slots and 2K active entities, **When** all entities have pending commands, **Then** no entity monopolizes a slot indefinitely — all entities progress.

---

### User Story 3 — Per-Entity FIFO Under Concurrency (Priority: P1)

As a runtime implementer, I need per-entity FIFO ordering to be strictly preserved regardless of cross-entity scheduling decisions, so that entity state transitions are deterministic.

**Why this priority**: Reordering within an entity would produce different event streams, fundamentally breaking event sourcing correctness.

**Independent Test**: Can be tested by sending commands C1, C2, C3 to the same entity and verifying the mailbox processes them in exact FIFO order, even when the entity is competing for budget slots with other entities.

**Acceptance Scenarios**:

1. **Given** commands C1, C2, C3 sent to entity E in order, **When** the Actor processes them, **Then** they are executed in C1 → C2 → C3 order regardless of other entity activity.
2. **Given** an entity whose mailbox contains 10 commands, **When** the entity is scheduled, **Then** all 10 commands are processed in FIFO order before the entity's slot yields.
3. **Given** concurrent senders delivering commands to the same entity, **When** the mailbox receives them, **Then** processing order equals mailbox arrival order.

---

### User Story 4 — Recovery Does Not Block Scheduling (Priority: P2)

As a runtime implementer, I need recovering entities to be excluded from active scheduling so that large recovery operations do not consume budget slots and prevent active entities from making progress.

**Why this priority**: If recovery counts toward the budget, a burst of reactivations could saturate all slots with recovering entities, creating a livelock where no entity can transition to ACTIVE.

**Independent Test**: Can be tested by triggering recovery for 100 entities while imposing a budget of 5 slots, verifying that active entities continue to process commands within the 5-slot budget.

**Acceptance Scenarios**:

1. **Given** a budget of 5 slots and 100 entities in RECOVERING state, **When** commands arrive for active entities, **Then** active entities obtain budget slots without waiting for recovery to complete.
2. **Given** an entity transitioning from RECOVERING to ACTIVE, **When** the transition completes, **Then** the entity becomes eligible for budget slots on its next command.
3. **Given** commands arriving during RECOVERING state, **When** the entity transitions to ACTIVE, **Then** all buffered mailbox commands are processed in FIFO order.

---

### User Story 5 — Backend Independence (Priority: P2)

As a runtime implementer, I need the scheduling policy to be fully independent of the ExecutionBackend implementation, so that swapping backends (Tokio → Yoke → WASM) does not change scheduling behavior.

**Why this priority**: The determinism guarantee requires identical scheduling output across all backends. Backend-dependent scheduling would break replay across different runtime configurations.

**Independent Test**: Can be tested by running the same entity stream through two different backends and verifying identical scheduling decisions.

**Acceptance Scenarios**:

1. **Given** a TokioBackend and a YokeBackend, **When** processing the same input stream, **Then** the Scheduler produces identical activation order decisions for both backends.
2. **Given** a backend that uses a different internal task model, **When** the Scheduler makes scheduling decisions, **Then** no backend-specific data influences the decision.
3. **Given** a backend swap at runtime configuration time, **When** the system processes commands, **Then** the scheduling order is unchanged.

---

### Edge Cases

- **Budget exhaustion during sustained load**: When the concurrency budget is fully saturated with ACTIVE entities processing commands, new activation requests block at the Actor's activation guard. The block is bounded — the guard is released when budget frees up. No timeout is implied; the system guarantees eventual progress.
- **All entities passivated during budget saturation**: If all active entities passivate simultaneously while budget slots are occupied, slots become available as passivation completes. New activation requests proceed as slots free up.
- **Single entity with back-to-back commands**: The Actor processes commands from its mailbox in a loop. If the mailbox has multiple commands, the entity's dedicated task processes all of them sequentially without yielding its budget slot between commands. Budget slots are per-task, not per-command.
- **Scheduler receives activation proposal for FAILED entity**: The Scheduler proposes activation; the Actor's spawning path detects FAILED state and initiates recovery (per canonical spec FR-027: on-demand). The Scheduler is not involved in failure handling.
- **Fairness window expires with no progress**: If all entities are starved for longer than the fairness window (implementation bug or pathological scenario), the Scheduler MUST escalate — it forces the longest-waiting entity to the front of the activation queue. This is a circuit breaker, not normal behavior.
- **Zero-event command during budget saturation**: Zero-event commands are exempt from deduplication and are always re-executed. They do not advance version, consume no budget slot, and return immediately. They are invisible to the scheduling layer.

---

## Requirements *(mandatory)*

### Functional Requirements

#### Scheduling Policy Core

- **FR-SP-001**: The Scheduler MUST produce a deterministic activation order for a given input stream. Same input → same scheduling order on every execution and replay.
- **FR-SP-002**: The Scheduler MUST NOT execute commands, mutate entity state, handle failures, or manage persistence. Scheduling is a pure policy layer.
- **FR-SP-003**: The Scheduler MUST NOT depend on the ExecutionBackend implementation. Scheduling decisions are backend-agnostic.
- **FR-SP-004**: The Scheduler MUST define a configurable fairness window. Every pending entity MUST receive at least one activation proposal within the fairness window when all entities have pending commands.
- **FR-SP-005**: The Scheduler MUST enforce a per-entity FIFO guarantee. Within a single entity, commands MUST be processed in mailbox arrival order. No reordering across commands within the same entity stream.

#### Concurrency Budget

- **FR-SP-006**: The Scheduler MUST define the concurrency budget policy (slot count, fairness rules). The Actor MUST enforce the concurrency budget at the activation guard, BEFORE mailbox creation. If the budget is saturated, activation blocks at the guard until a slot frees up.
- **FR-SP-007**: The concurrency budget MUST apply exclusively to ACTIVE state command processing. RECOVERING entities are exempt — they do not consume budget slots.
- **FR-SP-008**: A single entity's dedicated task, once granted a budget slot, MUST process all pending mailbox commands in FIFO order without yielding the slot between commands. Budget slots are per-task, not per-command.
- **FR-SP-009**: Idle entity tasks parked on their mailbox receiver MUST NOT consume console budget slots. The budget applies to active command processing only.
- **FR-SP-010**: The ExecutionBackend MUST NOT enforce the concurrency budget. The backend receives already-decided execution requests and provides backend-internal task parallelism only.

#### Activation & Recovery Interaction

- **FR-SP-011**: Commands arriving during RECOVERING state MUST be queued in the Actor's mailbox. They MUST be processed in FIFO order after the entity transitions to ACTIVE and a budget slot is available.
- **FR-SP-012**: The Scheduler's activation proposal for a PASSIVATED entity MUST trigger the Actor's activation path (guard → budget check → mailbox → spawn → recover). The Scheduler proposes; the Actor enforces.
- **FR-SP-013**: The Scheduler MUST NOT be aware of activation guards. Guard acquisition and single-flight spawn coordination are the Actor's exclusive responsibility.

#### Fairness & Anti-Starvation

- **FR-SP-014**: The Scheduler MUST apply round-robin or weighted fairness across active entities. No entity may monopolize execution slots indefinitely.
- **FR-SP-015**: The Scheduler MUST guarantee bounded starvation prevention. The maximum time any pending entity waits for activation MUST be configurable and enforced.
- **FR-SP-016**: The Scheduler MUST provide a circuit-breaker escalation: if the fairness window expires for any entity, that entity MUST be promoted to the front of the activation queue.

#### Determinism & Replay

- **FR-SP-017**: Replay scheduling MUST produce identical activation order to live scheduling for the same input stream. Scheduling decisions MUST NOT depend on wall-clock time, resource utilization, or runtime state.
- **FR-SP-018**: Scheduling decisions MUST NOT be influenced by failure states. FAILED entities re-enter the scheduling pipeline through the Actor (on-demand recovery), not through the Scheduler.
- **FR-SP-019**: Observable scheduling-relevant state MUST be fully reproducible from the committed event stream. Scheduling decisions that depend on entity state MUST derive that state from events deterministically.

#### Zero-Event Commands

- **FR-SP-020**: Zero-event commands (Strict Queries) MUST NOT be subject to ExecutionKey deduplication. They MUST always be re-executed. They MUST NOT consume concurrency budget slots and MUST NOT interact with the Scheduler.

---

### Key Entities

- **Scheduler**: The policy engine that defines execution ordering, fairness rules, and concurrency budget policy. Does NOT execute, own state, or handle failures. Proposes activation to the Actor.
- **Scheduling Policy**: The formal set of rules governing entity activation order, fairness constraints, and concurrency budget semantics. Defined by this specification.
- **Concurrency Budget**: A configurable limit on how many entity tasks may be actively processing commands simultaneously. Defined by the Scheduler as policy; enforced by the Actor at the activation guard.
- **Fairness Window**: A configurable time or operation count after which every pending entity is guaranteed at least one activation proposal. Enforces anti-starvation.
- **Activation Proposal**: The Scheduler's output — a signal to the Actor that a specific entity should be activated. The Actor may accept (spawn task) or defer (budget saturated).
- **Activation Guard**: Owned by the Actor, not the Scheduler. Serializes concurrent activation attempts per entity via single-flight pattern. Budget check occurs at guard level before mailbox creation.
- **Budget Slot**: A logical permit for one entity task to actively process commands. Slots are per-task, not per-command. An entity's task processes all mailbox commands without yielding its slot.
- **Circuit-Breaker Escalation**: A Scheduler mechanism that forces the longest-waiting entity to the front of the activation queue when the fairness window expires. Prevents scheduler bugs from causing indefinite starvation.

---

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-SP-001**: Two runtime instances processing identical input streams produce identical per-entity activation orders, verifiable by comparing execution traces.
- **SC-SP-002**: Under sustained load with K budget slots and 2K active entities, every entity receives at least one activation within the configured fairness window. No entity starves.
- **SC-SP-003**: Per-entity FIFO ordering is never violated. A test sending 10,000 commands to a single entity under concurrent scheduling verifies command output matches send order.
- **SC-SP-004**: RECOVERING entities do not consume budget slots. A test with budget=5 and 100 simultaneous recoveries verifies active entities continue processing within the 5-slot budget.
- **SC-SP-005**: Swapping the ExecutionBackend produces identical scheduling decisions. A test running the same input stream through Tokio and Yoke backends verifies identical activation traces.
- **SC-SP-006**: Replay scheduling matches live scheduling exactly. A test comparing live execution traces with recovery replay traces verifies identical per-entity command processing order.
- **SC-SP-007**: Zero-event commands never block on budget or deduplication. A test sending 1000 zero-event queries verifies all complete without scheduler interaction.
- **SC-SP-008**: The Scheduler has no code path that invokes command handlers, event appliers, or state mutations. Verifiable by code audit.

---

## Assumptions

- The Actor Per Entity model (canonical spec Section 1) is the architectural foundation. Scheduling policy operates within this model without modifying it.
- The Execution Authority (execution-authority sub-spec) assigns the Actor as the sole execution gate. The Scheduler does not share or override this authority.
- The ExecutionBackend (execution-backend sub-spec) is a pure execution mechanism. It receives pre-decided requests from the Actor and provides backend-internal task concurrency only.
- The ExecutionKey and deduplication model (execution-unit-identity sub-spec) operates at the Actor level. The Scheduler is identity-agnostic.
- The activation-ordering sub-spec defines the Actor's internal activation flow. The Scheduler triggers the activation path; the Actor owns the guard and enforces the budget.
- This specification does not change the command lifecycle (load → execute → persist → apply → snapshot → publish → respond) or the event sourcing model.
- The Scheduler is a single-process component. Cross-process scheduling (cluster sharding) is out of scope (CORE-007 boundary per canonical spec Section 11).
- Scheduling determinism depends on deterministic inputs. Non-deterministic factors (wall-clock time, resource utilization, random tie-breaking) MUST NOT influence scheduling decisions.
