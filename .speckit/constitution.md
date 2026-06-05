# EGO-RS Global Execution Constitution

Single source of truth for all system behavior, module execution, and feature development. Prevails over all other specifications, modules, and features.

---

## 1. System Overview

EGO-RS is a strictly layered execution engine, not an application logic framework. It provides deterministic primitives for building distributed, event-sourced, and replayable backend systems.

This constitution governs:
- Runtime execution behavior
- Architecture constraints and boundaries
- Testing governance and quality thresholds
- Speckit workflow discipline
- Deterministic execution principles

All components, features, and extensions MUST comply.

---

## 2. Execution Model

The system operates through a strict, immutable pipeline:

```
Scheduler -> Worker -> BatchExecutor -> Session -> Stores
```

No component may bypass, reorder, merge, or shortcut any stage.

### Decision -> Execution -> Commit

1. **Decision** -- Scheduler produces next-tag decisions only. No execution occurs.
2. **Execution** -- Worker delegates to BatchExecutor, which runs the handler pipeline via Session. No scheduling occurs.
3. **Commit** -- Session persists offset and dedup state atomically. No handler logic occurs.

Any component operating outside its designated phase is a violation.

### Unit of Work (UoW)

The **Unit of Work** is the atomic execution unit of the system. It represents one complete cycle for one tag: load events, execute handlers, commit offset + dedup.

```
UoW = { tag, event_range, handler_invocations, atomic_commit }
```

**Identity:** `(uow_id, tag, sequence_number)`
- `uow_id`: globally unique per execution attempt (UUID).
- `tag`: the `RuntimeSliceId` being processed.
- `sequence_number`: monotonically increasing from Scheduler, scoped per tag.

A new UoW is created per `TagDecision`. A failed UoW is NEVER retried — the next Scheduler decision for the same tag creates a new UoW with a new identity.

**Lifecycle:**

| Stage | What happens | Crash-recoverable? |
|---|---|---|
| CREATED | Scheduler produces `TagDecision` with `uow_id` and `sequence_number` | Ephemeral — lost on crash. |
| ASSIGNED | Worker accepts decision, binds to `stream_position` | Ephemeral — lost on crash. |
| EXECUTING | BatchExecutor fetches events, Session invokes handlers | Ephemeral — lost on crash. No dedup or offset written. |
| COMMITTING | Session persists offset + dedup atomically | Stores detect partial commit on recovery. Rolled back. |
| COMPLETED | Commit succeeded. Tag advances. | Durable. Next UoW starts from committed offset. |
| FAILED | Error at any stage. Tag unchanged. | Ephemeral. Next Scheduler decision produces new UoW. |

**UoW Rules (UoW-R1 through UoW-R5):**

| ID | Rule |
|---|---|
| UoW-R1 | A UoW MUST NOT be split. One TagDecision produces exactly one BatchCommand, one Session, one commit. |
| UoW-R2 | A UoW MUST NOT be retried. Failure is final. A new decision for the same tag creates a new UoW. |
| UoW-R3 | Handler invocations within a UoW MUST be sequential and single-threaded. |
| UoW-R4 | A UoW owns its tag exclusively during EXECUTING and COMMITTING. No second UoW for the same tag may be ASSIGNED until the first is COMPLETED or FAILED. |
| UoW-R5 | A UoW's event range `[offset_before, offset_after)` MUST be contiguous and non-overlapping with any other UoW for the same tag. |

---

## 3. Mode System

The system operates in exactly one mode at any time. Mode switching requires explicit instruction.

### ARCHITECTURE MODE

- Design only. No code changes.
- Produces specifications, plans, architecture decisions.

### IMPLEMENTATION MODE

- Code changes only. No architectural redesign.
- Produces source code, tests, evidence.

### REVIEW MODE

- Validation only. No changes.
- Produces review findings, pass/fail verdict.

Mixing modes is FORBIDDEN.

---

## 4. Component Responsibility Rules

Each component has exactly one responsibility. No component may span multiple execution layers.

### Scheduler

