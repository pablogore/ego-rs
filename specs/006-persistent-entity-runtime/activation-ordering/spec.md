# Feature Specification: Activation Ordering Model for Persistent Entity Runtime

**Feature Branch**: `007-activation-ordering-model`

**Created**: 2026-06-07

**Status**: Draft

**Input**: User description: "CORE-006 Persistent Entity Runtime & SDK Clarify Prompt — Mailbox / Recovery / Actor Spawn Ordering — precisely define ordering guarantees and state visibility rules between mailbox creation, actor spawn, recovery process, and command arrival during transitional states"

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Define Activation Ordering Formal Model (Priority: P1)

As a runtime implementer, I need a formally-defined ordering model for the activation lifecycle so that concurrent command delivery is race-free and state visibility is deterministic.

**Why this priority**: Without this model, all downstream synchronization decisions are unspecified, leading to data races, double-activation bugs, and nondeterministic behavior under concurrency.

**Independent Test**: A test that launches two concurrent commands for a PASSIVATED entity and verifies exactly one actor task is created, all commands are processed in FIFO order, and the entity state is consistent.

**Acceptance Scenarios**:

1. **Given** an entity in PASSIVATED state, **When** two commands arrive concurrently, **Then** exactly one actor task is spawned, and both commands are processed sequentially in arrival order.
2. **Given** an entity in RECOVERING state, **When** a command arrives, **Then** the command is queued in the mailbox and processed only after recovery completes.
3. **Given** an active entity, **When** a command arrives, **Then** it is delivered directly to the existing mailbox without any activation synchronization.

---

### User Story 2 — Guarantee No Double Actor Spawn (Priority: P1)

As a runtime implementer, I need a single-flight activation mechanism so that under any concurrency level, at most one actor task exists per entity at any time.

**Why this priority**: Double spawns cause duplicate state, event stream corruption, and violate the single-writer principle of event sourcing.

**Independent Test**: A stress test that sends 100 concurrent commands to a passivated entity and asserts exactly one actor was spawned (count via registry active entries).

**Acceptance Scenarios**:

1. **Given** 100 concurrent commands target the same PASSIVATED entity, **When** activation is triggered, **Then** exactly one actor task spawns and all 100 commands are processed sequentially.
2. **Given** a spawn racing with registry insertion, **When** both attempt to create an actor, **Then** the activation mutex serializes them and the second caller either redirects to the existing mailbox or is rejected.

---

### User Story 3 — Deterministic Recovery Ordering (Priority: P2)

As a runtime implementer, I need to know the exact ordering of recovery events relative to command processing so that event replay semantics are deterministic.

**Why this priority**: If commands interleave with recovery events, the resulting state depends on race timing and replay is no longer deterministic.

**Independent Test**: A test that sends a command during recovery and asserts the command is processed only after all stored events are replayed, verified by comparing the resulting version number.

**Acceptance Scenarios**:

1. **Given** an entity with 100 stored events and an ongoing recovery, **When** a command arrives during recovery, **Then** the command is enqueued and processed only after all 100 events are applied.
2. **Given** a completed recovery, **When** commands are processed, **Then** the entity version after recovery matches the number of replayed events.

---

### Edge Cases

