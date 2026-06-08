# Final Consistency Lock: CORE-006 Persistent Entity Runtime

**Feature**: `006-persistent-entity-runtime/final-consistency-lock`
**Created**: 2026-06-07
**Status**: Complete

**Purpose**: Final system-wide consistency audit after full sub-specification decomposition and resolution of all known architectural gaps. This is the last validation step before considering the system implementation-ready.

**Prerequisites**:
- [Pre-Scheduling Consistency Audit](../scheduling-consistency-audit/spec.md) — all 5 integration-level issues resolved
- [Scheduling Policy Specification](../scheduling-policy/spec.md) — Gap #5 resolved
- All other sub-specs: execution-authority, execution-backend, execution-unit-identity, activation-ordering, canonical spec

---

## Clarifications

### Session 2026-06-07

- Q: What is the Actor's runtime implementation model? → A: Per-entity async task. `EntityActor::run()` is a `tokio::spawn`-ed async fn that loops on the mailbox receiver (`rx.recv().await`), processes commands sequentially, and completes on passivation. The task owns the state, lifecycle FSM, and mailbox receiver for its lifetime.
- Q: How does the Scheduler run? → A: Event-driven trigger system. The Scheduler is woken by events (Actor signals slot freed, command arrival notification, fairness circuit-breaker). On each trigger, it runs one scheduling decision cycle. No busy-polling. Uses tokio `Notify` or `watch` channels from Actor to Scheduler.
- Q: What is the ExecutionBackend trait shape? → A: Sync trait, Runtime wraps with async. `pub trait ExecutionBackend { fn execute(state, cmd, ctx) -> Result; }`. The backend method is synchronous (pure computation, no I/O). The Actor invokes it directly in its async loop or via `spawn_blocking` for isolation. This maximizes portability (WASM, no_std) and separates async Actor machinery from pure computation.
- Q: What is the mailbox's Rust implementation model? → A: Bounded MPSC queue. `tokio::sync::mpsc::channel(capacity)`. `Sender` (cloned for `EntityRef`), `Receiver` (owned by Actor task). `try_send` provides synchronous `MailboxFull` rejection. Channel close on Actor termination provides stale sender detection for passivation/reactivation.
- Q: How is recovery executed within the Actor? → A: Synchronous replay inside Actor. `EntityActor::recover(store, snapshot)` loads the snapshot, iterates events from the event store in order, and calls `apply_event` synchronously. Runs inside the Actor task during RECOVERING phase before the command processing loop. No scheduler coordination, no backend involvement. Recovery is the Actor's internal concern.

---

## A. System Status

### **STABLE — IMPLEMENTATION-READY WITH 3 MINOR SPEC MAINTENANCE ITEMS**

CORE-006 is a fully coherent deterministic runtime kernel. All cross-spec contradictions are resolved. All 6 gaps from the canonical spec's Section 10 are now addressed by dedicated sub-specifications. No blocking architectural issues remain.

---

## B. Consistency Findings

### Validated Dimensions

