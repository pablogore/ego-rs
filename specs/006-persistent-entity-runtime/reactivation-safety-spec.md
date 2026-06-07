# Reactivation Safety Specification

**Feature**: `006-persistent-entity-runtime` (Sub-specification)
**Parent Spec**: [spec.md](spec.md)
**Created**: 2026-06-07
**Status**: Draft

**Input**: Implementation safety specification for the PASSIVATED → RECOVERING reactivation path under concurrent command arrival.

## Scope

This specification defines the required runtime guarantees for handling concurrent commands arriving at a PASSIVATED entity during the same reactivation window. It is a formal sub-specification of CORE-006 that fills the residual implementation-level race condition in the reactivation path.

### In Scope

1. Reactivation coordination model (single-flight behavior)
2. Handling of concurrent commands during PASSIVATED state
3. Shared activation future semantics
4. Registry-based or equivalent coordination constraints
5. Failure behavior during partial or failed activation
6. Observable guarantees vs internal implementation freedom

### Out of Scope

- Modifying CORE-006 semantic model (this spec is additive, not overriding)
- Specific locking primitives or runtime constructs
- External infrastructure dependencies
- Performance optimization or benchmarking

---

## User Scenarios & Testing

### User Story 1 — Concurrent Commands Trigger Single Reactivation (P1)

A developer sends multiple commands to an entity that is currently PASSIVATED. All commands must be processed, but the entity should be reactivated exactly once regardless of how many commands arrive during the reactivation window.

**Why this priority**: This is the core safety invariant — without it, duplicate actors or split-state scenarios can occur under concurrent activation.

**Independent Test**: Can be tested by sending N concurrent commands to a PASSIVATED entity, then verifying that exactly N commands were processed and exactly one actor was created for the entity during the reactivation window.

**Acceptance Scenarios**:

1. **Given** an entity in PASSIVATED state, **When** 10 commands are sent concurrently, **Then** the entity reactivates exactly once and all 10 commands are processed sequentially through the single surviving mailbox.
2. **Given** a command sent during the PASSIVATED → RECOVERING transition window, **When** a second command arrives (while recovery is in-flight), **Then** the second command MUST NOT trigger a second reactivation attempt. It MUST be delivered to the in-flight mailbox and processed after recovery completes.
3. **Given** an entity in PASSIVATED state, **When** a reactivation attempt fails (I/O error during snapshot load), **Then** all concurrent commands receive a runtime error and the entity returns to PASSIVATED or FAILED state atomically — no command is silently dropped, no partial actor state is observable.

---

### User Story 2 — Activation Future Semantics (P1)

The implementation must ensure that all callers that trigger a reactivation for the same entity receive a consistent result — either the command is processed after successful recovery, or an error is returned if recovery fails.

**Why this priority**: Without shared activation futures, callers may independently observe different states of the same activation attempt, leading to inconsistent error handling or lost commands.

**Independent Test**: Can be tested by sending N concurrent commands to a PASSIVATED entity, causing a deliberate recovery failure, and verifying that all N callers receive a consistent error.

**Acceptance Scenarios**:

1. **Given** multiple concurrent activation triggers for the same PASSIVATED entity, **When** recovery succeeds, **Then** all triggers deliver their commands to the same mailbox and all callers receive a successful response.
2. **Given** multiple concurrent activation triggers for the same PASSIVATED entity, **When** recovery fails (snapshot corruption, I/O error), **Then** all triggers receive the same error — no caller sees success while another sees failure.

---

### Edge Cases

- **Reactivation during reactivation**: What happens if a command arrives while the entity is already in the PASSIVATED → RECOVERING transition? → The command MUST be redirected to the in-flight activation. No second activation begins.
- **Reactivation failure with queued commands**: What happens if recovery fails while commands are queued? → All queued commands receive an error. The entity transitions to FAILED (unrecoverable I/O error) or returns to PASSIVATED (recoverable) atomically.
- **Reactivation success after some timeouts**: What happens if the first caller times out waiting for recovery, but recovery eventually succeeds? → The command is processed. The caller receives the result if still listening, or the result is discarded if the oneshot was dropped. Other callers (still waiting) receive their results normally.
- **Entity created during reactivation**: What happens if a creation command arrives while an entity is being recovered? → This should not occur — creation commands target entities that are known to be PASSIVATED. If a creation command targets an already-existing PASSIVATED entity, the runtime MUST detect this and route the command to the in-flight activation or reject with EntityNotFound semantics depending on the entity registry state.

---

## Requirements

### Reactivation Coordination Model (FR-SF-001 through FR-SF-006)