- Produces next-tag decisions only.
- Determines tag processing order.
- MUST NOT execute batches, run handlers, or manage concurrency.
- Crash: no recovery needed. Decisions are ephemeral.

### Worker

- Consumes exactly one tag decision per iteration.
- Delegates to BatchExecutor for pipeline execution.
- Validates incoming decisions (sequence, tag existence).
- MUST NOT schedule, iterate multiple tags, or manage concurrency.
- Crash: stateless. Restarts from last decision in channel.

### BatchExecutor

- Owns execution pipeline, concurrency control, and backpressure.
- Coordinates Session lifecycle.
- Fetches events, manages dedup set, produces Session input.
- MUST NOT schedule or iterate multiple tags.
- Crash: in-flight UoW is lost. Tag unchanged. Next decision creates new UoW from last committed offset.

#### Batch Semantics

A **Batch** is the event set of exactly one UoW. It has a one-to-one relationship with the UoW — no batch spans multiple UoWs, and one UoW produces exactly one batch.

| Batch stage | UoW stage | Contents |
|---|---|---|
| CREATED | ASSIGNED | tag, stream_position, batch_size |
| EXECUTING | EXECUTING | Fetched events (in stream order), dedup set |
| COMPLETED | COMPLETED | CommitPayload (offset + dedup + events + external_effects) |
| FAILED | FAILED | Error only. No commit payload. |

**Batch rules (B-R1 through B-R5):**

| ID | Rule |
|---|---|
| B-R1 | A batch MUST contain events for exactly one tag. (Refines UoW-R1) |
| B-R2 | Events within a batch MUST be in stream order. |
| B-R3 | Dedup is applied within the batch: duplicate keys are counted but not re-processed. |
| B-R4 | Batch size is bounded by `batch_size` from `BatchCommand`. BatchExecutor MUST NOT fetch beyond this bound. |
| B-R5 | A batch MUST NOT outlive its UoW. If the UoW fails, the batch is discarded. |

### Session

- Executes handlers and manages the commit boundary.
- Ensures atomic offset + dedup persistence.
- Returns commit outcome (COMPLETED or FAILED) to BatchExecutor.
- MUST NOT schedule, manage concurrency, or iterate batches.
- Crash before commit: tag unchanged. Next UoW re-processes from last committed offset.
- Handler panic during EXECUTING: UoW enters FAILED. No offset advancement. No dedup persistence.
- Stores MUST reject partial commits on recovery.

### Forbidden Overlaps (FO-R1 through FO-R3)

| ID | Rule |
|---|---|
| FO-R1 | No component may assume a responsibility assigned to another component. |
| FO-R2 | Concurrency ownership is exclusively held by BatchExecutor. |
| FO-R3 | A component MUST NOT import or call APIs from a component it does not directly interface with in the pipeline. |

---

## 5. Concurrency and Backpressure Rules

### Ownership

BatchExecutor is the sole owner of concurrency control.

### Allowed Primitives

- Semaphore-based concurrency primitives.
- Bounded channels with capacity limits.
- Structured concurrency (spawn + join patterns).

### Forbidden Patterns

- CAS loops (AtomicUsize, compare_exchange) anywhere in the system.
- Unbounded concurrency without semaphore limits.
- Worker or Scheduler managing concurrency.
- Manual reference counting for flow control.

### Backpressure

Backpressure MUST flow upstream through bounded channels and semaphore permits. Components MUST block or reject when capacity is exhausted. Unbounded queuing is FORBIDDEN.

---

## 6. State and Consistency Rules

### Execution State Categories

States for recovery after interruption. A UoW is COMMITTED iff both offset and dedup were persisted atomically. Any other state leaves the tag unchanged.

