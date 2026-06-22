# Feature Specification: ExecutionUnit Identity Model

**Feature Branch**: `006-persistent-entity-runtime`

**Created**: Sun Jun 07 2026

**Status**: Draft

**Parent Spec**: [../spec.md](../spec.md) (CORE-006 Canonical, Section 10: Known Architecture Debt — Gap #3)

**Input**: Fix the architectural gap where ExecutionUnit is defined as pure deterministic computation but its identity model is undefined. This creates ambiguity in execution deduplication, replay consistency, versioning of execution logic, and mapping between commands and ExecutionUnit instances.

---

## Clarifications

### Session 2026-06-07

- Q: What is the identity of an ExecutionUnit? → A: The ExecutionUnit itself is a pure functional definition (the PersistentEntity trait implementation). It has no intrinsic identity. Execution identity is an external concept: the ExecutionKey, computed as `hash(entity_id, command, state_version)`, identifies a specific execution occurrence. The ExecutionUnit does not know or care about this identity.
- Q: Who owns execution identity? → A: The Actor (Execution Authority). The Actor computes the ExecutionKey, tracks which executions have occurred, and ensures no duplicate execution within the same lifecycle window. The ExecutionUnit, Scheduler, and Backend are identity-agnostic.
- Q: Is the ExecutionKey used for deduplication? → A: Yes. The ExecutionKey allows the Actor to detect and prevent duplicate execution of the same (entity, command, version) combination. This is a correctness mechanism, not a caching hint.
- Q: Does the ExecutionUnit need to be versioned? → A: At the logical level, the ExecutionUnit is the PersistentEntity trait implementation, which is versioned at the code level (deployment). Execution identity does not need to encode the ExecutionUnit code version because determinism guarantees that the same code version produces the same output for the same input. If code changes, replay identically reproduces the new behavior — this is the event sourcing model working as designed.

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Identity-Agnostic ExecutionUnit (Priority: P1)

As a domain developer, I need the ExecutionUnit (PersistentEntity trait) to remain fully identity-agnostic, so that my command handlers and event appliers are pure functions with no knowledge of execution identity, deduplication, or instance tracking.

**Why this priority**: If the ExecutionUnit knows its own identity, it couples pure computation to runtime concerns, violating the determinism guarantees and preventing clean replay.

**Independent Test**: Can be tested by auditing the PersistentEntity trait signature and verifying that no identity-related types (ExecutionKey, instance ID, sequence number) appear in handler/applier parameters or return types.

**Acceptance Scenarios**:

1. **Given** a PersistentEntity trait implementation, **When** auditing the `handle_command` and `apply_event` signatures, **Then** no identity-related types (ExecutionKey, instance ID, deduplication state) appear.
2. **Given** two command invocations with identical (entity_id, command, state_version), **When** the ExecutionUnit produces output, **Then** the output is identical regardless of which invocation came first — the ExecutionUnit has no awareness of execution order.
3. **Given** a persistent entity with event state, **When** recovery replays the same events, **Then** the ExecutionUnit's `apply_event` produces the same state transitions without any identity or sequencing metadata.

---

### User Story 2 — Actor Computes Execution Identity (Priority: P1)

As a runtime implementer, I need the Actor to compute and track execution identity, so that deduplication and replay matching are centralized under the Execution Authority.

**Why this priority**: Without centralized identity tracking, duplicate execution detection is spread across components, creating race conditions and inconsistent behavior.

**Independent Test**: Can be tested by sending the same (command, version) twice to the same entity and verifying the Actor detects the duplicate execution attempt and rejects or skips it.

**Acceptance Scenarios**:

1. **Given** an entity at version V, **When** a command is executed at version V, **Then** the Actor computes an ExecutionKey and records the execution. If the same (entity, command, V) is received again, the Actor detects it as a duplicate.
2. **Given** two concurrent commands with the same (entity_id, command, state_version), **When** both arrive at the Actor's mailbox, **Then** exactly one produces an execution; the second is rejected as duplicate or idempotently skipped.
3. **Given** a command that completes successfully, **When** the caller retries with the same command and expected version, **Then** the version has advanced (the first execution committed events), so the retry has a different ExecutionKey — no false positive deduplication.

---

### User Story 3 — Deterministic ExecutionKey for Replay (Priority: P2)

As a runtime implementer, I need the ExecutionKey to be deterministically computable from (entity_id, command, state_version), so that replay can verify execution consistency.

**Why this priority**: If the ExecutionKey is non-deterministic, replay cannot be verified against historical execution records.

**Independent Test**: Can be tested by computing the ExecutionKey for a recorded execution, then recomputing it from the same inputs during replay, and verifying the keys match.

**Acceptance Scenarios**:

1. **Given** a command executed during live operation with ExecutionKey K, **When** the same (entity_id, command, state_version) is used to recompute the key during audit, **Then** the key is K.
 content-recomputing it during replay verification produces the same key.
2. **Given** two different commands with different payloads but same entity and version, **When** their ExecutionKeys are computed, **Then** the keys differ.
3. **Given** the same command sent at two different state versions, **When** their ExecutionKeys are computed, **Then** the keys differ (version is part of the key).

---

### Edge Cases

- **Retry with same expected version but command already executed**: The version has advanced (events committed). The retry arrives with an old expected version. The Actor computes a new ExecutionKey (different version → different key). The persist phase detects VersionConflict, not the deduplication layer. The original execution is not retroactively deduplicated — it already committed.
- **Zero-event command (Strict Query)**: The version does not advance (FR-019). If the same query is re-sent with the same (entity, command, version), the ExecutionKey is identical. The Actor may return the cached result or re-execute — both produce identical output per determinism guarantees. This is a caching concern, not a deduplication concern.
- **Command that produces different events after code change**: After a deployment changes the handler logic, the same (entity, command, version) may produce different events. The ExecutionKey is the same (inputs unchanged). This is correct — the ExecutionKey identifies the execution occurrence, not the output. The event sourcing model handles this: replay with the new code produces the new output for all past events, including this one.
- **ExecutionKey collision**: Hash collision between two different (entity, command, version) triples is theoretically possible but statistically irrelevant. If it occurs, the Actor's version check catches the mismatch during persist — the collision would produce the same key but the entity state versions would not match.

---

## Requirements *(mandatory)*

### Functional Requirements

- **FR-EI-001**: The ExecutionUnit (PersistentEntity trait) MUST NOT maintain execution identity internally. No identity-related types (ExecutionKey, instance ID, sequence number) may appear in handler or applier signatures.
- **FR-EI-002**: The ExecutionUnit MUST NOT store execution history. The event store records events; the ExecutionUnit does not record which executions occurred.
- **FR-EI-003**: The ExecutionUnit MUST NOT deduplicate executions. Deduplication is the Actor's responsibility.
- **FR-EI-004**: Execution identity MUST be computed externally by the Actor (Execution Authority). The ExecutionKey is: `hash(entity_id, command_payload, state_version)`.
- **FR-EI-005**: The ExecutionKey MUST be deterministically computable. The same (entity_id, command, state_version) MUST produce the same ExecutionKey on every computation.
- **FR-EI-006**: The Scheduler MUST NOT compute or use ExecutionKeys. The Scheduler proposes activation; it does not track execution identity.
- **FR-EI-007**: The ExecutionBackend MUST NOT compute or use ExecutionKeys. The backend executes tasks; it has no identity awareness.
- **FR-EI-008**: The Actor MUST detect duplicate execution attempts for the same ExecutionKey within the same lifecycle window. A duplicate execution attempt where no state change has occurred (version unchanged) MUST be rejected or idempotently skipped.
- **FR-EI-009**: The same command with a different state version MUST produce a different ExecutionKey. Version advancement prevents false duplicate detection on retries.
- **FR-EI-010**: The ExecutionKey MUST be usable for execution traceability. Every command execution MUST have a recorded ExecutionKey that can be recomputed from the stored command and entity state during audit.

### Role Separation

| Role | Component | Identity Concern |
|------|-----------|-----------------|
| **ExecutionUnit** | PersistentEntity trait | IDENTITY-AGNOSTIC: no awareness of execution identity |
| **Actor (Execution Authority)** | EntityActor task | IDENTITY OWNER: computes ExecutionKey, detects duplicates, records execution traceability |
| **Scheduler** | Scheduling throttle | IDENTITY-AGNOSTIC: proposes activation, does not track execution identity |
| **ExecutionBackend** | Backend contract | IDENTITY-AGNOSTIC: executes tasks blindly without identity awareness |

### Hard Rules

1. The ExecutionUnit MUST NOT maintain identity internally or store execution history.
2. Execution identity MUST be computed externally: `ExecutionKey = hash(entity_id, command, state_version)`.
3. The Scheduler MUST NOT define or use execution identity.
4. The ExecutionBackend MUST execute blindly — no identity awareness.
5. Same ExecutionKey → same deterministic output → no duplicate execution in same lifecycle window.

---

## Key Entities

- **ExecutionKey**: A deterministic hash computed from `(entity_id, command_payload, state_version)`. Identifies a specific execution occurrence. Used by the Actor for deduplication, replay matching, and execution traceability. Not visible to the ExecutionUnit.
- **ExecutionUnit Definition**: The PersistentEntity trait implementation — immutable logic, stateless, deterministic. The definition has no identity; it is a pure transformation function.
- **ExecutionUnit Instance**: The combination `(ExecutionUnit definition + entity_id + command + state_version)` that represents a single execution occurrence. Identified by the ExecutionKey.
- **Deduplication Window**: The Actor's lifecycle window (RECOVERING → ACTIVE → PASSIVATING → PASSIVATED) within which duplicate ExecutionKeys are detected and rejected. Once the entity passivates and reactivates, a new lifecycle window begins.
- **Execution Trace**: A record of `(ExecutionKey, timestamp, result)` stored by the Actor for each command execution. Enables audit and replay verification.

---

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-EI-001**: The PersistentEntity trait contains no identity-related types. Verifiable by automated audit: grep for identity types in handler/applier signatures.
- **SC-EI-002**: Two concurrent commands with the same (entity_id, command, state_version) produce exactly one execution; the second is detected as duplicate. Verifiable by concurrent execution test.
- **SC-EI-003**: The ExecutionKey for a recorded execution matches the ExecutionKey recomputed from the same inputs during audit. Verifiable by log replay and key recomputation.
- **SC-EI-004**: A command retry with the same payload but after version advancement produces a different ExecutionKey and proceeds normally (no false positive deduplication). Verifiable by retry-after-commit test.
- **SC-EI-005**: The ExecutionUnit produces identical output for two invocations with identical inputs, regardless of which invocation occurs first, proving identity-agnostic determinism. Verifiable by repeated execution test.
- **SC-EI-006**: Recovery replay produces the same entity state as live execution for the same event stream, with recomputable ExecutionKeys for every event application. Verifiable by live-vs-replay state comparison.

---

## Assumptions

- The ExecutionKey is computed as a content-based hash (e.g., of the serialized entity_id, command_payload, and state_version). The exact hash algorithm is implementation-defined; only determinism is mandatory.
- Deduplication is scoped to the Actor's lifecycle window. Once an entity passivates and reactivates, the deduplication state is reset. Persistent deduplication across reactivations is not required.
- The ExecutionKey is an internal runtime concept. It is not exposed in the public API (EntityRef, CommandResult, EntityError).
- The ExecutionKey is used for deduplication and traceability, not for command idempotency. Command idempotency is the application layer's responsibility (canonical spec Assumptions).
- This specification does not change the command lifecycle or the event sourcing model. It adds identity tracking to the Actor's existing responsibility set without introducing new execution paths.
