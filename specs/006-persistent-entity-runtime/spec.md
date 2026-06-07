# Feature Specification: Persistent Entity Runtime and SDK

**Feature**: `006-persistent-entity-runtime`

**Created**: 2026-06-07

**Status**: Draft

**Input**: User description: "CORE-006 Persistent Entity Runtime & SDK — Event-sourced persistent entity abstraction inspired by Lagom Framework, with actor-per-entity execution, bounded FIFO mailbox, single-writer guarantees, snapshot support, optimistic concurrency, event publication, and Postgres as source of truth."

## Clarifications

### Session 2026-06-07

- Q: What is the execution unit model — 1 Tokio task per entity, pooled workers, or hybrid? → A: Dedicated Actor Task model — 1 Tokio task per active entity. Each ACTIVE or RECOVERING entity runs exactly one Tokio task that owns the mailbox and state. The task loops `receive → process → receive` and completes on passivation. Reactivation spawns a new task. Strictly single-threaded per entity (same coroutine exclusively owns state). Always resident when ACTIVE (task parked on mailbox receiver). Mailbox processing is permanently bound to the task. The "controlled task pool" is a scheduling limit on how many entities may be concurrently processing, not a fixed pool of shared workers.
- Q: What is the passivation atomicity model? → A: Irreversible Drain. Passivation is step-based (PASSIVATING exists as a real intermediate state). On transition to PASSIVATING, the mailbox is frozen atomically (sender handles removed from registry). Existing commands in the mailbox are drained by the task. New commands receive EntityPassivating error. Once the mailbox is empty and the current command completes, the task serializes state, registers as PASSIVATED, and terminates. Passivation cancellation does not exist — PASSIVATING → ACTIVE is forbidden. Reactivation always goes through PASSIVATED → RECOVERING → ACTIVE.
- Q: What is the failure determinism model? → A: Strict Event Sourcing. Two-class failure classification: deterministic business errors (handler returns error, applier returns error, version conflict) that are reproducible during replay, and non-deterministic runtime failures (storage I/O, snapshot I/O, publication I/O, handler panic) that are not replayed. Once an event is persisted, it is ALWAYS committed — never invalidated, rolled back, or skipped. The event stream is the single source of truth. Recovery replays ALL committed events deterministically, reproducing every prior state transition including those that returned business errors. FAILED→RECOVERING is on-demand (admin action or restart); there is no automatic retry loop. A persistent apply bug causes recovery to fail repeatedly until the code is fixed — this is by design, surfacing the bug immediately.
- Q: What is the versioning and snapshot consistency model? → A: Zero-Based, Last-Applied Version. Version starts at 0 (no events = version 0). Version is the count of committed events. Version is incremented atomically with event commit — the new event's sequence number. Snapshot at version V represents "state after applying events 1..V." Recovery: load snapshot V, replay events with seq > V. Zero-event commands do NOT advance version (FR-019). Version is strictly tied to the event store — never diverges. Snapshots are pure optimization — event stream is always the authoritative source of truth. Version gaps are FORBIDDEN: (seq N) → (seq N+1) is the only valid sequence.
- Q: What is the concurrency budget scheduling policy? → A: Best-effort FIFO with anti-starvation guarantee. Entries are processed in general arrival order, but the scheduler may reorder to prevent starvation. The runtime MUST guarantee that every pending entity eventually gets a slot under sustained load. The slot selection policy is implementation-defined with the constraint that no entity may be starved indefinitely. Weighted/priority-based fairness is NOT guaranteed by the runtime layer. This is purely a resource throttle — it does not affect execution semantics, correctness, or ordering guarantees of the actor-per-entity model.
- Q: What is the relationship between PASSIVATED reactivation semantics and internal implementation safety guards? → A: Strict separation — semantic guarantees are strict and observable; implementation guards are internal, optional in mechanism but mandatory in outcome. Reactivation is always transparent to the caller: the caller sends a command and receives the result; recovery and task lifecycle are invisible. The single-actor invariant is strict at the observable level: exactly one task processes entity commands at any given time, and no command is processed without an owning task. Reactivation is **single-flight per entity**: at most one activation process may be in-flight at any time. Transient duplicate spawn attempts MUST NOT occur — the runtime MUST coalesce concurrent activation triggers into a single reactivation. The guard mechanism (registry CAS, per-entity lock, single-flight pattern) is implementation-defined; only the outcome is mandatory. Stale sender detection is purely an implementation detail of the reactivation path, not part of the semantic model.
- Q: What is the PASSIVATED → RECOVERING reactivation model under simultaneous command arrival? → A: Single-flight strict reactivation. Exactly one reactivation process per entity at any time. All concurrent commands arriving while the entity is PASSIVATED or during early RECOVERING MUST coalesce into a single activation. Second activation attempts MUST be suppressed or redirected to the in-flight actor. This guarantees exactly-once actor creation per transition window. Multi-flight (concurrent attempts with guard resolution) is explicitly rejected — the simpler serialized model preserves the actor-per-entity philosophy without introducing concurrent spawn races into the implementation.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Define an Event-Sourced Entity (Priority: P1)

A developer defines a domain entity (e.g., BankAccount) by specifying what commands it accepts, what events represent state changes, how each command produces events, and how each event updates the entity state. The framework handles all persistence, recovery, and concurrency concerns.

**Why this priority**: This is the core value of the feature — without entity definition there is no persistent entity capability.

**Independent Test**: Can be tested by defining a simple counter entity with increment/decrement commands and events, sending commands, and verifying the final state reflects all applied events.

**Acceptance Scenarios**:

1. **Given** an entity with commands `Increment` and `Decrement`, **When** an `Increment` command is handled, **Then** a single `Incremented` event is produced.
2. **Given** an entity with initial state 0, **When** an `Incremented` event is applied, **Then** the resulting state is 1.
3. **Given** a command that violates a business rule (e.g., decrement below zero), **When** the command is handled, **Then** an error is returned and no event is produced.

---

### User Story 2 — Send Commands to an Entity (Priority: P1)