| State | Maps to UoW lifecycle | Meaning | Persisted? |
|---|---|---|---|
| NOT_STARTED | CREATED / ASSIGNED | Decision made but execution not yet begun | No (ephemeral) |
| IN_PROGRESS | EXECUTING | Session actively processing handlers | No (in-memory) |
| PARTIALLY_EXECUTED | EXECUTING (interrupted) | Some handlers completed, then failure struck | No (in-memory) |
| COMMIT_PENDING | COMMITTING | Atomic commit initiated but not confirmed | Yes (in-progress txn) |
| COMMITTED | COMPLETED | Commit confirmed. Offset + dedup durable. | Yes (persistent) |
| FAILED | FAILED | UoW terminated with error | No (ephemeral) |

### Recovery Rules

| Last known state | Recovery action |
|---|---|
| NOT_STARTED / IN_PROGRESS / PARTIALLY_EXECUTED | No recovery needed. Next UoW starts from last committed offset. |
| COMMIT_PENDING | Stores MUST detect and roll back partial commits on reconnect. Tag unchanged. |
| COMMITTED | Normal operation. Next UoW starts from committed offset. |
| FAILED | No recovery needed. Tag unchanged. |

**What is replayed:** Events in `[last_committed_offset, next_batch_end)`. Safe because:
- No dedup entries exist at or above `last_committed_offset` (failed UoW never committed).
- Handler execution is deterministic — same input, same output.
- Side effects are captured in the commit and applied atomically.

**What is skipped (dedup):** Events at or below `last_committed_offset`. BatchExecutor loads these; Session skips them.

**What is safe to re-execute:** Any uncommitted work. Determinism guarantees identical output on re-execution.

### Offset Management (OM-R1 through OM-R4)

| ID | Rule |
|---|---|
| OM-R1 | Offset MUST be loaded from persistent storage BEFORE fetching events. |
| OM-R2 | Offset MUST NOT be updated during handler execution. |
| OM-R3 | Offset MUST be persisted ONLY during the commit phase. |
| OM-R4 | On restart, the authoritative offset is the last COMMITTED offset. No offset may advance beyond what was committed. |

### Deduplication (DD-R1 through DD-R4)

| ID | Rule |
|---|---|
| DD-R1 | Dedup state MUST be checked BEFORE handler execution. |
| DD-R2 | Dedup MUST be persisted ONLY during the commit phase. |
| DD-R3 | Dedup MUST NOT be checked or persisted outside the commit boundary. |
| DD-R4 | Dedup entries are created only on COMPLETED. A FAILED UoW produces no dedup entries. |

### Atomic Commit (AC-R1 through AC-R3)

| ID | Rule |
|---|---|
| AC-R1 | Commit MUST be atomic: offset + dedup persisted together in one transaction. |
| AC-R2 | Partial commits (offset or dedup alone) are FORBIDDEN. |
| AC-R3 | Commit failure MUST roll back both offset and dedup. Stores MUST reject partial state on recovery. |

### Failure Semantics (FS-R1 through FS-R4)

| ID | Rule |
|---|---|
| FS-R1 | Ambiguous states produce rejection, never silent continuation. |
| FS-R2 | Partial failures are explicit errors, never retried. |
| FS-R3 | Unknown inputs, undefined transitions, and inconsistent states MUST terminate the current execution cycle immediately. |
| FS-R4 | Handler panic during EXECUTING terminates the UoW. Tag does not advance. Next UoW re-processes from the last committed offset. |

---

## 7. External Boundary

EGO-RS defines execution abstractions, not application frameworks. The public API MUST expose stable execution contracts. Implementations are replaceable.

The system is fully deterministic internally. External effects (HTTP calls, message publishing, external DB writes, API calls) are non-deterministic and must be isolated from the execution pipeline.

### Principle

External effects MUST NOT be executed during handler execution (EXECUTING) or commit (COMMITTING). They MUST be **described** as intents during EXECUTING and **dispatched** only after the atomic commit succeeds (COMPLETED).

**Core contract rule:** Public API types MUST NOT leak implementation-specific types (Tokio, Goakt, Actix, gRPC, Kafka clients, etc.). The boundary types (`IdempotencyKey`, `ExternalEffectDescription`, `Effect::ExternalEffects`) are domain-owned and runtime-neutral.

