# Feature Specification: ExecutionBackend Abstraction Contract

**Feature Branch**: `006-persistent-entity-runtime`

**Created**: Sun Jun 07 2026

**Status**: Draft

**Parent Spec**: [../spec.md](../spec.md) (CORE-006 Canonical, Section 10: Known Architecture Debt — Gap #4)

**Input**: Fix the architectural gap where the execution model is implicitly coupled to Tokio semantics. Define a formal ExecutionBackend abstraction contract that decouples ExecutionUnit (pure computation) from the runtime implementation (Tokio / Yoke / WASM / custom).

---

## Clarifications

### Session 2026-06-07

- Q: What is the ExecutionBackend? → A: An abstraction contract that defines the interface between ExecutionUnit (pure computation) and the underlying runtime (Tokio, Yoke, WASM, custom). The ExecutionBackend owns task execution, not domain semantics.
- Q: Does the ExecutionBackend replace the Runtime Backend from the execution-authority spec? → A: Yes and refines it. The Runtime Backend was previously defined as "executes tasks only." The ExecutionBackend is the formal contract for that interface, defining what "executes tasks only" means: no semantic decisions, no state access, no ordering logic.
- Q: Does each entity have its own ExecutionBackend? → A: No. The ExecutionBackend is a process-wide abstraction. All entities within a runtime instance share the same backend. The backend is configured at EntityRuntime construction time.
- Q: Is the ExecutionBackend the same as the pluggable backend from Section 2? → A: Yes. Section 2 (ExecutionUnit Model) references "pluggable execution backends." This specification defines that pluggable contract formally.

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Decouple ExecutionUnit from Runtime Implementation (Priority: P1)

As a runtime implementer, I need a formal ExecutionBackend contract so that the ExecutionUnit and Actor are fully decoupled from the underlying runtime, enabling multiple backend implementations without changing domain logic.

**Why this priority**: Without this, the system is implicitly coupled to Tokio, preventing alternative runtimes (Yoke, WASM, distributed workers) and violating the portability guarantee.

**Independent Test**: Can be tested by implementing two backends (e.g., Tokio and a synchronous test backend), executing the same command through both, and verifying identical ExecutionUnit output.

**Acceptance Scenarios**:

1. **Given** a TokioBackend, **When** an ExecutionUnit is executed through the Actor, **Then** the command produces the same events and state as any other conforming backend.
2. **Given** a custom backend implementation, **When** the same (state, command, context) input is provided to the ExecutionUnit, **Then** the output (events, error, new state) is identical to the TokioBackend output.
3. **Given** an ExecutionBackend contract, **When** a backend is swapped at runtime configuration time, **Then** no domain code (PersistentEntity trait implementations) requires modification.

---

### User Story 2 — Backend Cannot Leak Into Domain Logic (Priority: P1)

As an application developer, I need the ExecutionBackend to be invisible to my domain code, so that my entity logic (command handlers, event appliers) contains no references to Tokio or any specific runtime.

**Why this priority**: If domain code depends on runtime specifics, swapping backends becomes impossible and determinism across environments is broken.

**Independent Test**: Can be tested by auditing the PersistentEntity trait signature and verifying no runtime-specific types appear in handler/applier parameter or return types.

**Acceptance Scenarios**:

1. **Given** a PersistentEntity trait implementation, **When** auditing the `handle_command` and `apply_event` signatures, **Then** no Tokio, Yoke, WASM, or any backend-specific types appear.
2. **Given** a backend swap from Tokio to a test backend, **When** all entity tests are re-run, **Then** all tests pass with identical results — no test requires modification for the backend change.
3. **Given** a command handler that produces events, **When** executed through two different backends, **Then** the events, their order, and their metadata are identical.

---

### User Story 3 — Deterministic Execution Across All Backends (Priority: P2)

As a runtime implementer, I need a guarantee that identical ExecutionUnit input produces identical output regardless of which backend executes it, so that replay and recovery are correct under any runtime configuration.

**Why this priority**: The event sourcing model requires deterministic replay. If different backends produce different results, recovery breaks.

**Independent Test**: Can be tested by executing the same command sequence through two backends and comparing the resulting event stream and entity states byte-for-byte.

**Acceptance Scenarios**:

1. **Given** an entity with 100 persisted events, **When** recovery is performed through a TokioBackend and through a YokeBackend, **Then** the reconstructed state is identical.
2. **Given** a command that produces events, **When** executed through any backend, **Then** the events, their ordering, and the resulting stream version are identical.
3. **Given** a backend implementation that introduces non-determinism (e.g., random timing), **When** the same input is executed multiple times, **Then** the output is identical — the backend's non-deterministic behavior does not propagate to the ExecutionUnit output.

---

### Edge Cases

- **Backend crash during ExecutionUnit execution**: The Actor (Execution Authority) detects the failure and transitions the entity to FAILED. The backend's crash does not corrupt entity state because the Actor owns state, not the backend. The Actor handles backend failures as non-deterministic runtime failures (Section 9 of canonical spec).
- **Backend produces different output than expected**: If a backend violates the determinism contract and produces different output from the same input, the Actor MUST reject the result and transition to FAILED. The backend is considered defective.
- **Backend does not support concurrency**: A single-threaded backend (e.g., WASM) must still respect the concurrency budget. The Scheduler and Actor are backend-agnostic — the backend executes what it is given; the Actor controls what is given.
- **Custom backend with different scheduling model**: A backend may implement its own internal scheduling, but MUST NOT override the Actor's FIFO ordering or execution decisions. The backend's scheduling is invisible to the Actor.

---

## Requirements *(mandatory)*

### Functional Requirements

- **FR-EB-001**: The system MUST define an ExecutionBackend trait (contract) that abstracts task execution. This contract is the sole interface between the Actor/ExecutionUnit layer and the underlying runtime.
- **FR-EB-002**: The ExecutionBackend contract MUST support execution of an ExecutionUnit with a given (state, command, context) input and return (events | error, new_state) output. The contract signature MUST be runtime-agnostic.
- **FR-EB-003**: The ExecutionBackend MUST NOT access Actor internal state directly. The backend receives inputs from the Actor and returns outputs to the Actor — it does not hold references to the Actor's state or lifecycle.
- **FR-EB-004**: The ExecutionBackend MUST NOT access Scheduler decision logic. The backend executes tasks it is given; it does not decide what to execute.
- **FR-EB-005**: The ExecutionBackend MUST NOT access Event Store internals. Event persistence is handled by the Actor through the PersistenceFacade, not by the backend.
- **FR-EB-006**: The ExecutionBackend MUST NOT introduce semantic differences in execution meaning. Identical ExecutionUnit input MUST produce identical execution output across all backend implementations.
- **FR-EB-007**: The ExecutionUnit MUST be backend-agnostic. No backend-specific types, traits, or behaviors may appear in handler/applier signatures.
- **FR-EB-008**: The Actor MUST remain the Execution Authority regardless of which backend is active. The authority chain (Section 3 of execution-authority sub-spec) does not change when the backend is swapped.
- **FR-EB-009**: The Scheduler MUST NOT depend on the backend implementation. The Scheduler makes activation proposals; which backend executes the resulting task is invisible to the Scheduler.
- **FR-EB-010**: The system MUST support at least one reference backend implementation (TokioBackend). Additional backends (YokeBackend, WASMBackend, CustomBackend) are optional but MUST conform to the same contract.
- **FR-EB-011**: The ExecutionBackend MUST be configured at EntityRuntime construction time. There is exactly one active backend per EntityRuntime instance. Backend swapping at runtime is not required.
- **FR-EB-012**: The ExecutionBackend MUST delegate state changes back through the Actor. The backend produces raw ExecutionUnit output; the Actor validates it, applies events to state, persists, snapshots, and publishes — the backend does none of these.

### ExecutionBackend Contract Definition

```
ExecutionUnit Execution:
  Input:  (state, command, context)
  Output: (events | error, new_state)
  Invariant: Same input → Same output (all backends)

Scheduling Integration:
  Input:  scheduling decision from Scheduler
  Output: activation signal to Actor
  Invariant: Backend accepts decisions, does not make them

Concurrency Model:
  The backend provides controlled concurrency execution.
  The backend does NOT define business-level ordering rules.
  Ordering is the Actor's responsibility (mailbox FIFO).

Isolation Guarantee:
  Backend MUST NOT access:
    - Actor internal state directly
    - Scheduler decision logic
    - Event Store internals
```

### Role Definition

| Role | Component | Owns | Does NOT Own |
|------|-----------|------|-------------|
| **Actor (Execution Authority)** | EntityActor task | State ownership, command ordering, event persistence, lifecycle, replay/live gating | Task execution mechanics, async runtime scheduling |
| **ExecutionUnit** | PersistentEntity trait | Pure computation: `(state, cmd) → events`, `(state, event) → state` | Execution initiation, runtime scheduling, state persistence |
| **ExecutionBackend** | Backend contract (Tokio/Yoke/WASM/custom) | Task execution mechanics, async runtime scheduling, concurrency primitives | Entity state, command ordering, execution correctness, Scheduler decisions, Event Store |
| **Scheduler** | Scheduling throttle | Activation proposals, concurrency budget | Task execution, entity state, command ordering |

### Hard Rules

1. The ExecutionUnit MUST be runtime-agnostic — no backend-specific code in handlers or appliers.
2. The Actor MUST remain the Execution Authority — the backend executes tasks, not commands.
3. The Scheduler MUST NOT depend on the backend implementation — scheduling is backend-agnostic.
4. The ExecutionBackend MUST NOT introduce semantic differences in execution meaning — determinism is backend-independent.
5. Identical ExecutionUnit input MUST produce identical output across all backends.

---

## Key Entities

- **ExecutionBackend Contract**: The abstraction interface defining task execution. Every backend implementation conforms to this single contract. The contract defines what it means to "execute tasks only" — the Runtime Backend role from the execution-authority spec.
- **TokioBackend**: The default reference implementation using the Tokio async runtime. Provides async task execution, semaphore-based concurrency control, and mpsc channel integration. All other backends must produce equivalent execution output.
- **YokeBackend**: An experimental deterministic scheduler backend. May trade concurrency for stronger determinism guarantees but must produce identical ExecutionUnit output.
- **WASMBackend**: A sandboxed execution backend targeting WebAssembly runtimes. ExecutionUnit runs in an isolated WASM module; output is returned to the Actor.
- **CustomBackend**: A user-defined backend implementing the ExecutionBackend contract. Must pass deterministic equivalence tests against the TokioBackend reference.
- **Backend Isolation Boundary**: The interface between the Actor/ExecutionUnit and the ExecutionBackend. The Actor provides (state, command, context) and receives (events | error, new_state). The backend is never given direct access to entity state, mailbox, registry, or event store.

---

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-EB-001**: The PersistentEntity trait contains no runtime-specific types (Tokio, Yoke, WASM, custom). Verifiable by automated audit: grep for backend-specific imports in domain code.
- **SC-EB-002**: Two backend implementations executing the same command through the same entity produce identical event streams, state, and version. Verifiable by running the full entity test suite against both backends and asserting identical results.
- **SC-EB-003**: Swapping the backend at EntityRuntime construction time requires zero changes to domain code (PersistentEntity implementations). Verifiable by changing the backend configuration and re-running all tests.
- **SC-EB-004**: Recovery replay produces identical entity state regardless of which backend performs the replay. Verifiable by persisting 100 events, then replaying through two different backends and comparing state.
- **SC-EB-005**: The ExecutionBackend contract is implementable without access to Actor internals, Scheduler logic, or Event Store. Verifiable by implementing a backend that has only the contract-defined inputs and outputs.
- **SC-EB-006**: Backend failure (crash, timeout, resource exhaustion) leaves entity state uncorrupted. Verifiable by injecting backend failures and verifying entity recovery produces correct state.

---

## Assumptions

- The Actor Per Entity model (canonical spec, Section 1) is the container. The ExecutionBackend is a component inside the container, not a replacement for it.
- The Execution Authority (defined in the execution-authority sub-spec) is the Actor. The ExecutionBackend does not override or share authority.
- The ExecutionUnit (canonical spec, Section 2) is pure computation. The backend is the mechanism that invokes the pure computation — it adds no semantics.
- The Scheduler (canonical spec, Section 3) makes activation proposals. The backend executes tasks on behalf of activated Actors.
- The primary backend implementation is TokioBackend. Additional backends are optional but must pass determinism equivalence tests.
- This specification does not change the command lifecycle (load → execute → persist → apply → snapshot → publish → respond). The backend participates only in the "execute" step.
- Backend configuration is static per EntityRuntime instance. Runtime backend hot-swap is not a requirement.