- **FR-SF-001**: Reactivation MUST be single-flight per entity. At most one activation process may be in-flight for a given entity triple `(TenantId, EntityType, EntityId)` at any time.
- **FR-SF-002**: When a command arrives for a PASSIVATED entity, the runtime MUST atomically mark the entity as "pending-reactivation." Any concurrent command arriving after this atomic mark MUST NOT trigger a separate activation — it MUST be redirected to the in-flight activation.
- **FR-SF-003**: The atomic check-and-mark mechanism MUST be safe under concurrent access. All commands arriving during the same activation window MUST observe a consistent state (either "reactivation pending" or "reactivation complete").
- **FR-SF-004**: The specific coordination mechanism (registry-level Mutex, single-flight future, channel ownership) is implementation-defined. Only the outcome — exactly one activation per trigger window — is mandatory.
- **FR-SF-005**: After recovery completes (success or failure), the "pending-reactivation" state MUST be cleared atomically. The entity transitions to ACTIVE (on success) or FAILED/PASSIVATED (on failure).
- **FR-SF-006**: CAS loops (`AtomicUsize::compare_exchange` loops) are FORBIDDEN for coordination per constitution §5. Allowed primitives: `tokio::sync::Mutex`, `tokio::sync::RwLock`, channel-based ownership, or single-flight future patterns.

### Concurrent Command Handling (FR-CCH-001 through FR-CCH-005)

- **FR-CCH-001**: All commands arriving during the reactivation window MUST be delivered to the same mailbox. The mailbox MUST be created before or atomically with the "pending-reactivation" mark, ensuring no command can arrive without a mailbox to receive it.
- **FR-CCH-002**: Commands arriving while the entity is RECOVERING MUST be queued in the mailbox. They MUST be processed in FIFO order after the entity transitions to ACTIVE (FR-022).
- **FR-CCH-003**: If recovery fails, all queued commands MUST receive an error response. The runtime MUST NOT silently drop commands or leave them in an indeterminate state.
- **FR-CCH-004**: The mailbox capacity bound (FR-020) applies cumulatively during reactivation: commands queued during recovery count toward the capacity. If the mailbox fills during recovery, subsequent senders receive MailboxFull (as with any ACTIVE entity).
- **FR-CCH-005**: Reentrancy prohibition (FR-024) applies during and after reactivation. A command handler running after recovery MUST NOT be able to send a command to its own entity.

### Shared Activation Future Semantics (FR-SAF-001 through FR-SAF-004)

- **FR-SAF-001**: Multiple concurrent callers that all encounter a PASSIVATED entity for the same entity triple MUST be bound to the same activation flow. The implementation MUST ensure that exactly one activation future is created per activation window.
- **FR-SAF-002**: All callers bound to the same activation MUST observe a consistent outcome: either all see successful delivery and processing of their commands, or all see the same error (recovery failure).
- **FR-SAF-003**: The activation future MUST be resolved (either by success or failure) before any caller's command is delivered to the mailbox. Command delivery and future resolution happen atomically from the caller's perspective.
- **FR-SAF-004**: If a caller disconnects (oneshot receiver dropped) before recovery completes, other callers MUST NOT be affected. The activation continues normally; the disconnected caller's command is still processed (for consistency) but the result is discarded.

### Registry Coordination Constraints (FR-RC-001 through FR-RC-004)

- **FR-RC-001**: The passivation registry MUST support atomic state transitions. The transition from "entity is PASSIVATED and available for activation" to "entity has an in-flight activation" MUST be atomic and observable to all concurrent threads/tasks.
- **FR-RC-002**: The registry MUST NOT allow two concurrent tasks to both observe PASSIVATED and both spawn an actor for the same entity triple. This is the core invariant that the coordination mechanism enforces.
- **FR-RC-003**: The registry MAY store an activation token or future reference alongside the entity triple for the duration of the activation. This allows concurrent callers to subscribe to the in-flight activation rather than starting their own.
- **FR-RC-004**: After activation completes (entity is ACTIVE), the registry entry for the entity MUST be removed or updated to reflect the active state. Subsequent commands go directly to the actor's mailbox without registry interaction.

### Failure Behavior During Reactivation (FR-FB-001 through FR-FB-006)