A developer sends commands to an entity by its identifier and receives the command result. The framework loads the entity state, executes the command, persists events, and returns the response — all in a single operation. The actor-per-entity model guarantees sequential command execution per entity.

**Why this priority**: This is the primary interaction pattern — without command delivery there is no way to modify entity state.

**Independent Test**: Can be tested by instantiating an entity reference for a known ID, sending a command, and verifying the response matches the expected outcome.

**Acceptance Scenarios**:

1. **Given** an entity with ID "account-123", **When** a valid command is sent to that ID, **Then** the command executes successfully and returns a response.
2. **Given** an entity that has never been created, **When** a non-creation command is sent to its ID, **Then** the command fails with EntityNotFound error.
3. **Given** an entity that has never been created, **When** a creation command (e.g., CreateAccount) is sent to its ID, **Then** the entity is initialized with initial state, the creation command is handled, and the entity is persisted.
4. **Given** two commands sent to the same entity ID, **When** both are executed, **Then** they execute sequentially via the actor mailbox and the second command sees the state produced by the first.
5. **Given** an entity whose mailbox is full, **When** a new command is sent, **Then** the sender receives a MailboxFull error immediately.

---

### User Story 3 — Entity Recovery After Restart (Priority: P2)

When the system restarts, entities recover their state automatically. The framework loads the most recent snapshot, replays any events that occurred after that snapshot, and reconstructs the full state before processing commands.

**Why this priority**: Recovery is essential for production operation — without it every restart would lose entity state.

**Independent Test**: Can be tested by processing commands to build entity state, simulating a restart (clearing in-memory state), then sending another command and verifying the entity correctly reflects the full prior history.

**Acceptance Scenarios**:

1. **Given** an entity with 100 events persisted, **When** the system restarts, **Then** the first command loads the entity with the correct state from persisted events.
2. **Given** a snapshot at version 50 and events 51-100, **When** the system restarts, **Then** the entity is restored from the snapshot plus the 50 replayed events.
3. **Given** no snapshot exists for an entity with 100 events, **When** the system restarts, **Then** all 100 events are replayed to reconstruct state.

---

### User Story 4 — Configure Snapshot Strategy (Priority: P2)

A developer configures when snapshots are taken to optimize recovery performance. Options include never taking snapshots, taking a snapshot every N events, or providing a custom strategy.

**Why this priority**: Snapshots are a performance optimization — without them, entities with long event streams become costly to recover.

**Independent Test**: Can be tested by configuring a snapshot-every-N strategy, processing N+1 events, and verifying a snapshot was stored at version N.

**Acceptance Scenarios**:

1. **Given** a snapshot strategy of "every 10 events", **When** 10 events are persisted, **Then** a snapshot is stored at version 10.
2. **Given** a snapshot strategy of "never", **When** any number of events are persisted, **Then** no snapshot is ever stored.
3. **Given** a custom strategy that takes snapshots on even versions, **When** events at versions 2 and 4 are persisted, **Then** snapshots are stored at those versions.

---

### User Story 5 — Multi-Tenant Entity Isolation (Priority: P2)

Entities belonging to different tenants are fully isolated. The same entity ID in two different tenants represents two independent entities with independent state, event streams, and snapshots.

**Why this priority**: Multi-tenancy is an architectural requirement — without it, data from different customers would mix.

**Independent Test**: Can be tested by creating the same entity ID in two tenants, sending different commands to each, and verifying each returns the expected isolated state.

**Acceptance Scenarios**:

1. **Given** entity "acc-1" in tenant A with state 10, **When** a command is sent to "acc-1" in tenant B, **Then** tenant B's entity starts from initial state, not tenant A's state.
2. **Given** an entity operation in tenant A, **When** a command is sent to a different entity in tenant B, **Then** both operations proceed without blocking each other.
3. **Given** no tenant is specified (single-tenant mode), **When** commands are sent, **Then** all entities operate in a default scope without tenant isolation.

---

### User Story 6 — Event Publication for Downstream Consumers (Priority: P3)

After events are successfully persisted, they are published via the EventPublisher SPI so downstream consumers (read-side projections, other services) can react to them. Events are never published before persistence is confirmed.

**Why this priority**: Event publication enables the reactive architecture — but the core entity runtime works without it, making it lower priority.

**Independent Test**: Can be tested by sending a command that produces events, verifying the events are persisted before they are published, and that a registered consumer receives the published events.

**Acceptance Scenarios**:

1. **Given** a command that produces two events, **When** the command completes successfully, **Then** both events are published after persistence confirms.
2. **Given** a command that fails during persistence, **When** an error is returned, **Then** no events are published.
3. **Given** a read-side projection registered for the entity's event type, **When** events are published, **Then** the projection receives them and updates its read model.

---

### User Story 7 — Handle Concurrent Modification Conflicts (Priority: P3)

When two concurrent writers attempt to modify the same entity, the second writer receives a version conflict error. The developer can retry the command with the updated state if needed. The actor-per-entity mailbox prevents direct concurrent command execution, but version conflicts can still arise from distributed or retried command streams.

**Why this priority**: Optimistic concurrency is important for correctness but most applications will not experience frequent conflicts given the sequential mailbox model.

**Independent Test**: Can be tested by opening two concurrent command streams to the same entity, sending commands on both, and verifying exactly one succeeds while the other receives a conflict.

**Acceptance Scenarios**:

1. **Given** two concurrent commands sent to the same entity, **When** both attempt to persist at the same expected version, **Then** exactly one succeeds and the other receives a version conflict error.
2. **Given** a version conflict error, **When** the sender retries with the current version, **Then** the retried command may succeed if no further conflict occurs.

---

### Edge Cases