- What happens when the activation mutex guard is dropped (e.g., panic in spawn block)? The registry `remove_activation()` must always run (e.g., in a finally/drop handler) to prevent permanent lockout.
- How does the system handle mailbox overflow during recovery? Commands arriving during recovery are buffered in the bounded channel; if the channel is full, the sender backs pressure via `.await` on `send()`.
- What happens when the actor panics during recovery? The entity remains in FAILED state; the activation guard is released; the entity is removed from the active registry; subsequent commands trigger a fresh activation attempt.
- How does the system handle re-activation after passivation? `mark_passivated()` removes the entity from active registry; subsequent commands observe PASSIVATED and trigger a new activation cycle.
- What happens when two different entity types share the same aggregate ID? Entity identity is `(tenant_id, entity_type, entity_id)` triple; activation is scoped to this triple, so no collision occurs.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001 (Activation Single-Flight)**: The system MUST ensure that across all concurrent callers, at most one actor task is created per `(tenant_id, entity_type, aggregate_id)` triple.
- **FR-002 (Recovery Before Processing)**: An actor task MUST complete state recovery (snapshot load + event replay) before processing any commands from its mailbox.
- **FR-003 (Mailbox Before Recovery)**: The mailbox channel MUST be created and the sender registered in the active entity registry before the actor task begins recovery, so that commands arriving during recovery are queued without loss.
- **FR-004 (Activation Lock Lifecycle)**: The activation lock MUST be acquired before mailbox creation and released after the sender is registered in the active registry, protecting the spawn-vs-redirect decision window.
- **FR-005 (FIFO Ordering Within Entity)**: Commands sent to the same entity MUST be processed in FIFO order as received by the mailbox channel.
- **FR-006 (No Observable Partial State)**: A command MUST NOT observe a state that reflects only a subset of replayed events; recovery is all-or-nothing from the consumer perspective.
- **FR-007 (Panic Recovery)**: If the actor task panics during recovery or command processing, the activation guard MUST be released and the entity MUST be removed from the active registry.
- **FR-008 (Passivation Consistency)**: Before an entity transitions to PASSIVATED, the actor MUST process all already-enqueued commands and store a final snapshot.
- **FR-009 (Activation Retry)**: If a command arrives for an entity in FAILED state, the system MUST retry activation from scratch (new recovery attempt), not reuse a stale actor.
- **FR-010 (Marker Event Ordering)**: Replayed events from the event store MUST be applied to the state in ascending version order, matching the order they were originally persisted.

### Key Entities *(include if feature involves data)*

- **Actor Task**: A Tokio task executing `EntityActor::run()`. Owns the mailbox receiver, entity state, and lifecycle state machine. Exactly one per active entity.
- **Mailbox**: A bounded `tokio::sync::mpsc` channel (`Sender<CommandEnvelope>` / `Receiver<CommandEnvelope>`). Created at activation time, consumed by the actor task. Sender is registered in `EntityRegistry` for active routing.
- **Activation Guard**: A per-entity synchronization token (`Arc<tokio::sync::Mutex<()>>` stored in a `HashMap` keyed by `EntityTriple`). Guards the spawn-vs-redirect decision window.
- **EntityRegistry**: A shared concurrent map of active entities (sender lookup) and passivated entities (version tracking). Also houses activation guards.
- **Recovery Process**: The deterministic sequence of loading the latest snapshot from the snapshot store, deserializing it into state `S`, then replaying all events after the snapshot version from the event store in order.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Under any concurrency level (1–1000 concurrent commands to the same entity), exactly one actor task is created per activation cycle.
- **SC-002**: A command arriving during recovery is always processed after the recovered state is complete; no test can observe an intermediate recovery state.
- **SC-003**: Commands processed by an entity are output in FIFO order matching their send order, demonstrable via sequence-numbered commands.
- **SC-004**: After an actor panic during recovery, a subsequent command successfully triggers a fresh activation and recovery from the event store, producing correct state.
- **SC-005**: The model prevents message loss at all stages: commands sent to a PASSIVATED entity are eventually processed after activation; commands sent during recovery are processed after it completes.

## Assumptions

- The entity runtime operates within a single Tokio runtime instance (multi-process coordination is out of scope for this model).
- The activation mutex guard is per-process only; cross-process entity affinity requires external coordination (e.g., consistent hashing or lease-based locking).
- The mailbox channel capacity is configured sufficiently high to buffer commands arriving during worst-case recovery time; backpressure is acceptable and indicated by `send()` awaiting.
- The domain event store and snapshot store are external SPI implementations that are assumed to be consistent (linearizable reads within a single actor's recovery window).
- The `EntityTriple` (tenant_id, entity_type, aggregate_id) is a unique and stable entity identity; no two entities share the same triple.
- Network partitions or store unavailability during recovery cause the actor to fail and the command caller to receive an error; retry is handled externally.