```
INSIDE (deterministic)                          OUTSIDE (non-deterministic)
======================                          ============================
Scheduler → Worker → BatchExecutor → Session
                                         ↓
                                    Stores (atomic commit)
                                         ↓
                              BatchExecutor → HTTP, Kafka, APIs
```

### Effect Models

| Name | Description |
|---|---|
| **Internal Effect** | State mutation, event emission, reply. Executed during EXECUTING by Session. Deterministic. |
| **External Effect** | HTTP call, message publish, external write. Described during EXECUTING as intent. Dispatched by BatchExecutor after COMMITTED. Non-deterministic outcome. |

### External Effect Rules (EE-R1 through EE-R6)

| ID | Rule |
|---|---|
| EE-R1 | Handlers MUST describe external effects as intents (`ExternalEffectDescription`). Direct calls to external systems from handlers are FORBIDDEN. |
| EE-R2 | External effect intents MUST be collected in the commit payload and persisted atomically with offset + dedup. No dispatch without commit. |
| EE-R3 | Every external effect MUST carry an `IdempotencyKey` derived from the UoW identity. The receiving system MUST use this key to reject duplicate dispatches. |
| EE-R4 | Dispatch MUST occur AFTER commit succeeds. Failed dispatches are retried with the same idempotency key. At-least-once delivery with exactly-once semantics via idempotency. |
| EE-R5 | External effect failure MUST NOT roll back the commit. The commit is immutable. Failed dispatches are retried asynchronously. The UoW is COMPLETED regardless of external effect status. |
| EE-R6 | Only BatchExecutor may dispatch external effects. Scheduler, Worker, and Session MUST NOT make external calls or manage dispatch state. |

### Failure scenarios

| Scenario | Why safe |
|---|---|
| Commit succeeds, dispatch fails | EE-R4: retry with idempotency key. External system deduplicates. |
| Dispatch succeeds, process crashes before ack | EE-R3: external system sees same idempotency key on retry, skips duplicate. |
| Handler calls external system directly | EE-R1: constitution violation. CI guard `detect-violations.sh` catches it. |
| Commit fails, external effects persisted | EE-R2: effects are part of the commit. If commit fails, effects are not persisted. Never dispatched. |

### Batch semantics extension

| Batch stage | UoW stage | Contents |
|---|---|---|
| CREATED | ASSIGNED | tag, stream_position, batch_size |
| EXECUTING | EXECUTING | Fetched events, dedup set |
| COMMITTED | COMPLETED | CommitPayload (offset + dedup + events + **external_effects**) |
| FAILED | FAILED | Error only. No commit payload. |

---

## 8. Testing Constitution

### TDD Required

All production code MUST be developed using Test-Driven Development.

Required workflow:
1. Write a failing test.
2. Verify the test fails.
3. Implement minimal code to pass.
4. Verify the test passes.
5. Refactor while keeping tests green.

Implementation without prior tests is a constitution violation.

### Minimum Coverage

All modified or newly created production code MUST maintain:
- Line Coverage >= 85%
- Branch Coverage >= 85%

Changes below these thresholds MUST NOT be considered complete. Exceptions require explicit written justification.

### Test Isolation

Unit tests MUST execute offline and deterministically. All external dependencies MUST be isolated through interfaces, traits, ports, or adapters.

FORBIDDEN:
- Real databases, Kafka clusters, NATS servers, Redis instances
- Real HTTP APIs, gRPC services, cloud services
- Real filesystem access for test data

ALLOWED:
- Mocks, stubs, fakes, and test doubles
- In-memory implementations of domain interfaces

### Deterministic Test Execution

Tests MUST produce identical results on every execution. FORBIDDEN:
- Time-based flakiness
- Network dependencies
- Environment-specific behavior
- Non-deterministic ordering

### Testability by Design

Production code MUST be designed for testability.

REQUIRED:
- Constructor injection
- Dependency inversion
- Interface-driven design

FORBIDDEN:
- Hidden singletons
- Global mutable state
- Hardcoded infrastructure dependencies

### Functional Programming

The codebase SHALL prefer functional programming techniques where practical.