- **Restart during command execution**: What happens when the system crashes between persisting events and returning the response? The events are already persisted and committed — they survive the restart. On recovery, the entity replays these events and reconstructs the state. The caller receives an error (timeout) and may retry; the retry either proceeds (if version check passes) or receives a VersionConflict (if another command advanced the version). Events are NEVER rolled back on restart.
- **Non-existent entity**: What happens when a non-creation command is sent to a non-existent entity? The command fails with EntityNotFound error. Only an explicit creation command can bring an entity into existence.
- **Snapshot corruption**: What happens when a stored snapshot is corrupted or unreadable? The framework falls back to full event replay from the beginning.
- **Replay with empty event stream**: What happens when an entity has a snapshot but zero events since that snapshot? The entity is restored from the snapshot directly with no replay.
- **Tenant ID edge cases**: What happens when tenant IDs contain special characters or exceed length limits? The underlying storage must handle sanitization; the framework passes the identifier through without transformation.
- **Command that produces zero events**: What happens when a command handler decides no events should be produced (read-only query)? The entity state is returned immediately with the handler result. The stream version MUST NOT advance, no events are persisted, no snapshots are created, and no publications are triggered.
- **Concurrent commands to different entities**: What happens when commands target different entities concurrently? They execute in parallel with no blocking — the mailbox model applies per entity only.
- **Replay safety**: What happens if the framework attempts to execute side effects during recovery replay? Side effects must be suppressed during replay — only state reconstruction is permitted.
- **Mailbox full during backpressure**: What happens when a burst of commands saturates an entity's mailbox? The sender receives MailboxFull error and must retry. The entity remains ACTIVE and processes queued commands from the mailbox.
- **Reentrancy attempt**: What happens when a command handler sends a command to its own entity? The runtime detects the reentrancy attempt and returns ReentrancyNotAllowed error.
- **Command during passivation**: What happens when a command arrives while the entity is being passivated? If the entity is in PASSIVATING state, the mailbox is frozen — the sender receives EntityPassivating error immediately and should retry later. The in-flight command already in the mailbox is drained and processed normally. If the entity is already PASSIVATED, the sender detects a stale handle, the runtime auto-reactivates the entity (spawns new task, transitions to RECOVERING), and delivers the command transparently.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Developers MUST be able to define an entity by declaring its command type, event type, state type, and error type. The framework provides the persistence, recovery, and concurrency infrastructure.
- **FR-002**: An entity definition MUST specify an initial state, a command handler that produces events (or returns an error), and an event applier that evolves state. The command handler signature MUST be a pure function: `(state, command, context) -> (events | error)`.
- **FR-003**: Developers MUST be able to send commands to an entity by providing the entity identifier, the command, and the CommandContext, receiving the command result asynchronously via an EntityRef API.
- **FR-004**: The framework MUST handle the full lifecycle for each command in this exact order: load/recover state, execute command and generate events, persist events atomically (snapshot NOT included in this atomic unit), apply events to in-memory state, store snapshot if needed (after event commit — outside the atomic transaction), publish events via EventPublisher SPI, and return the response. Snapshot storage failure MUST NOT roll back the already-committed events.
- **FR-005**: The framework MUST maintain an in-memory cache of active entities. When a command targets an entity not in the cache, the framework MUST recover the entity state by loading the most recent snapshot (if any) and replaying all events that occurred after that snapshot. After command execution completes, the entity MUST remain in the cache for subsequent commands. The cache eviction policy is governed by the passivation mechanism.
- **FR-006**: The framework MUST support at least three snapshot strategies: never take snapshots, take a snapshot every N events, and a custom strategy defined by the developer.
- **FR-007**: The runtime MUST implement an Actor Per Entity model. For each unique combination of tenant identifier, entity type, and entity identifier, exactly one actor with an exclusive bounded FIFO mailbox exists. The actor processes commands from its mailbox sequentially in FIFO order, guaranteeing the single-writer invariant: at most one command execution is active per entity at any time.
- **FR-008**: Event persistence MUST use optimistic concurrency control based on expected stream version. If the expected version does not match the current version, the persistence MUST fail with a version conflict error.
- **FR-009**: Events MUST be published to downstream consumers only after successful event commit. The framework MUST define an EventPublisher SPI (trait contract) that decouples publication from the core runtime. The runtime invokes the SPI after each successful commit. Under no circumstance may events be published before persistence confirms. Concrete publisher implementations are outside the scope of CORE-006.
- **FR-010**: Read-side projections MUST consume only persisted events. Direct updates of read models from entity command handlers are FORBIDDEN.
- **FR-011**: Every entity MUST be identified by a three-part key: tenant identifier (from CommandContext), entity type, and entity identifier. Entity state, event streams, and snapshots MUST be scoped by this triple.
- **FR-012**: During recovery replay, the framework MUST NOT execute side effects, emit new events, invoke external services, or trigger publications. Only state reconstruction is permitted.
- **FR-013**: Developers MUST be able to register entity types with the framework so the runtime knows which entities exist and how to handle commands for them.
- **FR-014**: The framework MUST define extension points for entity passivation (unloading entities from memory to free resources). Passivation may be triggered by inactivity, memory pressure, or explicit requests. Passivation is runtime-controlled; developers configure the policy but do not manually passivate individual entities as the primary mechanism.
- **FR-015**: The framework MUST provide a test-only in-memory backend that implements all persistence operations without external infrastructure, enabling deterministic unit tests.
- **FR-016**: Every command MUST carry a CommandContext with the following fields: tenant_id (required for multi-tenant scoping), correlation_id (end-to-end traceability), causation_id (causal chain linking commands to parent events/commands), approval_id (functional correlation for approvals and workflows), and an extensible metadata map. The CommandContext MUST be available to command handlers and MUST be included in persisted event metadata.
- **FR-017**: All entity operations MUST be deterministic given the same inputs. Command results, produced events, and final state MUST be reproducible during replay. The Handler Safety Contract defines the specific prohibitions.
- **FR-018**: Entities MUST be created via an explicit creation command defined by the developer (e.g., CreateAccount). A non-existent entity MUST reject all non-creation commands with an EntityNotFound error. The creation command is the only mechanism that transitions an entity from non-existent to existent.
- **FR-019**: Commands that produce zero events (read-only queries) MUST follow Strict Query Semantics: MUST NOT advance stream version, MUST NOT persist events, MUST NOT create snapshots, MUST NOT trigger event publication, and MUST return immediately with the handler result.
- **FR-020**: Each entity's mailbox MUST be a bounded FIFO queue with configurable capacity. When the mailbox is full, the sender MUST receive a MailboxFull error (synchronous rejection). Unbounded mailboxes are FORBIDDEN.
- **FR-021**: The entity actor MUST expose a lifecycle state machine with the following states: RECOVERING, ACTIVE, PASSIVATING, PASSIVATED, FAILED. State transitions MUST follow the rules defined in the Entity Runtime Execution Model section.
- **FR-022**: Commands arriving during the RECOVERING state MUST be queued in the mailbox. They MUST be processed in FIFO order after the entity transitions to ACTIVE. No command MAY execute before recovery completes.
- **FR-023**: Commands arriving during the PASSIVATING state MUST be rejected with EntityPassivating error. The mailbox MUST be frozen atomically on entry to PASSIVATING (sender handles removed from registry), guaranteeing no command can be accepted after the transition. Commands arriving during the PASSIVATED state MUST trigger automatic recovery: the entity transitions to RECOVERING, a new mailbox is created, and the command is enqueued.
- **FR-024**: Command handler reentrancy MUST be forbidden. If a command handler attempts to send a command to its own entity, the runtime MUST return a ReentrancyNotAllowed error.
- **FR-025**: Entity state MUST be thread-local to its actor. No shared mutable state MAY exist between entity actors. Actor state is accessed exclusively through the mailbox — there is no external state access path.
- **FR-026**: Once an event is persisted, it MUST be considered irrevocably committed. No subsequent failure (apply error, snapshot error, publication error, restart) MAY roll back, invalidate, or skip a committed event. The event stream is ALWAYS the single source of truth.
- **FR-027**: Recovery from FAILED state MUST be on-demand (admin action or restart). There MUST NOT be an automatic retry loop. Recovery replays ALL committed events deterministically, reproducing every prior state transition including those that produced business errors during original execution.
- **FR-028**: Stream version MUST start at 0 (no events) and MUST equal the count of committed events. Events in the stream MUST be contiguous (no version gaps). Version MUST be incremented atomically with event commit. The runtime MUST NOT maintain a version separate from the persisted event store.

