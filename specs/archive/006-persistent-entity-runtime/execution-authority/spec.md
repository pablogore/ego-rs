# Feature Specification: Execution Authority for Persistent Entity Runtime

**Feature Branch**: `006-persistent-entity-runtime`

**Created**: Sun Jun 07 2026

**Status**: Draft

**Parent Spec**: [../spec.md](../spec.md) (CORE-006 Canonical, Section 10: Known Architecture Debt — Gaps #1, #5)

**Input**: Fix the architectural gap where no single Execution Authority is formally defined for entity command execution. Currently, execution control is distributed across Actor, Scheduler, and Runtime Backend without explicit ownership boundaries, creating ambiguity in command execution ownership, ordering guarantees, concurrency control, and replay vs live execution routing.

---

## Clarifications

### Session 2026-06-07

- Q: What is the Execution Authority for a persistent entity? → A: The Actor (EntityActor task) IS the Execution Authority per entity. Each entity has exactly one Actor that serves as the single gatekeeper for all execution decisions. This is not a new component — it is a formal role assignment to the existing Actor.
- Q: Can the Scheduler execute commands directly? → A: No. The Scheduler proposes which entity should be activated, but MUST NEVER execute commands. It does not own state, does not guarantee ordering, and does not make correctness decisions.
- Q: Can the ExecutionUnit initiate execution? → A: No. The ExecutionUnit is pure deterministic computation. It is invoked by the Execution Authority; it does not initiate, schedule, or control execution.
- Q: Is the Execution Authority a new abstraction layer? → A: No. It is a formal role assignment to the existing Actor component. The Actor already performs this function implicitly in the canonical spec. This specification makes the role explicit, defines its boundaries, and prohibits cross-boundary violations.

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Unambiguous Execution Ownership (Priority: P1)

As a runtime implementer, I need a single, unambiguous authority per entity that decides whether a command can be executed, so that no two components can simultaneously claim execution authority over the same entity.

**Why this priority**: Without this, concurrent activation, scheduling, and replay can produce double-execution or ordering violations.

**Independent Test**: Can be tested by sending concurrent commands to the same entity and verifying that exactly one Actor makes the execution decision, that no Scheduler bypass occurs, and that the command processing order matches the mailbox FIFO order.

**Acceptance Scenarios**:

1. **Given** an entity in ACTIVE state, **When** a command arrives, **Then** the Actor (Execution Authority) makes the sole decision to execute and the command is processed through the Actor's mailbox in FIFO order.
2. **Given** concurrent commands arriving at the same entity, **When** the mailbox accepts them, **Then** the Actor processes them sequentially — no other component can bypass the Actor to execute commands concurrently.
3. **Given** an entity in PASSIVATED state, **When** a command triggers reactivation, **Then** the newly spawned Actor becomes the sole Execution Authority from the moment it enters RECOVERING state.

---

### User Story 2 — Scheduler Cannot Execute (Priority: P1)

As a runtime implementer, I need the Scheduler to be formally prohibited from executing commands, so that all execution paths are gated through the Actor's mailbox.

**Why this priority**: If the Scheduler can execute directly, it bypasses the single-writer guarantee, ordering guarantees, and concurrency control of the Actor model.

**Independent Test**: Can be tested by verifying that no code path exists where the Scheduler invokes an ExecutionUnit handler directly or bypasses the Actor's mailbox.

**Acceptance Scenarios**:

1. **Given** a Scheduler that has decided an entity should be activated, **When** the Scheduler issues an activation proposal, **Then** the Actor is spawned and the Scheduler's involvement ends — the Scheduler does NOT invoke the command handler.
2. **Given** a concurrency budget that delays entity activation, **When** the entity is eventually scheduled, **Then** the Scheduler's only action is to permit activation; the Actor processes commands through its own mailbox.
3. **Given** a Scheduler operating under sustained load, **When** multiple entities are pending activation, **Then** the Scheduler proposes execution order but each entity's Actor independently processes its own commands in FIFO order.

---

### User Story 3 — Replay and Live Execution Share Authority (Priority: P2)

As a runtime implementer, I need replay execution and live execution to be gated by the same Execution Authority, so that the execution model is consistent regardless of whether the entity is recovering or actively processing new commands.

**Why this priority**: Without this, replay could bypass the Actor, producing state that diverges from live execution because replay ordering and isolation differ from command processing ordering.

**Independent Test**: Can be tested by comparing entity state after 100 live commands vs. state after recovery replay of those same 100 events, verifying they are identical.

**Acceptance Scenarios**:

1. **Given** an entity recovering from PASSIVATED state, **When** the Actor replays events during RECOVERING, **Then** replay execution occurs within the Actor's single-threaded context — no external component processes events on the Actor's behalf.
2. **Given** an entity that has recovered and transitioned to ACTIVE, **When** commands are processed, **Then** the same Actor that performed replay now processes live commands through its mailbox — the execution authority is identical.
3. **Given** a recovered entity, **When** comparing the final state after replay with the state before passivation, **Then** the states are identical, proving that the Actor's execution authority produces consistent results in both replay and live modes.

---

### User Story 4 — ExecutionUnit Cannot Initiate Execution (Priority: P2)

As a runtime implementer, I need the ExecutionUnit to be formally prohibited from initiating its own execution, so that all execution is triggered by the Actor as Execution Authority.

**Why this priority**: If the ExecutionUnit can schedule or trigger its own execution, the single-writer guarantee is broken because the ExecutionUnit operates outside the Actor's mailbox ordering.

**Independent Test**: Can be tested by verifying that the `PersistentEntity` trait methods (`handle_command`, `apply_event`) are only invoked by the Actor's command processing loop, never directly by Scheduler, Runtime Backend, or any other component.

**Acceptance Scenarios**:

1. **Given** an ExecutionUnit (PersistentEntity trait implementation), **When** a command is processed, **Then** the `handle_command` method is invoked only by the Actor during its mailbox processing loop.
2. **Given** an entity recovery replay, **When** events are applied, **Then** the `apply_event` method is invoked only by the Actor during its recovery phase — no external component calls the applier directly.
3. **Given** the full execution pipeline, **When** tracing the call stack from command arrival to event persistence, **Then** every execution step is gated through the Actor (Execution Authority), never bypassing it.

---

### Edge Cases

- **Scheduler proposes activation during Actor shutdown**: If the Actor is PASSIVATING and the Scheduler proposes activation for the same entity, the Scheduler MUST NOT create a parallel Actor. The entity must complete passivation and be reactivated through the normal PASSIVATED → RECOVERING path.
- **Recovery and live command arrive simultaneously**: The Actor processes recovery first (RECOVERING state), then processes live commands from the mailbox in FIFO order. The Actor is the sole authority for both replay and live execution; no component may interleave recovery with live command execution.
- **Concurrency budget is exhausted**: The Scheduler delays activation but does NOT execute commands on the entity's behalf. Commands wait in the mailbox (if Actor is active) or in the activation queue (if Actor is not yet spawned).
- **Runtime Backend crash during execution**: The Actor (Execution Authority) transitions to FAILED. No other component may execute on the entity's behalf. Recovery is on-demand (admin action or restart), at which point a new Actor is spawned as the new Execution Authority.

---

## Requirements *(mandatory)*

### Functional Requirements

- **FR-EA-001**: Each entity MUST have exactly one Execution Authority at any time. The Actor (EntityActor task) is the Execution Authority for its entity.
- **FR-EA-002**: The Execution Authority MUST be the sole component that decides whether a command can be executed and in what order.
- **FR-EA-003**: The Execution Authority MUST enforce FIFO command ordering within its entity through the mailbox.
- **FR-EA-004**: The Execution Authority MUST prevent double execution under concurrency through the single-writer guarantee (one Actor task per entity triple).
- **FR-EA-005**: The Scheduler MUST NOT execute commands directly. The Scheduler proposes entity activation; it does NOT invoke command handlers, event appliers, or state transitions.
- **FR-EA-006**: The Scheduler MUST NOT own entity state, make correctness decisions, or guarantee command ordering. Ordering is the exclusive responsibility of the Execution Authority.
- **FR-EA-007**: The ExecutionUnit (PersistentEntity trait — handle_command, apply_event) MUST NOT initiate execution. It is pure computation invoked by the Execution Authority.
- **FR-EA-008**: The Runtime Backend MUST execute tasks only. It MUST NOT decide command ordering, correctness, or execution semantics.
- **FR-EA-009**: Both replay execution and live execution MUST be gated by the same Execution Authority. The Actor performs replay during RECOVERING and command processing during ACTIVE — no external component may process events or commands on the entity's behalf.
- **FR-EA-010**: The execution flow MUST follow the authority chain: Scheduler (proposes activation) → Execution Authority / Actor (authorizes and sequences execution) → ExecutionUnit (computes) → Runtime Backend (executes tasks). No component may bypass or shortcut this chain.
- **FR-EA-011**: When the Execution Authority is unavailable (entity in PASSIVATED or FAILED state), no execution may occur for that entity. Commands MUST be queued (RECOVERING) or rejected (PASSIVATING) or trigger reactivation (PASSIVATED).
- **FR-EA-012**: The Execution Authority's lifecycle transitions (RECOVERING → ACTIVE → PASSIVATING → PASSIVATED, ACTIVE → FAILED) MUST be the only mechanism by which execution capability is granted or revoked for an entity.

### Role Definition Table

| Role | Component | Owns | Does NOT Own |
|------|-----------|------|-------------|
| **Scheduler** | Scheduling throttle | Activation proposal, concurrency budget timing | Entity state, command ordering, execution correctness, replay decisions |
| **Execution Authority** | EntityActor task | Command execution authorization, FIFO ordering, concurrency control per entity, replay/live execution gating, lifecycle state machine | Activation proposal, backend execution, computation logic |
| **ExecutionUnit** | PersistentEntity trait (handle_command, apply_event) | Deterministic computation (state + command → events, state + event → new state) | Execution initiation, lifecycle, ordering, scheduling |
| **Runtime Backend** | Tokio runtime | Task execution, async scheduling | Command ordering, entity state, correctness decisions, execution semantics |

### Hard Rules

1. ONLY the Execution Authority (Actor) can trigger ExecutionUnit execution.
2. The Scheduler MUST NEVER execute commands or invoke handlers/appliers directly.
3. The ExecutionUnit MUST NEVER initiate execution, schedule itself, or control its own lifecycle.
4. The Runtime Backend MUST NOT decide ordering or correctness.
5. No component may bypass the Execution Authority to execute commands on an entity.

---

## Key Entities

- **Execution Authority**: The formal role assigned to the Actor (EntityActor task). Every entity has exactly one Execution Authority at any time. It is the single gatekeeper for all execution decisions for that entity.
- **Execution Authority Chain**: The mandatory execution flow: Scheduler → Execution Authority → ExecutionUnit → Runtime Backend. No shortcuts or bypasses are permitted.
- **Authority Boundary**: The interface between components in the execution authority chain. Each boundary has explicit responsibilities and prohibitions defined in the Role Definition Table.

---

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-EA-001**: No ambiguous execution ownership exists — every execution decision for an entity is traceable to exactly one Actor task.
- **SC-EA-002**: The Scheduler has no code path that invokes command handlers or event appliers directly. Verifiable by code audit: no Scheduler module calls `handle_command` or `apply_event`.
- **SC-EA-003**: Concurrent command delivery produces exactly one execution per command, with FIFO ordering enforced by the Actor's mailbox. Verifiable by stress testing: 100 concurrent commands to the same entity produce 100 sequential executions with no duplicates.
- **SC-EA-004**: Entity state after recovery replay is identical to state before passivation, proving that the Actor's execution authority produces consistent results in both replay and live modes.
- **SC-EA-005**: The ExecutionUnit trait methods (`handle_command`, `apply_event`) are only invoked through the Actor's command processing loop or recovery loop. Verifiable by call-graph audit.
- **SC-EA-006**: Under concurrency budget saturation, no entity has commands executed by a component other than its Actor. Verifiable by code review and integration test.

---

## Assumptions

- The Actor Per Entity model (defined in the parent spec, Section 1) already performs the Execution Authority role implicitly. This specification makes the role explicit and defines its boundaries.
- The Scheduler exists as described in the parent spec, Section 3: it proposes activation and enforces concurrency budget, but does not execute.
- The ExecutionUnit is defined in the parent spec, Section 2: it is pure computation with no lifecycle or execution control.
- The Runtime Backend (Tokio) executes tasks as instructed by the Actor but makes no correctness decisions.
- This specification does not introduce a new component — it assigns a formal role to the existing Actor and defines prohibited behaviors for existing components.
- Concurrent activation (single-flight reactivation) is covered by the activation-ordering sub-spec and the parent spec, Section 4. This specification does not change reactivation semantics.