Guidelines:
- Prefer pure functions and immutable data
- Minimize shared mutable state and side effects
- Isolate side effects at system boundaries
- Prefer composition over inheritance
- Prefer explicit inputs/outputs over hidden dependencies

### Deterministic Business Logic

Business logic MUST be deterministic.

FORBIDDEN inside domain logic:
- Hidden global state
- Current time lookups
- Random number generation
- Network access

Inject dependencies instead.

### Rustdoc Documentation

All public Rust APIs MUST include rustdoc documentation covering public structs, enums, traits, functions, and modules. Documentation MUST explain purpose and usage.

---

## 9. Speckit Workflow Governance

### Single Source Of Truth

There SHALL be exactly one active feature at any time, resolved only from `.speckit/state.yaml`. Files such as AGENTS.md, README.md, plan.md, and tasks.md are informational only.

### Workflow Stages

Speckit SHALL operate in this order:

```
/specify -> /clarify -> /plan -> /tasks -> /implement -> /review -> /archive
```

Skipping stages is FORBIDDEN. Commands MUST NOT regenerate previous artifacts unless explicitly requested.

### Task Format

Every task MUST contain:
- Exact file path
- Modification type: Create, Modify, Refactor, or Delete
- Target symbol (struct, trait, enum, function)
- Expected outcome
- Validation criteria

File-level instructions alone are insufficient. Tasks lacking these fields are invalid.

### Evidence Requirements

A task MUST NOT be marked complete without evidence.

Required format:
```
evidence:
  command: cargo test --workspace
  exit_code: 0
```

Claims such as "Implementation complete" without evidence are FORBIDDEN. Every task's completion status MUST be enumerated with evidence presence verified.

### Archive Gate

A feature SHALL NOT be archived unless:
- All tasks complete with evidence
- Coverage >= 85%
- cargo test, cargo clippy, and cargo fmt all pass

### Local Model Optimization

Speckit SHALL prefer deterministic instructions over reasoning-heavy prompts. Prefer "Modify file X", "Update symbol Y", "Run validation Z". Avoid "Analyze", "Review", "Think deeply", "Explore alternatives", "Generate coverage map" unless executing /clarify.

---

## 10. Hard Failure Conditions

The following constitute hard failures. Detection MUST halt execution immediately.

### Architecture Violations

- Worker executing scheduling logic, iterating multiple tags, or producing decisions. (References FO-R1, FO-R3)
- Scheduler executing batches, running handlers, or managing Session lifecycle. (References FO-R1)
- CAS loops or manual concurrency outside BatchExecutor. (References FO-R2)
- Backpressure bypass via unbounded queues.

### State Violations

- Offset updated before commit phase. (Violates OM-R2)
- Handler execution without prior dedup check. (Violates DD-R1)
- Non-atomic commit (offset or dedup alone). (Violates AC-R2)
- Partial commit state detected on recovery. (Violates AC-R3)
- Offset advanced without corresponding event processing. (Violates OM-R4)

### UoW Violations

- UoW split across multiple sessions. (Violates UoW-R1)
- UoW retried with same identity after failure. (Violates UoW-R2)
- Two concurrent UoWs for the same tag. (Violates UoW-R4)
- UoW event range overlaps with another UoW for the same tag. (Violates UoW-R5)
- UoW finalized without atomic commit. (Violates AC-R1)

### Mode Violations

- Code changes in ARCHITECTURE MODE.
- Architectural changes in IMPLEMENTATION MODE.
- Any changes in REVIEW MODE.
- Mixing modes within a single session.

### External Effect Violations

- Handler calling external system directly (not via `ExternalEffectDescription`). (Violates EE-R1)
- External effect dispatched before commit. (Violates EE-R2)
- External effect without idempotency key. (Violates EE-R3)
- Commit rolled back after external effect dispatched. (Violates EE-R5)
- Worker or Session dispatching external effects. (Violates EE-R6)

### Governance Violations

- Task completion claimed without evidence.
- Feature archived with incomplete tasks.
- Workflow stages skipped.
- Contract version not bumped on breaking change.