### Handler Safety Contract

All command handlers, event appliers, and recovery replay logic MUST behave as deterministic functions:

```
(state, command, context) -> (events | error)   // command handler
(state, event) -> state                          // event applier
```

**Forbidden in handlers and appliers:**

- Reading system clock or wall-clock time
- Generating random numbers
- Network calls (HTTP, gRPC, sockets)
- Filesystem access (read or write)
- Thread spawning or concurrency primitives
- Accessing global mutable state
- Direct infrastructure calls (databases, message brokers)
- Triggering workflows
- Modifying external infrastructure

**During recovery replay:**

In addition to the above, the following are FORBIDDEN:
- Emitting new events
- Triggering event publication
- Executing side effects of any kind
- Invoking external services

**CI guard**: The project SHOULD incorporate automated CI validation to detect known non-determinism patterns (clock access, random generation, network calls). The CI guard is a development aid; the architectural contract defined by this specification remains the source of truth.

### Key Entities

- **Persistent Entity**: A domain object whose state is derived from an event stream. Defined by the developer with command handling, event generation, and event application logic. The framework manages its lifecycle.
- **CommandContext**: Runtime metadata carried with every command. Contains: tenant_id (multi-tenant isolation), correlation_id (end-to-end traceability), causation_id (causal chain linking to parent event/command), approval_id (functional correlation key for approvals and workflows), and an extensible metadata map. Available to command handlers and included in persisted event metadata.
- **Entity Command**: An instruction to modify or query an entity. Contains the domain-specific payload and is always accompanied by a CommandContext. Mutation commands produce one or more events; query commands (zero-event) return a result immediately without persisting any state.
- **Domain Event**: A record of a state change that occurred in an entity. Immutable once persisted. The source of truth for entity state reconstruction.
- **Entity State**: The current derived state of an entity, obtained by applying all events in order to the initial state. Reconstructed on recovery from snapshots and events.
- **Entity Reference (EntityRef)**: An ephemeral handle created per command invocation that targets a specific entity by its three-part key (tenant, type, ID). It does not hold the entity state — the runtime manages the in-memory cache internally. Used by application code to send commands to the entity.
- **Entity Runtime**: The framework component that manages entity lifecycle: state recovery, command execution, event persistence, snapshot management, event publication, and concurrency control.
- **Snapshot Strategy**: A policy that determines when snapshots of entity state are taken. Snapshots are stored after event commit (outside the atomic transaction). They represent the entity state at a specific stream version (last-applied version). Supports: never, every N events, or custom logic defined by the developer.
- **EventPublisher SPI**: A trait contract defining how committed events are published to downstream consumers. The runtime invokes the SPI after successful event commit. Concrete implementations (in-memory, outbox, broker-backed) are defined outside CORE-006.
- **Entity Actor**: The runtime-owned execution unit for a single entity. Implemented as a dedicated Tokio task that owns the mailbox receiver, the in-memory state, and the lifecycle state machine. One actor task per (tenant, entity_type, entity_id) when ACTIVE or RECOVERING. The task completes on passivation; a new task is spawned on reactivation.
- **Mailbox**: A bounded FIFO queue (Tokio mpsc channel) attached to each entity actor. Commands are delivered to the mailbox via the sender handle and processed sequentially by the dedicated task. Capacity is configurable at the runtime level.
- **Passivation Registry**: A lightweight in-memory registry that tracks which entities are PASSIVATED (entity triple + last known version). Used to detect and reactivate entities on command arrival.
- **EntityNotFound Error**: An error returned when a command is sent to a non-existent entity. Only an explicit creation command can create an entity; all other commands sent to non-existent entities MUST fail with this error.
- **Version Conflict Error**: An error returned when an optimistic concurrency check fails. Indicates the entity was modified by another writer between loading and persisting.
- **EntityPassivating Error**: Error returned when a command is sent to an entity that is currently PASSIVATING. The caller should retry — the entity will be available shortly.
- **MailboxFull Error**: Error returned when a command is sent to an entity whose mailbox has reached capacity. The caller must handle backpressure.
- **ReentrancyNotAllowed Error**: Error returned when a command handler attempts to send a command to its own entity.

