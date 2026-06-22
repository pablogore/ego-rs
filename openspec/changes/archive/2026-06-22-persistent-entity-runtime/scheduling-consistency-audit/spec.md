# Pre-Scheduling System Consistency Audit: CORE-006 Persistent Entity Runtime

**Feature**: `006-persistent-entity-runtime/scheduling-consistency-audit`
**Created**: 2026-06-07
**Status**: Draft

**Purpose**: Validate that CORE-006 Persistent Entity Runtime is internally consistent, free of semantic contradictions between sub-specifications, and structurally stable enough to introduce Scheduling Policy (Gap #5).

**Scope**: Cross-cutting interaction analysis between:
- `execution-authority` (Actor as Execution Authority + state gate)
- `execution-backend` (ExecutionBackend abstraction for Tokio/Yoke/WASM)
- `execution-unit-identity` (ExecutionKey + deduplication model)
- `activation-ordering` (lifecycle + activation semantics)
- Canonical CORE-006 runtime spec (merged system model)

**This is a CONSISTENCY VALIDATION phase, NOT a design phase.** NO new features are proposed. NO architecture is redesigned.

---

## Clarifications

### Session 2026-06-07

- Q: Where does concurrency budget enforcement belong? (Issue #1) → A: Actor/Activation Guard — The Scheduler defines scheduling policy (limit count, fairness rules); the Actor enforces the budget at the activation guard stage, BEFORE mailbox creation. This prevents orphan mailboxes by gating the full activation sequence (guard → budget check → mailbox → spawn → recover) behind the budget decision point.
- Q: Who owns the final scheduling decision under contention? (Issue #2) → A: The Actor owns the activation guard; the Scheduler is guard-agnostic. The Scheduler proposes activation order (policy); the Actor's spawning path acquires the guard and serializes concurrent activation attempts (enforcement). The Scheduler has no guard awareness and does not participate in guard acquisition or release.
- Q: Does the ExecutionBackend manage concurrency limits or only execute already-decided units? (Issue #3) → A: Backend only executes already-decided units. The backend receives pre-decided execution requests from the Actor. All concurrency decisions (budget, ordering, fairness) are made upstream by the Actor/Scheduler. The backend's "concurrency primitives" in the Role Definition table refer exclusively to backend-internal task execution parallelism, not Actor-level synchronization mechanisms.
- Q: Where are commands queued/blocked during RECOVERING state relative to the concurrency budget? (Issue #4) → A: RECOVERING is exempt from the concurrency budget. Recovery does not consume budget slots. Commands arriving during RECOVERING are queued in the Actor's mailbox (per activation-ordering FR-002/FR-003) and processed after ACTIVE transition. This prevents starvation: if recovery consumed budget slots, entities could never complete recovery under load.
- Q: Should zero-event commands be subject to ExecutionKey deduplication? (Issue #5) → A: No. Zero-event commands (Strict Queries) are exempt from ExecutionKey deduplication. They are always re-executed — deterministic output guarantees identical results on every execution. Deduplication adds cache management complexity for no correctness benefit. FR-EI-008 is amended with an explicit exemption for zero-event commands.

---

## Audit Summary

The system is **LARGELY CONSISTENT** but contains **5 issues** that must be resolved before Scheduling Policy (Gap #5) can be introduced. Two issues are Medium risk and require spec-level clarification. Three issues are Low risk and represent integration contract gaps or definitional ambiguity.

### Key Questions — Answered

| Question | Finding |
|----------|---------|
| Is Actor still the single execution authority across all sub-specs? | **YES** — FR-EA-001, FR-EA-009, FR-EB-008, FR-EI-004, and activation-ordering FR-001 all converge on Actor as sole authority. No conflicting claims exist. |
| Does ExecutionBackend remain fully stateless and isolated? | **YES** — FR-EB-003 through FR-EB-005 prohibit backend access to Actor state, Scheduler logic, and Event Store. Minor Role Definition ambiguity (Issue #3). |
| Is ExecutionUnit identity consistent with Actor-level deduplication? | **MOSTLY** — ExecutionKey model is consistent across specs. One contradiction in zero-event command handling (Issue #5). |
| Are activation ordering rules consistent with scheduling assumptions? | **PARTIALLY** — Activation ordering model is sound internally but does not account for concurrency budget delay points defined in the canonical spec (Issues #1, #2). |
| Is replay behavior guaranteed to match live execution semantics? | **YES** — FR-EA-009 gates both replay and live execution through same Actor. SC-EB-004 guarantees backend-independent replay. SC-EI-006 guarantees recomputable ExecutionKeys. No mismatch found. |

---

## Issues Found

### Issue #1: Concurrency Budget Insertion Point Undefined

**Category**: 🔵 Integration Contract Gap
**Risk Level**: Medium

**Description**:

The canonical spec (Section 1, Section 3) states that the concurrency budget may delay task activation: *"If the concurrency budget is saturated, the scheduler delays new task activation until capacity frees up."* The activation-ordering sub-spec defines the full activation sequence (acquire guard → create mailbox → register sender → spawn task → recover → process) but does **not** model where the concurrency budget check occurs in this flow.

If the mailbox is created and the sender is registered (per activation-ordering FR-003: *"mailbox... created and the sender registered... BEFORE the actor task begins recovery"*) but task spawning is delayed by the concurrency budget, this creates an **orphan mailbox state**: commands are enqueued in a mailbox with no consuming task. The canonical spec asserts that *"commands enqueued in existing mailboxes are processed normally (the entity is already active)"*, but for a newly activated entity whose task spawning is delayed, there IS no active task to consume the mailbox.

**Affected sub-specs**:
- Canonical spec: Sections 1 (Concurrency budget), 3 (Scheduler Model, Activation Flow step 4)
- `activation-ordering`: FR-003 (Mailbox Before Recovery), entire activation flow

**Why this emerges AFTER decomposition**: The activation-ordering sub-spec defines the internal activation flow atomically (guard → mailbox → spawn → recover). The canonical spec defines the budget as an external throttle on that flow. Neither defines the interaction point between them.

**Resolution** (2026-06-07): The concurrency budget check occurs at the Actor's activation guard, BEFORE mailbox creation. The Scheduler defines the policy (limit count, fairness rules); the Actor enforces it. If the budget is saturated, activation blocks at the guard until capacity frees up. No orphan mailbox state exists.

---

### Issue #2: Scheduler & Activation Guard Responsibility Ambiguity

**Category**: 🔵 Integration Contract Gap
**Risk Level**: Medium

**Description**:

The activation-ordering sub-spec defines an activation guard (per-entity synchronization token) that guarantees single-flight spawn: *"The activation lock MUST be acquired before mailbox creation and released after the sender is registered in the active registry, protecting the spawn-vs-redirect decision window"* (FR-004). The canonical spec Section 3 assigns the Scheduler responsibility for proposing activation: *"The Scheduler consumes activation triggers and decides which entity actor to activate."*

These two components have **overlapping authority** over the activation decision:
- Who acquires the activation guard? The Scheduler (as activation proposer), the activation trigger handler (command arrival path), or the Actor itself?
- If the Scheduler proposes activation but the activation guard is held by another entity's trigger handler, does the Scheduler's proposal wait, redirect, or degrade?
- The execution-authority spec says *"The Scheduler MUST NOT execute commands directly"* (FR-EA-005) but says nothing about guard acquisition.

**Affected sub-specs**:
- Canonical spec: Section 3 (Scheduler Model)
- `activation-ordering`: FR-004 (Activation Lock Lifecycle)
- `execution-authority`: FR-EA-005, FR-EA-010 (execution authority chain)

**Why this emerges AFTER decomposition**: The activation-ordering sub-spec formalized the guard as a standalone mechanism. The execution-authority sub-spec formalized the Scheduler as a standalone component. Neither defines their interaction boundary.

**Resolution** (2026-06-07): The Actor owns the activation guard. The Scheduler proposes activation order (policy layer) but is guard-agnostic — it has no awareness of per-entity guard state. The Actor's spawning path acquires the guard, checks the budget (per Issue #1 resolution), creates the mailbox, and spawns the task. The Scheduler's activation proposal is a signal to the Actor, not a guard acquisition.

---

### Issue #3: Concurrency Primitives Ownership Ambiguity

**Category**: 🟡 Ownership Ambiguity
**Risk Level**: Low

**Description**:

The execution-backend sub-spec Role Definition table assigns the backend ownership of *"concurrency primitives"*. The activation-ordering sub-spec defines mailbox as `tokio::sync::mpsc` and activation guard as `tokio::sync::Mutex` (key entities table). If the backend is responsible for concurrency primitives but the Actor uses Tokio-specific primitives directly, there is ambiguity about who provides synchronization mechanisms.

However, the execution-backend contract is intended for ExecutionUnit computation execution — the backend receives `(state, command, context) → (events \| error, new_state)` and returns results. The *"concurrency primitives"* entry in the Role Definition table is broader than the contract's actual scope.

**Affected sub-specs**:
- `execution-backend`: Role Definition table ("Concurrency Model" section)
- `activation-ordering`: Key Entities table (Mailbox, Activation Guard)

**Why this emerges AFTER decomposition**: The execution-backend Role Definition was written to comprehensively describe backend responsibilities but inadvertently claimed primitives that the Actor infrastructure already defines with Tokio-specific types.

**Resolution** (2026-06-07): The ExecutionBackend does NOT manage concurrency limits. It only executes pre-decided ExecutionUnit invocations from the Actor. The Role Definition table's "concurrency primitives" entry is scoped to backend-internal task parallelism only. The execution-backend sub-spec should refine its Role Definition to state: *"Backend provides controlled concurrency execution of ExecutionUnit computations. The backend does NOT provide mailbox channels, activation guards, concurrency budget enforcement, or entity lifecycle primitives."*

---

### Issue #4: Concurrency Budget Applicability During Recovery

**Category**: 🔵 Integration Contract Gap
**Risk Level**: Low

**Description**:

The canonical spec Section 1 states: *"Idle entity tasks parked on their mailbox receiver do not count toward the concurrency budget. The budget applies to active command processing, not task existence."* Activation Flow step 4 states: *"Only newly spawned entities may be delayed."*

But once a task IS spawned, it immediately begins recovery (RECOVERING state). During recovery, the task replays events — this is non-trivial work. Does the concurrency budget apply during recovery? The canonical spec says the budget applies to *"active command processing"*, which implies ACTIVE state command execution. But event replay during RECOVERING is also work that consumes system resources.

**Affected sub-specs**:
- Canonical spec: Sections 1 (Concurrency budget), 3 (Activation Flow), 4 (RECOVERING state)
- `activation-ordering`: FR-002 (Recovery Before Processing)

**Why this emerges AFTER decomposition**: Before decomposition, the scheduling model was implicit. After formalizing both the scheduler and activation flow, the recovery-phase budget question surfaces.

**Resolution** (2026-06-07): RECOVERING state is exempt from the concurrency budget. The budget applies exclusively to ACTIVE state command processing. Commands arriving during RECOVERING are queued in the Actor's mailbox (per activation-ordering) and processed after ACTIVE transition. New entity spawns may be delayed by the budget (per Issue #1 resolution), but once spawned, recovery proceeds without budget constraint.

---

### Issue #5: Zero-Event Command Deduplication Contradiction

**Category**: 🟠 Cross-Subsystem Consistency Conflict
**Risk Level**: Medium

**Description**:

The canonical spec FR-019 states: *"Commands that produce zero events (read-only queries) MUST NOT advance stream version."* The execution-unit-identity sub-spec defines ExecutionKey as `hash(entity_id, command, state_version)`. Since zero-event commands do not advance the version, re-sending the same query with the same state would produce an identical ExecutionKey.

The execution-unit-identity sub-spec contains contradictory guidance for this scenario:

- **FR-EI-008** (Requirement): *"A duplicate execution attempt where no state change has occurred (version unchanged) MUST be rejected or idempotently skipped."*
- **Edge Cases**: *"This is a caching concern, not a deduplication concern. The Actor may return the cached result or re-execute — both produce identical output per determinism guarantees."*

The contradiction: FR-EI-008 says **reject** or **idempotently skip** (implying do NOT re-execute). The Edge Case says **re-execute** is acceptable. While both preserve correctness (deterministic output), they represent different behaviors: one suppresses execution, one allows it.

**Affected sub-specs**:
- Canonical spec: Section 2/FR-019 (Strict Query Semantics)
- `execution-unit-identity`: FR-EI-008 (Deduplication), Edge Cases (zero-event command)

**Why this emerges AFTER decomposition**: During the consolidated canonical spec, zero-event commands were formalized as Strict Queries. When execution-unit-identity was later created, the ExecutionKey deduplication model did not account for the version-not-advancing property of zero-event commands, creating an internal asymmetry.

**Resolution** (2026-06-07): Zero-event commands (Strict Queries) are exempt from ExecutionKey deduplication. They are always re-executed — deterministic output guarantees identical results. FR-EI-008 must be amended in the execution-unit-identity sub-spec to add: *"Zero-event commands (Strict Queries per FR-019) are exempt from deduplication. They are always re-executed. The deduplication check does not apply when the execution produces zero events."* The Edge Case text allowing re-execution is preserved; the contradiction with FR-EI-008 is resolved by adding the exemption.

---

## Post-Resolution Stability Update

All 5 issues from the consistency audit have been resolved:
- **Issue #1**: Concurrency budget enforced at Actor activation guard, BEFORE mailbox creation
- **Issue #2**: Actor owns activation guard; Scheduler is guard-agnostic
- **Issue #3**: Backend only executes already-decided units; concurrency primitives are backend-internal
- **Issue #4**: RECOVERING exempt from budget; commands queue at Actor mailbox
- **Issue #5**: Zero-event commands exempt from deduplication; always re-execute

### Resolved Authority Chain

```
Scheduler (policy) → Actor (enforcement, guard, budget) → ExecutionUnit (compute) → ExecutionBackend (execute)
```

### Resolved Activation Flow (with Budget)

```
Command arrives → Activation guard acquired → Budget check (Actor) → Mailbox created
→ Sender registered → Task spawned → Recovery (exempt from budget) → ACTIVE
→ Process mailbox commands in FIFO (consumes budget slot)
```

The system is now fully ready for Scheduling Policy (Gap #5) definition without architectural drift.

---

## Determinism Verification

### No Conflicting Authority Models Found

All sub-specs defer to the Actor (EntityActor task) as the sole Execution Authority per entity:
- `execution-authority` FR-EA-001: Actor IS the Execution Authority
- `execution-backend` FR-EB-008: Actor remains authority regardless of backend
- `execution-unit-identity` FR-EI-004: Actor computes ExecutionKey
- `activation-ordering` FR-001: At most one actor task per entity triple
- Canonical spec Section 1: 1 entity = 1 dedicated task (single-writer guarantee)

No sub-spec proposes an alternative authority model.

### No Overlapping Execution Responsibilities Found

The Role Definition tables across sub-specs are compatible:

| Concern | Owned by | Spec source |
|---------|----------|-------------|
| Command ordering | Actor (Execution Authority) | execution-authority FR-EA-003 |
| Activation proposal | Scheduler | canonical Section 3, execution-authority FR-EA-005 |
| Pure computation | ExecutionUnit | execution-backend FR-EB-001 |
| Task execution mechanics | ExecutionBackend | execution-backend FR-EB-001 |
| Execution identity tracking | Actor | execution-unit-identity FR-EI-004 |
| Mailbox management | Actor | activation-ordering FR-003 |
| Concurrency budget | Scheduler | canonical Section 1 |

The only overlap is between Scheduler and activation-ordering's guard (Issue #2), which is a coordination gap, not a responsibility conflict.

### No Hidden Dependency Cycles Detected

Component dependency graph (all edges unidirectional):
```
Scheduler → Actor → ExecutionUnit → ExecutionBackend
                ↕ (guard coordination - Issue #2)
           activation-guard
```

The Actor → ExecutionUnit → ExecutionBackend chain is well-defined. The Scheduler ↔ activation-guard interaction is undefined but creates no cycle — it is a missing edge, not a bidirectional dependency.

### All Sub-Specs Compose Into a Single Deterministic Runtime Model

Given the same entity event stream, the composed runtime model guarantees deterministic output:
1. Actor (Execution Authority) ensures FIFO command processing per entity (FR-EA-003)
2. ExecutionUnit is deterministic pure computation (canonical Section 2)
3. ExecutionBackend is backend-agnostic and deterministic (FR-EB-006)
4. ExecutionKey is deterministically computable (FR-EI-005)
5. Activation ordering guarantees single-flight spawn with mailbox-before-recovery (FR-001, FR-003)

No sub-spec erodes or contradicts these guarantees. Issues #1, #2, #3, #4 are integration contract gaps that affect implementation clarity but not the theoretical determinism model. Issue #5 is a behavioral ambiguity that must be resolved but does not threaten determinism (both options preserve it).

---

## Stability Assessment for Scheduling Policy Introduction

### Pass: Structural Prerequisites

- [x] Single Execution Authority per entity — Actor owns all execution decisions
- [x] Mailbox ordering model — FIFO per entity, bounded, with explicit backpressure
- [x] ExecutionUnit identity model — ExecutionKey = hash(entity_id, command, state_version)
- [x] Backend isolation — ExecutionBackend decoupled from Actor semantics
- [x] Activation single-flight — exactly one task per entity triple per lifecycle window
- [x] State machine with well-defined transitions — RECOVERING → ACTIVE → PASSIVATING → PASSIVATED → FAILED

### Requires Resolution Before Scheduling Policy: Issues #1, #2

The scheduling policy will define how entities are ordered, prioritized, and dispatched. This directly interacts with:
- **Issue #1**: Where does the scheduling budget check occur in the activation flow?
- **Issue #2**: Who coordinates between scheduler activation proposals and activation guards?

Without resolving these, the scheduling policy's integration points are undefined.

### Can Be Resolved Concurrently With Scheduling Policy: Issues #3, #4, #5

These issues affect definitional clarity but do not block scheduling policy design:
- **Issue #3**: Concurrency primitives ownership — scheduling policy doesn't depend on who provides primitives
- **Issue #4**: Budget during recovery — scheduling policy defines activation dispatch, recovery timing is an implementation detail
- **Issue #5**: Zero-event deduplication — affects ExecutionUnit identity model, but scheduling policy operates at entity/dispatch granularity, not ExecutionKey granularity

---

## Validation Against Audit Criteria

### Pass Criteria

| Criterion | Status |
|-----------|--------|
| No conflicting authority models exist | ✅ PASS |
| No overlapping execution responsibilities exist | ✅ PASS (Issue #2 is coordination gap, not overlap) |
| All sub-specs compose into a single deterministic runtime model | ✅ PASS |
| No hidden cyclic dependencies exist between subsystems | ✅ PASS |

### Overall Verdict

**CORE-006 IS STRUCTURALLY STABLE** for Scheduling Policy (Gap #5) introduction, conditional on resolving Issues #1 and #2. The remaining three issues can be addressed concurrently with or after scheduling policy design.

---

## Success Criteria

- **SC-AUDIT-001**: No conflicting authority claims exist across sub-specs — every execution decision traces to exactly one Actor task.
- **SC-AUDIT-002**: All Role Definition tables across sub-specs agree on ownership boundaries without contradiction.
- **SC-AUDIT-003**: The composed runtime model guarantees deterministic state reconstruction for any given event stream, independent of backend implementation.
- **SC-AUDIT-004**: Every identified issue includes: affected sub-specs, emergence reason after decomposition, and a recommended resolution direction.
- **SC-AUDIT-005**: Issues are classified by risk level (Critical/High/Medium/Low) and by category (Determinism Violation Risk / Cross-Subsystem Consistency Conflict / Ownership Ambiguity / Integration Contract Gap).
- **SC-AUDIT-006**: No issue requires architectural redesign — all recommended resolutions are specification clarifications.

---

## Assumptions

- The Actor Per Entity model (canonical spec Section 1) is the architectural foundation. No sub-spec proposes replacing or circumventing it.
- The Scheduler is a runtime throttle, not a correctness mechanism. Its reordering capability for fairness does not affect per-entity determinism (mailbox FIFO).
- The ExecutionBackend is scoped to ExecutionUnit computation execution. Actor-level concurrency primitives (mailbox, activation guard) are not backend responsibilities.
- The activation-ordering sub-spec's Tokio-specific types represent the default Actor implementation; they do not force all backends to be Tokio-based.
- Issues identified are specification-level gaps, not implementation bugs. All sub-specs are internally self-consistent; gaps emerge at cross-spec integration points.
- The scheduling policy (Gap #5) will be defined in a separate sub-specification within `006-persistent-entity-runtime/`.