- **FR-FB-001**: If recovery fails (snapshot I/O error, event store unavailable, deserialization error), the entity MUST transition to FAILED state. The activation is considered failed for ALL bound callers.
- **FR-FB-002**: Upon recovery failure, all commands queued in the mailbox during recovery MUST receive a runtime error response. The order of error delivery is implementation-defined.
- **FR-FB-003**: The "pending-reactivation" state MUST be cleared upon recovery failure. The entity registry entry MUST reflect the terminal state (FAILED).
- **FR-FB-004**: Recovery failure MUST NOT leave the entity in an inconsistent state. No partial state, no half-constructed actor, no orphaned mailbox. The entity returns to FAILED and the registry reflects the terminal state.
- **FR-FB-005**: After a recovery failure, the next command arrival MUST follow the same reactivation path as a freshly failed entity. There is no "lockout" period or additional guard beyond what the FAILED state lifecycle provides (FR-027: on-demand recovery).
- **FR-FB-006**: A handler that panics during recovery (e.g., in `apply_event`) MUST be treated as a runtime failure. The entity transitions to FAILED. The panic does NOT corrupt the registry or leave stale activation state.

### Observable Guarantees (FR-OG-001 through FR-OG-004)

- **FR-OG-001**: The reactivation process MUST be transparent to the caller. The caller sends a command via `EntityRef::send()` and receives a result — the caller MUST NOT be able to distinguish between a command delivered to an already-ACTIVE entity and one that triggered reactivation.
- **FR-OG-002**: Internal coordination details (registry locks, activation futures, pending-reactivation state) are implementation-private. They MUST NOT be visible in the public API, error types, or any observable behavior.
- **FR-OG-003**: Under concurrent activation triggers, the observable behavior MUST be indistinguishable from a sequential execution where: (a) the first command triggers reactivation, (b) recovery completes, (c) all commands are processed in FIFO order. The implementation MUST NOT expose the concurrent nature of the triggers.
- **FR-OG-004**: Stale sender handle detection (checking if the mpsc channel is closed) is purely internal. The caller MUST NOT be able to observe whether their command arrived before or after the mailbox was created.

---

## Key Entities

- **Activation Future**: A synchronization primitive that represents the in-flight reactivation of a single entity. Created atomically when the first command encounters a PASSIVATED entity. All subsequent concurrent callers await the same future instead of creating a new one. Resolved when recovery completes (success or failure).
- **Pending-Reactivation Guard**: An atomic mark in the passivation registry for a specific entity triple indicating that reactivation is in progress. Prevents concurrent tasks from starting duplicate activation attempts. Cleared atomically when recovery completes.
- **Passivation Registry**: The in-memory store of entity triples that have been passivated. Extended with activation coordination state (pending-reactivation mark, optional activation future reference) during the reactivation window.
- **Coordination Primitive**: The implementation-chosen mechanism for enforcing single-flight activation. Options: per-entity `tokio::sync::Mutex`, single-flight future, channel-based ownership. CONSTRAINED: CAS loops are forbidden per constitution §5.

---

## Success Criteria

### Measurable Outcomes

- **SC-001**: Under N concurrent commands to the same PASSIVATED entity, exactly one actor task is created. Verifiable by counting actor spawns in test instrumentation.
- **SC-002**: Under N concurrent commands to the same PASSIVATED entity that fails recovery, all N callers receive the same error. Verifiable by collecting all error responses and asserting they are identical.
- **SC-003**: The internal coordination mechanism (registry state, activation future, lock) is not observable through any public API. Verifiable by asserting that no public type contains synchronization primitives.
- **SC-004**: After reactivation completes successfully, processing N commands through an entity that was PASSIVATED is indistinguishable from processing N commands through an entity that was already ACTIVE. Verifiable by comparing execution traces under both paths.
- **SC-005**: No combination of concurrent commands, recovery failures, partial timeouts, or network delays can produce duplicate actors or orphaned mailboxes. Verifiable by fault injection testing.
- **SC-006**: All acceptance scenarios from User Story 1 and User Story 2 pass deterministically under concurrent execution. Verifiable by running each scenario 100 times with randomized timing.

---

## Assumptions

- The parent CORE-006 specification defines the full semantic model for passivation, recovery, mailbox, and lifecycle. This sub-specification only adds implementation safety guarantees for the concurrent activation edge case.
- The constitution's CAS prohibition (§5) is authoritative. All coordination mechanisms must use allowed primitives only.
- The existing `ego-infrastructure::InMemoryEventStore` and `InMemorySnapshotStore` are sufficient for testing all scenarios, including recovery failure modes (by injecting errors into the in-memory stores).
- `tokio::sync::Mutex` is the recommended default coordination primitive because it is async-safe, well-understood, and satisfies the constitution's allowed primitives list.
- Single-flight future patterns (e.g., `tokio::sync::watch` or `tokio::sync::oneshot` shared across tasks) are acceptable as an alternative to per-entity Mutex, provided they maintain the atomicity of the first-acquirer-wins semantics.