## Entity Runtime Execution Model

This section defines the single coherent execution model for the entity runtime, resolving all behavioral ambiguities and serving as the authoritative reference for implementation.

### 1. Execution Model: Actor Per Entity

Each persistent entity is modeled as a logical actor with exclusive ownership of its execution context:

- **1 entity = 1 dedicated Tokio task** — each (tenant, entity_type, entity_id) triple in ACTIVE or RECOVERING state runs exactly one Tokio task. The task owns the entity's mailbox receiver and in-memory state. When the entity passivates, the task completes. Reactivation spawns a new task.
- **Exclusive mailbox** — every command targeting an entity is delivered to its mailbox (bounded Tokio mpsc sender). The actor task processes commands sequentially from its mailbox: one command at a time, in FIFO order, in a loop (`receive → process → receive`).
- **No reentrancy** — a command handler MUST NOT send a new command to its own entity. Attempting to do so MUST result in a runtime error (ReentrancyNotAllowed).
- **No reentrancy across the pipeline** — command handlers, event appliers, and snapshot operations for the same entity MUST NOT interleave. The full lifecycle completes before the next command starts.
- **Concurrency budget** — the runtime enforces a global concurrency limit on how many entity tasks may be actively processing at once. This is a scheduling throttle (not a pool of shared workers). Idle entity tasks parked on their mailbox receiver do not count toward the limit. The limit prevents resource exhaustion while allowing all entities to remain resident with zero scheduling overhead when idle. **Scheduling**: best-effort FIFO with anti-starvation guarantee — entities are generally processed in arrival order, but the scheduler MAY reorder to prevent starvation; every pending entity MUST eventually get a slot under sustained load. The slot selection policy is implementation-defined with the no-starvation constraint.
- **Single-writer guarantee** is enforced by the dedicated task: only one task can access the entity's state at any time, and the task processes commands sequentially. The mailbox IS the single-writer lock.

### 2. Mailbox Model: Bounded FIFO

- **Queue semantics** — each entity's mailbox is a bounded FIFO queue backed by a Tokio mpsc channel. Commands are delivered in arrival order and processed sequentially by the entity's dedicated task.
- **Bounded capacity** — the mailbox (Tokio mpsc channel) has a configurable maximum capacity. If the mailbox is full, the sender receives a MailboxFull error immediately (synchronous rejection via `try_send`). This provides built-in backpressure.
- **No unbounded queues** — mailbox capacity MUST be bounded to prevent memory exhaustion under load.
- **Ordering guarantee** — FIFO within a single entity's mailbox (enforced by mpsc channel ordering). No ordering guarantees exist across different entities.
- **Mailbox lifecycle** — the mailbox (mpsc channel) is created when the entity task is spawned (first command after passivation or initial load). The task loops on the receiver; the sender handle is stored in the entity registry. When the entity passivates, the task completes and the receiver is dropped. Commands sent after passivation trigger a new task spawn with a fresh mailbox.
- **Sender interaction** — the EntityRef API holds a clone of the mpsc sender handle and submits the command via `try_send`. If the mailbox is full, `try_send` fails immediately (MailboxFull error). If the entity is PASSIVATED (no task running), the sender handle is stale — the runtime detects this, spawns a new task with a fresh mailbox, and retries the send.

### 3. Entity Lifecycle Model

Each entity transitions through the following states during its lifetime:

```
         ┌──────────────────────────────┐
         │                              ▼
    [PASSIVATED] ◄─── [PASSIVATING] ◄─── [ACTIVE] ◄─── [RECOVERING]
         │                                              │
         └──────────────────────────────────────────────┘
                    (command arrives during passivation
                     → reactive from PASSIVATED)
         
    [ACTIVE] ──── [FAILED]  (irrecoverable error)
```

| State | Description |
|-------|-------------|
| **RECOVERING** | Entity is loading snapshot and replaying events to reconstruct state. No commands are processed during recovery. Commands arriving during recovery are queued in the mailbox and processed once ACTIVE. |
| **ACTIVE** | Entity is fully operational. Commands are dequeued from the mailbox and executed through the command lifecycle. |
| **PASSIVATING** | Entity is draining before shutdown: the mailbox was frozen atomically on entry (sender handles removed from registry; new commands rejected with EntityPassivating). The current command (if any) completes, then any commands already in the mailbox are drained (processed in FIFO order). After the mailbox is empty, the in-memory state is serialized, the entity is registered as PASSIVATED, and the dedicated Tokio task completes. PASSIVATING is irreversible — passivation cancellation does not exist. |
| **PASSIVATED** | Entity has no in-memory state and no running task. The passivation metadata (entity triple + last known version) is retained in a lightweight registry so the runtime knows the entity exists. The next command spawns a new task and starts recovery. |
| **FAILED** | Entity encountered an irrecoverable error (e.g., persistent storage failure, handler panic, applier bug). Recovery from FAILED is on-demand (admin action or restart). There is no automatic retry loop. On recovery, ALL committed events are replayed deterministically — a persistent applier bug causes recovery to fail again by design, surfacing the bug immediately. |

**State transitions:**

- RECOVERING → ACTIVE: automatic when recovery (snapshot load + event replay) completes successfully.
- ACTIVE → RECOVERING: does not occur in normal operation. An entity transitions directly from PASSIVATED → RECOVERING when a new command arrives.
- ACTIVE → PASSIVATING: triggered by the passivation policy (inactivity timeout, memory pressure, explicit request).
- PASSIVATING → PASSIVATED: automatic when the current command completes, the mailbox is drained, and the state is serialized. PASSIVATING is irreversible — no transition back to ACTIVE exists.
- PASSIVATED → RECOVERING: automatic when a command arrives for a passivated entity.
- ACTIVE → FAILED: triggered by an irrecoverable error during command execution.
- FAILED → RECOVERING: triggered by explicit recovery request or runtime restart.