| Dimension | Status | Evidence |
|-----------|--------|----------|
| Execution Semantics | CLEAR | Single ExecutionUnit purity model, Actor as sole authority |
| State Model | CLEAR | Actor is sole state mutator; Event Store is SSOT |
| Scheduling Model | CLEAR (1 minor) | Deterministic policy; fairness ambiguity noted (Finding #1) |
| Backend Model | CLEAR | Full isolation; no semantic leakage into domain |
| Identity Model | CLEAR | ExecutionKey deterministic; deduplication rules consistent |
| Failure Model | CLEAR | Two-class classification; Actor-driven recovery |
| Replay Equivalence | CLEAR | Replay == live across all sub-specs |
| Activation Ordering | CLEAR | Single-flight, mailbox-before-recovery, budget-aware |

---

### Finding #1: Fairness Window Measurement Ambiguity

**Area**: Scheduling Model — Determinism
**Severity**: Low
**Affected sub-specs**: `scheduling-policy`

**Description**: FR-SP-015 states *"The maximum time any pending entity waits for activation MUST be configurable and enforced."* FR-SP-017 states *"Scheduling decisions MUST NOT depend on wall-clock time, resource utilization, or runtime state."* The Key Entities entry for Fairness Window says *"A configurable time or operation count"* but does not specify which mode applies when.

If fairness enforcement uses wall-clock time during live execution, and wall-clock is unavailable during replay, replay scheduling could produce different activation orders, violating FR-SP-017. The system needs to clarify that the fairness window is measured in deterministic units (operation count, command-processed count), not wall-clock time.

**Root cause**: The fairness window concept was defined for live execution latency guarantees. The determinism constraint was derived from replay requirements. The measurement unit was not reconciled between the two concerns.

**Resolution direction**: Amend FR-SP-015 to use *"maximum operation count (commands processed)"* instead of *"maximum time."* Retain the configurable window concept but specify that fairness is measured in deterministic units — the number of scheduling decisions processed since the entity was last activated. This preserves live/replay equivalence.

---

### Finding #2: Canonical Spec Section 10 Gap Table Needs Update

**Area**: Spec Maintenance
**Severity**: Low
**Affected sub-specs**: Canonical `spec.md` Section 10

**Description**: The canonical spec's Section 10 (Known Architecture Debt) documents all 6 gaps with *"Spec fix in progress"* or *"Partially addressed"* status. All 6 gaps have been resolved by sub-specifications:

| Gap | Status | Resolved By |
|-----|--------|-------------|
| #1 (Execution Authority Implicit) | **Resolved** | `execution-authority/` |
| #2 (Scheduler vs Replay Overlap) | **Resolved** | `execution-authority/` (FR-EA-009) |
| #3 (ExecutionUnit Identity) | **Resolved** | `execution-unit-identity/` |
| #4 (Backend Coupled to Semantics) | **Resolved** | `execution-backend/` |
| #5 (Scheduling Policy Undefined) | **Resolved** | `scheduling-policy/` |
| #6 (Failure Boundary Fuzzy) | **Resolved** | Canonical Section 9 + `execution-authority/` |

The gap table and *"Systemic Impact"* summary in Section 10 should be updated to reflect the resolved status, or converted to a resolution log.

**Root cause**: The canonical spec was written before all sub-specs existed. The gap table documented open work that is now complete.

---

### Finding #3: Scheduler Role Definition Incomplete in execution-authority

**Area**: Role Definition Consistency
**Severity**: Low
**Affected sub-specs**: `execution-authority`, `scheduling-policy`

**Description**: The execution-authority sub-spec's Role Definition table describes the Scheduler as owning *"Activation proposal, concurrency budget timing."* The scheduling-policy sub-spec adds responsibilities not reflected in the execution-authority table: fairness window definition (FR-SP-004), circuit-breaker escalation (FR-SP-016), and round-robin/weighted fairness (FR-SP-014).

This is an omission in the execution-authority table, not a contradiction. The scheduling-policy is the authoritative definition of Scheduler responsibilities. The execution-authority table should be updated to reference fairness enforcement as a Scheduler responsibility.

**Root cause**: The execution-authority spec was written when scheduling was an implicit concept. The scheduling-policy sub-spec later formalized and expanded the Scheduler's role. The execution-authority table was not updated to reflect the expanded definition.

---

## C. Architectural Summary

### Final Validated Runtime Model

```
┌─────────────────────────────────────────────────────────────────┐
│                     ENTITY RUNTIME BOUNDARY                      │
│                                                                  │
│  ┌──────────────┐    ┌───────────┐    ┌──────────┐    ┌───────┐ │
│  │   Scheduler  │───▶│   Actor   │───▶│  Exec    │───▶│ Exec  │ │
│  │   (Policy)   │    │ (Authority)│    │  Unit    │    │Backend│ │
│  └──────────────┘    └─────┬─────┘    └──────────┘    └───────┘ │
│        │                  │                                     │
│        │  Proposes        │  Owns:                              │
│        │  activation      │  - State & mailbox                  │
│        │  order           │  - Activation guard                 │
│        │                  │  - Concurrency budget enforcement   │
│        │  Defines:        │  - ExecutionKey & deduplication     │
│        │  - Fairness      │  - Lifecycle FSM                    │
│        │  - Budget policy │  - Replay gating                   │
│        │                  │                                     │
│        ▼                  ▼                                     │
│  ┌──────────────────────────────────────────────────┐          │
│  │                  Event Store (SSOT)               │          │
│  └──────────────────────────────────────────────────┘          │
└─────────────────────────────────────────────────────────────────┘
```

### Resolved Authority Hierarchy

| Layer | Component | Owns | Does NOT Own |
|-------|-----------|------|-------------|
| **Policy** | Scheduler | Activation order, fairness rules, budget policy, circuit-breaker | Execution, state, failures, backend specifics |
| **Authority** | Actor | Execution gating, state ownership, mailbox, guard, budget enforcement, deduplication, lifecycle | Activation proposals, backend execution, computation logic |
| **Compute** | ExecutionUnit | Pure handlers/appliers, deterministic (state → events → state) | Lifecycle, scheduling, identity, execution initiation |
| **Execute** | ExecutionBackend | Task execution mechanics, internal parallelism | State, ordering, fairness, budget, identity, semantics |

### Resolved Activation Flow

```
Command arrives
    │
    ▼ (Scheduler proposes)
Actor acquires activation guard
    │
    ▼
Budget check (Actor) ──[saturated]──▶ Block at guard
    │
    ▼ [slot available]
Create mailbox, register sender
    │
    ▼
Spawn task (RECOVERING — exempt from budget)
    │
    ▼
Recover (replay events, deterministic)
    │
    ▼
ACTIVE — process mailbox commands in FIFO (consumes budget slot)
    │
    ▼
PASSIVATING — drain mailbox, serialize, terminate
    │
    ▼
PASSIVATED — await next activation trigger
```

### All 6 Architectural Gaps — Resolved

| # | Gap | Resolution |
|---|-----|------------|
| 1 | Execution Authority Implicit | Actor IS Execution Authority per entity (execution-authority/) |
| 2 | Scheduler vs Replay Overlap | Single Actor gates both live and replay (execution-authority/) |
| 3 | ExecutionUnit Identity Underdefined | ExecutionKey = hash(entity_id, command, state_version) (execution-unit-identity/) |
| 4 | Backend Coupled to Semantics | Formal ExecutionBackend contract; determinism across backends (execution-backend/) |
| 5 | Scheduling Policy Undefined | Deterministic scheduling policy with fairness and budget (scheduling-policy/) |
| 6 | Failure Boundary Fuzzy | Two-class failure model; Actor-driven recovery (canonical Sections 9, 4) |

---

## D. Implementation Readiness Verdict

### **YES — READY FOR RUST IMPLEMENTATION**

CORE-006 is a fully coherent deterministic runtime kernel. All architectural contradictions are resolved. All sub-specs compose into a single consistent model. No blocking issues remain.

The 3 findings identified above are spec maintenance items (terminology, table updates, gap status) that do not affect architectural correctness or implementation viability. They can be addressed during or after implementation without structural impact.

### Implementation Approach

The system can be implemented as follows:

1. **Core types**: Entity trait (`handle_command`, `apply_event`), CommandContext, Event, EntityState
2. **Actor infrastructure**: Dedicated task per entity, bounded FIFO mailbox, activation guard, lifecycle FSM
3. **Scheduler**: Policy engine — activation queue, fairness tracking, circuit-breaker, budget policy definition
4. **ExecutionBackend trait**: Task execution abstraction — reference Tokio implementation
5. **Event store SPI**: Persistence interface per canonical Sections 6, 9
6. **Entity registry**: In-memory registry for active/passivated entity tracking

### Non-Blocking Spec Maintenance Items

- [ ] scheduling-policy: Clarify fairness window measurement unit (Finding #1)
- [ ] canonical spec Section 10: Update gap status to "Resolved" (Finding #2)
- [ ] execution-authority Role Definition: Add fairness enforcement to Scheduler column (Finding #3)

---

## Success Criteria

- **SC-LOCK-001**: No two sub-specs make conflicting claims about any component's responsibility. Verified across 6 sub-specs and the canonical spec.
- **SC-LOCK-002**: The execution flow is traceable from command arrival to event persistence through a single unambiguous path. Verified: Scheduler → Actor → ExecutionUnit → Backend → EventStore.
- **SC-LOCK-003**: Every architectural gap from the canonical spec Section 10 has a dedicated resolution sub-specification. Verified: 6/6 gaps resolved.
- **SC-LOCK-004**: Replay equivalence is guaranteed by at least one explicit cross-reference in every relevant sub-spec. Verified: execution-authority FR-EA-009, execution-backend SC-EB-004, execution-unit-identity SC-EI-006, scheduling-policy FR-SP-017.
- **SC-LOCK-005**: No sub-spec introduces a component that bypasses the Actor as Execution Authority. Verified: all execution paths gate through Actor.
- **SC-LOCK-006**: No sub-spec introduces non-deterministic behavior into the scheduling or execution model. Verified: Handler Safety Contract, ExecutionBackend determinism, scheduling determinism.

---

## Assumptions

- The Actor Per Entity model (canonical Section 1) is not modified by any sub-spec. All sub-specs extend or clarify existing concepts without altering the foundational model.
- The command lifecycle (load → execute → persist → apply → snapshot → publish → respond) is defined in the canonical spec and referenced but not duplicated in sub-specs.
- The EventPublisher SPI, PersistenceFacade SPI, and SnapshotStore SPI are defined in the canonical spec. Sub-specs reference these but do not redefine them.
- State ownership boundary is covered by the execution-authority sub-spec (state gate via Actor) and canonical spec FR-025 (thread-local state). No separate state-ownership-boundary sub-spec file exists; the concept is embedded in the authority model.
- This audit validates specification consistency, not implementation correctness. Implementation bugs are possible even in a consistent spec.
- All sub-specs remain valid after canonical spec amendments. If the canonical spec changes, all sub-specs must be re-validated.