---

## 11. Core Invariant and Enforcement Model

### Core Invariant

**Strict separation: decision -> execution -> commit.** No operation may span more than one layer. No component may own more than one concern. This invariant is immutable and MAY NOT be violated.

### Enforcement Layer Hierarchy

Each constitution rule belongs to exactly ONE primary enforcement layer. The primary layer is authoritative for that rule. Lower layers may provide safety nets but must defer to the primary.

| Priority | Layer | Scope | Mechanism examples |
|---|---|---|---|
| 1 (highest) | **Compile-time** | Type system, sealed traits, visibility | ConcurrencyToken, type constructors, module visibility |
| 2 | **CI-time** | Static analysis, pipeline gates | detect-violations.sh, verify-layers.sh, check-contract-versions.sh |
| 3 | **Test-time** | Behavioral and state invariants | validate_replay_equivalence, contract tests, fault injection |
| 4 | **Runtime** | Dynamic state transitions, guards | PhaseGuard, AtomicityGuard, fail-closed mode |

### Rule-to-Layer Assignment

| Rule ID | Rule | Primary layer | Mechanism |
|---|---|---|---|---|
| FO-R2, UoW-R3 | Concurrency ownership, sequential execution | Compile-time | ConcurrencyToken type |
| FO-R3, B-R4 | Forbidden imports, layer boundaries | CI | detect-violations.sh |
| OM-R4, FS-R3 | Recovery correctness | Test | Fault injection + mock store tests |
| OM-R1-3, DD-R1-4 | Offset/dedup lifecycle | CI + Test | detect-violations.sh + integration tests |
| AC-R1-3 | Atomic commit | Runtime + Test | AtomicityGuard + fault injection |
| FS-R1-2, FS-R4 | Failure semantics | Runtime | PhaseGuard, fail-closed mode |
| UoW-R1-5 | UoW boundaries | CI + Test | detect-violations.sh + UoW contract tests |
| EE-R1 | No direct external calls from handlers | CI | detect-violations.sh |
| EE-R2 | Effects persisted atomically with commit | Compile-time + Test | Type system + contract tests |
| EE-R3 | Idempotency key required | Compile-time | Type-system (required field) |
| EE-R4-E6 | Dispatch ownership, retry safety, commit immutability | Runtime + Test | BatchExecutor post-commit dispatch + fault injection tests |

### Conflict Resolution

| Disagreement | Resolution |
|---|---|
| CI passes, runtime guard fires | Runtime is correct. CI missing coverage. Fix CI. |
| Runtime passes, test fails | Test is correct. Runtime guard insufficient. Fix guard. |
| Compile-time allows, CI rejects | CI is correct. Type-level enforcement too weak. Fix type design. |
| Multiple layers disagree | Constitution rules. Strictest outcome wins. No voting. |

### Drift Definitions

| Drift type | Definition | Detected by |
|---|---|---|
| **Architectural drift** | Code structure violates a constitutional boundary rule | CI guard |
| **Execution drift** | Actual execution trace mismatches intended execution flow | Trace validation |
| **Behavioral drift** | Same input produces different output across runs | Replay equivalence test |
| **Layer drift** | Two enforcement layers contradict each other on the same rule | Manual audit |
| **External boundary drift** | External effect dispatched from uncommitted state, or handler bypasses effect description | CI guard + runtime post-commit guard |

### System Correctness Definition

A system execution is correct if and only if:

1. Every UoW completed or failed atomically (no partial commits).
2. Every committed UoW has a contiguous, non-overlapping event range.
3. Every committed offset matches the number of processed events.
4. Every handler invocation was deterministic (same input, same output).
5. **Every committed UoW's external effects are dispatched at least once with idempotency keys.**
6. **No external effect is dispatched from an uncommitted UoW.**
7. The strictest enforcement layer outcome prevails when layers disagree.
8. No constitution rule was violated during the execution.

---

*This constitution is the single source of truth for EGO-RS system behavior. All specifications, modules, features, and agent instructions must comply. In case of conflict, this document prevails.*