### 4. Failure Determinism Model

This section defines the single coherent failure model for the entity runtime: Strict Event Sourcing.

#### 4.1 Failure Classification

Failures are classified into exactly two categories:

**A) Deterministic Business Errors** — part of the event-sourced model, reproducible during replay, derived from events/state/command:
- Handler returns an error (business rule violation, e.g., insufficient balance)
- Event applier returns an error (bug in event applier)
- VersionConflict (optimistic concurrency check)
- These are deterministic: given the same (state, command, context), the same error is produced.

**B) Non-Deterministic Runtime Failures** — infrastructure or runtime issues, NOT part of the event model, NOT replayed:
- Storage I/O error (EventStore unavailable)
- Snapshot I/O error
- EventPublisher SPI failure
- Handler panic (non-determinism, bug, memory corruption)
- These are non-deterministic: they depend on external system state and may not reproduce on replay.

#### 4.2 Failure Timing Semantics

**Event Store Consistency Contract**: The event stream is ALWAYS the single source of truth. Once an event is persisted, it is irrevocably committed. Committed events are NEVER invalidated, rolled back, or skipped. "Persist then fail" is possible — the event is committed even if subsequent processing fails.

| Stage | Failure | Event committed? | Entity state | Retry policy |
|-------|---------|-----------------|--------------|--------------|
| Load/recover state | Storage error (EventStore/Snapshot unavailable) | N/A — no event in flight | → FAILED. Command remains in mailbox. | On-demand or restart-triggered recovery. Command re-processed after successful recovery. |
| Execute command | Handler returns business error | No — no event persisted | Stays ACTIVE. Next mailbox command processed normally. | Caller may retry the same command (new execution). |
| Execute command | Handler panics | No — no event persisted | → FAILED. Offending command discarded from mailbox. | On-demand recovery. Command is NOT retried (discarded). Subsequent mailbox commands processed after recovery. |
| Persist events | VersionConflict | No — persist rejected by store | Stays ACTIVE. | Caller must retry with current version. No automatic retry. |
| Persist events | Storage error | No — persist did not complete | → FAILED. Command remains in mailbox. | On-demand recovery re-processes the command. |
| **Persist → Apply (gap)** | Persist succeeds, then crash before apply starts | **Yes — event committed** | System crashes before entity state can reflect commit. On recovery, event is replayed and applied. | Recovery replays ALL committed events deterministically. The apply phase executes as normal. |
| Apply events | Applier returns error (bug) | **Yes — event committed** | → FAILED. Recovery replays this event and reproduces the same applier error (deterministic). | Code must be fixed before recovery can succeed. Recovery is on-demand. There is NO automatic retry loop. |
| Apply → Response (gap) | Apply succeeds, but response fails (caller timeout, sender dropped) | **Yes — event committed** | Stays ACTIVE (apply completed). | Caller sees timeout/error. Caller may retry; retry hits version check (version advanced) → VersionConflict if same expected version used, or proceeds if version matches new state. |
| Snapshot | Storage error | **Yes — event already committed** | Stays ACTIVE. Snapshot failure is logged; does NOT roll back events. | Snapshot retried on next command. |
| Publish events | EventPublisher SPI error | **Yes — event already committed** | Stays ACTIVE. Publication failure is logged. | Publication retried asynchronously by publisher implementation (outside CORE-006). |

**Key invariants:**
- Once an event is persisted, it is ALWAYS committed. No subsequent failure can un-commit it.
- If the event is committed, ALL future recovery runs WILL apply it (deterministic replay).
- If the event is NOT committed, the command is retried or discarded — no partial state exists.
- At no point does a committed event become conditionally applied. The event stream is the exclusive source of truth.

#### 4.3 Replay Semantics

Recovery replay follows Strict Event Sourcing:

- Replay reproduces ALL prior state transitions deterministically, including those that produced business errors during the original execution.
- Every committed event is visited in order. The event applier is invoked for each event.
- If an event applier has a bug, recovery reproduces the exact same applier failure — this is by design, surfacing the bug immediately.
- Replay does NOT execute command handlers, produce new events, trigger publications, or perform side effects. Only event appliers run (FR-012).
- Replay is idempotent: replaying the same event stream from the same snapshot produces identical state.
- There is no concept of "skipping" or "marking unapplied" a committed event.

#### 4.4 Entity Recovery After Failure

- FAILED → RECOVERING is triggered ON-DEMAND: by explicit admin request, runtime restart, or deployment of a code fix. There is NO automatic retry loop.
- Recovery loads the most recent snapshot (if any) and replays ALL events committed after that snapshot in order.
- Snapshots are optional: if no snapshot exists, recovery replays from the beginning of the event stream.
- If recovery fails (e.g., applier bug persists), the entity returns to FAILED. Admin must fix the code and trigger a new recovery attempt.
- Failed commands during recovery: commands that were in the mailbox when the entity entered FAILED remain queued (discarded only for handler panics). After successful recovery → ACTIVE, they are processed normally.
- Recovery does not re-execute command handlers. It only reconstructs state via event appliers.

#### 4.5 Concurrency During Recovery

- Commands arriving while an entity is RECOVERING are queued in the mailbox.
- The mailbox MUST accept commands during recovery (it is created before recovery begins).
- Commands are processed in FIFO order after the entity transitions to ACTIVE.
- Recovery itself is single-threaded and non-concurrent.

#### 4.6 Concurrency Across Entities

- Different entities execute concurrently with no synchronization between them. Each entity has its own dedicated task.
- The runtime enforces a global concurrency budget (maximum number of entity tasks actively processing). This is a scheduling throttle, not a pool of shared workers. If the budget is saturated, the runtime delays spawning new entity tasks until capacity frees up. Mailbox sends to unspawned entities succeed (the sender handle exists via the entity registry), but processing begins only when the budget allows. **Scheduling**: best-effort FIFO with anti-starvation guarantee — entities are generally admitted in arrival order, but the scheduler MAY reorder to prevent starvation. Every pending entity MUST eventually get a slot under sustained load.

### 5. Passivation Interaction (Irreversible Drain)

- **Passivation trigger**: Passivation is triggered by the runtime's passivation policy (inactivity timeout, memory pressure, explicit administrative request). The policy is configured at the runtime level, not per entity.
- **Atomic mailbox freeze on entry**: When the entity transitions to PASSIVATING, the mailbox sender handles are atomically removed from the entity registry. Any concurrent or subsequent `try_send` from an EntityRef finds no handle and resolves immediately with EntityPassivating error. This freeze is the first step of the state transition — there is no window where a new command could be accepted after PASSIVATING is entered.
- **Drain existing commands**: The entity's dedicated task continues running. It completes the current command (if any), then processes any commands already in the mailbox in FIFO order. No new commands can arrive (mailbox is frozen), so the drain is bounded and deterministic.
- **Shutdown**: After the mailbox is empty and the current command completes:
  1. The in-memory state is serialized (if needed for snapshot).
  2. The entity is registered as PASSIVATED in the passivation registry (entity triple + last known stream version).
  3. The Tokio task completes; the mailbox receiver is dropped, closing the channel permanently.
- **Command during PASSIVATED**: If a command arrives after passivation is complete, the EntityRef detect the stale sender handle (channel closed). The runtime looks up the entity in the passivation registry, spawns a new task with a fresh mailbox (mpsc channel), transitions the entity to RECOVERING, and delivers the command. This is transparent to the caller — the EntityRef API abstracts the recovery.
- **Reactivation safety (single-flight)**: The single-actor invariant (exactly one task per entity triple) is a strict runtime guarantee. Reactivation is **single-flight per entity**: at most one activation process may be in-flight at any time. When a command arrives and the entity is PASSIVATED, the runtime atomically marks the entity as pending-reactivation and spawns the task. Any concurrent command arriving during this window MUST be redirected to the in-flight activation — it MUST NOT trigger a separate spawn attempt. The guard mechanism (registry CAS, per-entity lock, single-flight pattern, or equivalent) is implementation-defined; only the outcome (exactly-once actor creation per transition window) is mandatory. Stale sender handle detection and registry synchronization are purely internal implementation details, not part of the semantic model.
- **No passivation cancellation**: PASSIVATING → ACTIVE does not exist. Once an entity enters PASSIVATING, it always proceeds to PASSIVATED. Reactivation always goes through PASSIVATED → RECOVERING → ACTIVE. This eliminates all race conditions around command arrival during the drain window.
- **Zero command loss guarantee**: Every command that arrives before the mailbox freeze is drained and processed. Every command that arrives after the freeze is returned to the sender as EntityPassivating error — the command is not lost, the sender must retry. At no point is a command silently dropped.
- **Passivation is not blocking**: the runtime does not wait synchronously for passivation to complete. Passivation is an asynchronous background operation. The entity's task completes asynchronously after the mailbox drains.

## Versioning & Snapshot Consistency Model

This section defines the single coherent versioning and snapshot model: Zero-Based, Last-Applied Version.

### 6.1 Version Semantics

- **Version start**: Version starts at **0**, representing the initial state before any events exist. When a creation command persists the first event, the stream transitions from version 0 to version 1.
- **Version = committed event count**: The stream version is the total number of committed events for that entity. Version N means "N events have been committed."
- **Version increment timing**: Version is incremented atomically during event persist. The new event's sequence number is `current_version + 1`. After the atomic commit, the stream version becomes `current_version + 1`. There is no window where an event is persisted but the version has not advanced — they are part of the same atomic operation.
- **Zero-event commands**: Do NOT advance version (FR-019). The stream version after a zero-event command is identical to before it.
- **Version gaps**: Version gaps are FORBIDDEN. Events in an entity's stream MUST be contiguous: (seq 1, seq 2, seq 3, ...). A gap (e.g., seq 1, seq 3) is a stream corruption and MUST be detected during recovery.

### 6.2 Snapshot Version Alignment

- **Snapshot represents last applied event version**: A snapshot stored at version V represents the entity state after applying all events from seq 1 through seq V inclusive.
- **Snapshot metadata**: Each snapshot MUST store the stream version it represents (the "last applied" version). This allows the runtime to know exactly which events to replay after loading the snapshot.
- **Recovery procedure**:
  1. Load the most recent snapshot (if any). Extract its version V.
  2. Load events from the event stream.
  3. Replay only events with sequence number > V.
  4. Apply each event to the snapshot state in order.
  5. After all events replayed, the reconstructed state is authoritative.
- **No snapshot case**: If no snapshot exists, V = 0. All events are replayed from the beginning of the stream.

### 6.3 Event Stream + Snapshot Consistency

- **Snapshots are pure optimization**: The event stream is ALWAYS the authoritative source of truth. Snapshots MAY be safely ignored or deleted — recovery falls back to full replay from version 0. No correctness requirement depends on snapshots existing.
- **Stale snapshot**: A snapshot at version V where events exist at version > V is not "stale" — it is simply older than the latest events. Recovery correctly replays events > V. This is the normal and expected case.
- **Missing snapshot**: Recovery replays from version 0. Correct state is reconstructed. No error is raised.
- **Corrupted snapshot**: The snapshot load fails (I/O error, deserialization error). Recovery falls back to full replay from version 0. The entity transitions through RECOVERING normally. No correctness impact — only a performance impact (longer recovery).
- **Snapshot version > latest event version**: This is a corruption or inconsistency. The snapshot claims version V but the event stream has fewer than V events. The runtime MUST treat this as a stream integrity error: discard the snapshot, fall back to full replay.

### 6.4 Recovery Determinism

- Recovery ALWAYS produces identical state given the same event stream and snapshot (if used). This is guaranteed by:
  - Events are immutable once committed.
  - Event appliers are deterministic functions (Handler Safety Contract).
  - Events are replayed in strict sequence order.
  - Snapshots are deterministic: given identical event stream up to V, snapshot at V is identical.
- Snapshot + replay is deterministic under all concurrency assumptions because:
  - The mailbox guarantees sequential command processing (no interleaved state mutations).
  - Events are committed atomically with version increment.
  - Recovery replays events in a single thread with no concurrent access.
- Version gaps are FORBIDDEN and detected during recovery.

### 6.5 Failure Interaction with Versioning

- **Command fails before persistence**: No event committed. Version unchanged. Entity stays ACTIVE (or transitions to FAILED for runtime errors). Next command retries or is discarded.
- **Persistence succeeds but apply fails**: Event committed. Version advanced (event seq N). Entity → FAILED. On recovery, event N is replayed and the apply failure is reproduced deterministically. Version remains advanced — committed events are NEVER reverted.
- **Response fails after commit**: Event committed. Version advanced. Entity stays ACTIVE (apply completed). The caller sees a timeout/error. On retry, the caller provides the expected version from before the commit — the store rejects with VersionConflict because the version has advanced. The caller must refresh state and retry with the current version.
- **Version is strictly tied to the event store**: The runtime never tracks a version that diverges from the persisted event count. The authoritative version is always read from the event store during recovery. There is no "runtime version" separate from "persisted version." This eliminates dual-version ambiguity.

## Out of Scope

### ❌ 1. Cluster Sharding

CORE-006 does NOT include distributed entity placement across nodes, shard allocation, or rebalancing.

**Reason**: Entity placement across nodes is a clustering concern, deferred to future work (CORE-007).

### ❌ 2. Distributed Execution

CORE-006 does NOT include algorithms or infrastructure for deciding which node hosts which entity.

**Reason**: Single-node entity runtime must work before distribution can be considered.

### ❌ 3. Message Brokers and Event Streaming

CORE-006 does NOT include Kafka, NATS, RabbitMQ, or any message broker integration.

**Reason**: Event publication is abstract — concrete broker integration is the responsibility of the deployment layer.

### ❌ 4. Transport Layer

CORE-006 does NOT define or require gRPC, REST, GraphQL, or any specific transport protocol.

**Reason**: The entity runtime is transport-agnostic by design. Protocol bindings are separate concerns.

### ❌ 5. Workflows and Sagas

CORE-006 does NOT implement long-running transactions, compensation logic, or multi-entity orchestration patterns.

**Reason**: These are higher-level patterns built on top of the entity runtime, not part of it.

### ❌ 6. Remote Actor Communication

CORE-006 does NOT provide remote actor protocols, location-transparent references, or wire protocols for inter-node entity communication.

**Reason**: All entity communication is in-process. Remote communication is a separate concern.

### ❌ 7. Replication

CORE-006 does NOT replicate entity state across nodes for fault tolerance or read scaling.

**Reason**: Replication is an infrastructure concern outside the entity runtime scope.

### ❌ 8. Service Registration and Discovery

CORE-006 does NOT provide service registration, health checks, or endpoint discovery.

**Reason**: Deployment and discovery are infrastructure concerns.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer can define an event-sourced entity and send commands to it within 15 minutes of starting implementation, using only the framework abstractions and an in-memory backend.
- **SC-002**: After a system restart, any entity with persisted events recovers to the correct state automatically without developer intervention. Verifiable by comparing pre-restart and post-restart state.
- **SC-003**: When two commands target the same entity concurrently, the actor model guarantees they execute sequentially — the first completes before the second begins (enforced by the mailbox, not by external locks). Verifiable with timestamped test logs.
- **SC-004**: An optimistic concurrency conflict between two writers produces a version conflict error for the second writer. Verifiable by controlled concurrent test.
- **SC-005**: Events are never observable by downstream consumers before persistence confirms. Verifiable by instrumenting both the persist and publish calls and asserting the order.
- **SC-006**: Full event replay (recovery from scratch without snapshots) produces identical entity state to the original execution. Verifiable by comparing states.
- **SC-007**: The same entity ID in two different tenants produces independent states, event streams, and snapshots. Verifiable by cross-tenant read isolation tests.
- **SC-008**: All entity behavior is testable using an in-memory backend — no database, message broker, or external service required.
- **SC-009**: When a mailbox is full, the sender receives a MailboxFull error synchronously. Verifiable by saturating an entity's mailbox and asserting the error on the next send.
- **SC-010**: A command sent to a passivated entity triggers automatic recovery and succeeds transparently. Verifiable by monitoring lifecycle state transitions in test logs.
- **SC-011**: A command sent to a PASSIVATING entity receives EntityPassivating error and succeeds on retry after passivation completes.
- **SC-012**: A command handler that attempts to send a command to its own entity receives ReentrancyNotAllowed error.
- **SC-013**: A zero-event command (Strict Query) does not advance stream version, persist events, create snapshots, or trigger publication. Verifiable by comparing pre-query and post-query version and asserting no storage/publish side effects.

## Assumptions

- The persistence SPI (CORE-001) exists and provides EventStore, Snapshot, and Repository abstractions that the entity runtime implements against.
- The read-side projection engine (CORE-005) exists and consumes published events to update read models.
- Entity command handlers are pure functions — they make decisions based on current state and command input without side effects.
- Entity event appliers are pure functions — they produce new state from current state and an event.
- Concurrency conflicts are expected to be rare in normal operation given the sequential mailbox model; the optimistic approach is acceptable.
- Tenant identity and isolation boundaries are enforced by the application layer; the entity runtime scopes data by provided tenant identifiers but does not authenticate them.
- Event serialization and deserialization are handled by the persistence layer; the entity runtime operates on typed in-memory representations.
- The entity runtime runs as a library embedded in the application process, not as a standalone service.
- Command idempotency is the responsibility of the application layer; the entity runtime provides optimistic concurrency as the conflict detection mechanism.
- Postgres is the production event store; the in-memory backend is used for testing and development.
- The mailbox capacity and passivation policy are configurable at the runtime level, not per entity.
- The Handler Safety Contract is enforced by convention and CI guard, not by the runtime at compile time.
