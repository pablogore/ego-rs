# Feature Specification: Read Side Projections

**Feature Branch**: `005-read-side-projections`

**Created**: 2026-06-04

**Status**: Draft

**Input**: User description: "CORE-005 Read Side Projections — batch-based, tag-partitioned, session-driven, idempotent, offset-controlled read-side projection engine"

## Clarifications

### Session 2026-06-04 (Initial)


- Q: ¿El Read Side consume EXCLUSIVAMENTE EventStreamElement? → A: Sí, nunca accede al EventStore directamente

### Session 2026-06-04 (Protobuf Integration)

- Q: ¿EventStreamElement payload type? → A: Generic `<E>` — domain stays protobuf-free; adapter provides concrete type
- Q: ¿Event source model? → A: Hybrid — gRPC → EventStore via adapter (write); ReadSide pulls from EventStore (read)
- Q: ¿Adapter layer crate placement? → A: New `crates/event-adapter` crate
- Q: ¿Dedup scope refinement? → A: `(projection_id, tag, event_id)` — previous "per tag" was shorthand
- Q: ¿Replay dedup behavior? → A: Dedup ON by default, configurable OFF

### Session 2026-06-04 (CloudEvents & EventStore Read)

- Q: ¿EventStore tag-based read interface? → A: New `ReadSideStore` trait in `ego-domain/read_side/`
- Q: ¿CloudEvents dependency ownership? → A: In `ego-event-adapter` crate only — domain stays CE-free
- Q: ¿ReadSide polling runtime ownership? → A: In existing `ego-runtime` crate, `read_side` module
- Q: ¿Una ReadSideSession commit implica qué atomicidad? → A: handler + offset + dedup atomically
- Q: ¿Evento con múltiples tags? → A: se procesa en cada tag stream (fan-out)
- Q: ¿Dedup state scope? → A: por tag
- Q: ¿Offset semantics? → A: último event_version confirmado (post commit)

### Session 2026-06-04 (Commit Boundaries & Runtime State Machine)

- Q: ¿What is the atomic scope of a ReadSideSession commit? → A: Metadata state only — OffsetStore update + DedupStore mark_seen are atomic; handler side effects (external DB writes, APIs) are excluded.
- Q: ¿What happens if failure occurs between handler success and commit? → A: Full retry allowed; dedup prevents duplicates on subsequent processing.
- Q: ¿What are the explicit runtime states for a projection? → A: RUNNING, REPLAYING, REBUILDING, PAUSED, FAILED.
- Q: ¿How should runtime events (progress, errors, state transitions) be reported? → A: Trait-based callback — domain defines a `ProgressReporter` trait with methods like `on_batch_completed(projection_id, tag, count, offset)` and `on_error(projection_id, error)`; host injects implementation at runner construction.

### Session 2026-06-04 (Out of Scope Boundaries)

- Q: ¿Does CORE-005 include event transport, message brokers, or event ingestion? → A: No. Transport is owned by the write-side + adapter layer. CORE-005 operates on already-materialized EventStreamElement data.
- Q: ¿Does CORE-005 include Event Store implementation? → A: No. EventStore is an external system. Read-side only consumes via ReadSideStore.
- Q: ¿Does CORE-005 provide query APIs, REST endpoints, or read model access layers? → A: No. Read models are application-specific. CORE-005 only builds them.
- Q: ¿Does CORE-005 support cross-projection coordination or global ordering? → A: No. Each projection is fully independent by design.
- Q: ¿Does CORE-005 include distributed runtime clustering or leader election? → A: No. Runtime is single-process per projection instance. Scaling is external.
- Q: ¿Does CORE-005 provide schema evolution, schema registry, or payload migration? → A: No. Schema evolution is handled by the event contract / adapter layer.
- Q: ¿Does CORE-005 include global retry queues, DLQs, or persistent retry scheduling? → A: No. Retry is strictly internal (Transient, Fatal, PoisonEvent semantics only).
- Q: ¿Does CORE-005 define read model storage, indexing, or materialized views? → A: No. Handlers own the read model. CORE-005 guarantees event delivery semantics only.
- Q: ¿Does CORE-005 support time-based or windowed stream processing? → A: No. This is batch-based projection processing, not streaming analytics.
- Q: ¿Does CORE-005 define security, authorization, or tenant isolation enforcement? → A: No. Assumed to be enforced at application boundary or infrastructure layer.
- Q: ¿Does CORE-005 include event transformation or enrichment beyond EventStreamElement construction? → A: No. Transformations belong to ego-event-adapter or application layer.

## User Scenarios & Testing *(mandatory)*

### User Story 1 — Define and Run a Tagged Projection (Priority: P1)

A developer defines a projection that consumes events belonging to specific tags, processes them in batches, and updates a read model. The projection runtime fetches events grouped by tag, assembles them into batches, and delivers them to the developer's handler as a single batch.

**Why this priority**: This is the core value of the feature — without it there is no read-side projection capability.

**Independent Test**: Can be tested by registering a handler for a known tag, feeding events into the event store with that tag, running the projection, and verifying the handler received the expected batch.

**Acceptance Scenarios**:

1. **Given** a projection registered with tags `["order"]` and a batch handler, **When** two events tagged `"order"` are available, **Then** the handler receives a single batch containing both events.
2. **Given** a projection registered with tags `["order", "payment"]`, **When** events exist for both tags, **Then** each tag's events are processed independently and concurrent per configuration limits.
3. **Given** a projection with `batch_size = 10`, **When** 25 events are available for a tag, **Then** the handler is invoked three times (batches of 10, 10, 5).

---

### User Story 2 — Idempotent Processing with Deduplication (Priority: P2)

A projection must be safe to retry. If the same event is delivered more than once (e.g., after a crash or restart), the projection runtime detects the duplicate and skips processing it, ensuring the read model stays consistent.

**Why this priority**: Idempotency is essential for correctness in any production system that processes events.

**Independent Test**: Can be tested by delivering the same event twice with the same event ID and verifying the handler is only invoked once for that event.

**Acceptance Scenarios**:

1. **Given** a projection has processed an event with ID `"evt-1"`, **When** the same event is delivered again in a subsequent batch, **Then** the handler is not invoked for `"evt-1"` and the batch continues processing remaining events.
2. **Given** a projection crashes mid-batch after persisting its offset but before marking dedup state, **When** the runtime restarts and re-fetches events up to the last committed offset, **Then** duplicate events are detected via dedup state and skipped.

---

### User Story 3 — Offset-Controlled Resumable Processing (Priority: P1)

The projection runtime tracks the last processed offset per tag per processor. After a restart, processing resumes from the stored offset, not from the beginning.

**Why this priority**: Without offset tracking, every restart would require a full replay, making the system impractical for continuous operation.

**Independent Test**: Can be tested by processing a batch, recording the offset, restarting the runtime, and verifying that already-processed events are not re-delivered.

**Acceptance Scenarios**:

1. **Given** a projection has processed events up to offset 42 for tag `"order"`, **When** the runtime restarts, **Then** it resumes fetching events starting from offset 43.
2. **Given** a projection fails after committing offset 42 but before the handler completes, **When** the runtime recovers, **Then** it re-fetches events starting from offset 43 (potential duplicates resolved by dedup).

---

### User Story 4 — Replay and Rebuild (Priority: P2)

A developer needs to reprocess all events from the beginning (replay) or completely reset and reprocess (rebuild) for a given projection. Replay re-runs the handler on all events without offset filtering. Rebuild additionally clears all existing read model data, offsets, and dedup state before replaying.

**Why this priority**: Replay/rebuild is critical for correcting read models after handler logic changes or data corruption.

**Independent Test**: Can be tested by processing events to produce a read model, then triggering a rebuild and verifying the read model is reconstructed from scratch.

**Acceptance Scenarios**:

1. **Given** a projection has processed events up to offset 100, **When** a replay is triggered, **Then** events from offset 1 are fetched and the handler is invoked for all batches, ignoring stored offsets.
2. **Given** a projection has existing read model data, offsets, and dedup state, **When** a rebuild is triggered, **Then** all read model data, offsets, and dedup state are cleared before replay begins.
3. **Given** a rebuild is in progress, **When** new events arrive, **Then** they are queued and processed after the rebuild completes.

---

### User Story 5 — Failure Classification and Retry (Priority: P3)

When a handler fails, the projection runtime classifies the failure and either retries (transient), stops (fatal), or skips the offending event (poison event).

**Why this priority**: Robust error handling determines whether the system is production-grade. Lower priority because a basic version can work without classification.

**Independent Test**: Can be tested by making a handler return each error type and verifying the runtime behavior.

**Acceptance Scenarios**:

1. **Given** a handler returns a transient error, **When** the runtime detects it, **Then** the batch is retried with backoff up to a configurable limit.
2. **Given** a handler returns a fatal error, **When** the runtime detects it, **Then** the projection is stopped and an alert is raised.
3. **Given** a handler returns a poison event error for a specific event in a batch, **When** the runtime detects it, **Then** the offending event is skipped, the rest of the batch is processed, and the error is logged.

---

### User Story 6 — Concurrent Processing with Backpressure (Priority: P3)

The projection runtime respects configured concurrency limits, avoiding unbounded parallelism that could overwhelm the system.

**Why this priority**: Important for production deployments but not needed for an MVP.

**Independent Test**: Can be tested by configuring a concurrency limit and verifying that no more than that many tag streams are processed simultaneously.

**Acceptance Scenarios**:

1. **Given** `concurrency_per_tag = 2` and four tags with pending events, **When** the runtime starts processing, **Then** at most two tags are processed concurrently.
2. **Given** `max_in_flight = 5` and 20 batches ready to process, **When** the runtime dispatches work, **Then** no more than 5 batches are in-flight at any time.
3. **Given** `batch_size = 100` and 250 events for a tag, **When** the runtime processes them, **Then** the handler receives batches no larger than 100 events.

---

### Edge Cases

- **Empty batch**: What happens when the runtime fetches events but the batch contains zero events after dedup filtering? The session should be committed with an offset update but no handler invocation.
- **Concurrent rebuild and regular processing**: If a rebuild is triggered while regular processing is running, how is the conflict resolved? The rebuild takes precedence; regular processing pauses until rebuild completes.
- **Tag with no handlers**: What happens if a tag has events but no projection has registered for that tag? Those events are ignored by the runtime.
- **Processor registration race**: What happens if two projections register for the same tag? Each operates independently with its own offset and dedup state.
- **Monotonically increasing offsets**: What happens if events arrive out of order within a tag stream? The offset model assumes monotonic order within a tag; out-of-order delivery may result in gaps.
- **Dedup store growth**: How is the dedup store bounded over long-running systems? Dedup state associated with already-compacted offsets may be pruned.
- **Commit failure semantics**: If failure occurs before the atomic commit (offset + dedup), the full batch MAY be retried — no handler side effects have been committed. If failure occurs after handler success but before the atomic commit completes, the next fetch + dedup filter will skip already-processed events via DedupStore, preventing duplicates from reaching the handler.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The runtime MUST allow developers to register projections that define a set of tags and a batch handler.
- **FR-002**: The runtime MUST fetch events from the event store grouped by tag for each registered projection.
- **FR-003**: The runtime MUST assemble events into batches no larger than the configured `batch_size` before delivering them to handlers.
- **FR-004**: Handlers MUST receive events as a batch (multiple events at once), not one event at a time.
- **FR-005**: The runtime MUST track the last processed offset per tag per projection and resume from that offset after restart.
- **FR-006**: The runtime MUST apply deduplication by event ID before invoking handlers, skipping events that have already been processed.
- **FR-007**: The runtime MUST commit offset persistence and dedup state atomically as a single unit per batch. Handler side effects (external DB writes, APIs, etc.) are explicitly excluded from the atomic scope — the runtime guarantees metadata atomicity only.
- **FR-008**: The runtime MUST support three failure modes: transient (retry with backoff), fatal (stop projection), and poison event (skip event and continue).
- **FR-009**: The runtime MUST support replay mode that ignores stored offsets and reprocesses all events for the projection. Dedup MUST be ON by default in replay mode, with an option to disable it for full reprocessing.
- **FR-010**: The runtime MUST support rebuild mode that clears read model data, offsets, and dedup state before running a full replay.
- **FR-011**: The runtime MUST respect `concurrency_per_tag` limits, processing no more than the configured number of tag streams in parallel.
- **FR-012**: The runtime MUST respect `max_in_flight` limits, keeping no more than the configured number of batch operations in-flight.
- **FR-013**: The runtime MUST support multiple interchangeable backend storage implementations.
- **FR-014**: Offsets MUST be represented as a sequence number, derived from the event version within a tag stream.
- **FR-015**: Event tags MUST be deterministically computable from the event payload and aggregate ID.
- **FR-016**: Each event delivered to the runtime MUST include its complete metadata: event ID, aggregate ID, tenant ID, event type, payload, version, timestamp, and tags.
- **FR-020**: The read-side runtime MUST NOT access the EventStore directly; all event data is consumed exclusively via EventStreamElement.
- **FR-021**: An event with multiple tags MUST be processed independently in each tag stream (fan-out). Each tag stream receives its own copy of the event.
- **FR-017**: The runtime MUST NOT require global ordering across tags; ordering guarantees apply only within a single tag stream.
- **FR-018**: Projections MUST be able to register interest in multiple tags.
- **FR-019**: The runtime MUST support an in-memory backend for testing that does not require any external infrastructure.
- **FR-022**: There MUST be an Event Adapter Layer that converts protobuf-defined events to `EventStreamElement<ProtoEvent>`, applying EventTagger and normalizing versions.
- **FR-023**: The Event Adapter Layer MUST operate on the write path (before events enter the EventStore), not on the read path.
- **FR-024**: The Adapter Layer MUST support version routing for protobuf event types (v1/v2/etc) to ensure backward compatibility.
- **FR-025**: The runtime MUST provide a `ReadSideStore` trait (separate from `EventStore`) that supports fetching events by tag with offset-based pagination for read-side consumption.
- **FR-026**: The `ego-event-adapter` crate MUST own all CloudEvents types and conversions — the domain crate MUST NOT depend on CloudEvents.
- **FR-027**: The event flow MUST follow: protobuf → CloudEvent → EventStore → ReadSideStore pull → EventStreamElement.
- **FR-028**: The runtime MUST provide a `ProgressReporter` trait (in the domain crate) with methods for reporting batch completion, errors, and state transitions. The host application injects the implementation at runner construction.

### Key Entities

- **Event Stream Element**: The unit of consumption for the read side. Contains the event payload plus full metadata (event ID, aggregate ID, tenant, type, version, timestamp, and computed tags).
- **Tag**: A partition key that defines a stream of events. Events belonging to the same tag are ordered relative to each other. No ordering guarantees exist across tags.
- **Projection / Read-Side Processor**: A registered consumer that declares interest in specific tags and provides a batch handler. Each projection maintains its own offset and dedup state per (tag, tenant).
- **Offset**: A sequence number (derived from event version) representing the last event_version confirmed after an atomic commit. Used for resumability after restart; the runtime resumes from offset + 1.
- **Dedup State**: Per `(projection_id, tag, event_id)` tracking that ensures idempotent processing. Each projection maintains independent dedup state per tag stream, preventing cross-projection interference.
- **Read-Side Session**: The execution unit that groups events for a tag into a batch, runs the handler, persists offsets, and commits dedup state. The session commit guarantees atomicity for metadata state only (offset + dedup) — handler side effects are excluded from the transaction boundary.
- **Read-Side Config**: Controls batch size, max in-flight operations, and concurrency per tag.
- **Event Adapter Layer**: Converts protobuf-defined events through the CloudEvents envelope into EventStore records and back to `EventStreamElement` on the read path. Handles: protobuf→CloudEvent→EventStore (write), EventStore→EventStreamElement (read). Owns the CloudEvents dependency. Lives in `crates/event-adapter`.
- **Progress Reporter**: A trait-based callback interface for reporting runtime events. Methods include `on_batch_completed(projection_id, tag, count, offset)`, `on_error(projection_id, error)`, and `on_state_transition(projection_id, from, to)`. Implementations can log, emit metrics, or both. Lives in `ego-domain/read_side/progress.rs`.

### Runtime State Machine

A projection transitions between the following states during its lifecycle:

| State | Description |
|-------|-------------|
| `RUNNING` | Normal processing: fetching events per tag, executing batches, committing offsets and dedup |
| `REPLAYING` | Reprocessing all events from the beginning (ignoring stored offsets); dedup ON by default, configurable OFF |
| `REBUILDING` | Clearing all read model data, offsets, and dedup state, then running a full replay from scratch |
| `PAUSED` | Processing suspended; projection accepts no new batches until resumed or restarted |
| `FAILED` | Irrecoverable error (e.g., fatal handler error after all retries exhausted); projection stopped |

**State transitions** (defined at the runtime layer):
- `RUNNING` → `REPLAYING`: triggered by `ReadSideRunner::replay()` call
- `RUNNING` → `REBUILDING`: triggered by `ReadSideRunner::rebuild()` call
- `RUNNING` → `PAUSED`: triggered by manual pause or transient threshold exceeded
- `PAUSED` → `RUNNING`: triggered by manual resume
- `RUNNING` → `FAILED`: triggered by `ProjectionError::Fatal` from handler or unrecoverable runtime error
- `REPLAYING` → `RUNNING`: automatic when replay completes
- `REBUILDING` → `RUNNING`: automatic when rebuild completes

New events arriving during `REPLAYING` or `REBUILDING` are queued and processed after the replay/rebuild completes.

## Out of Scope

These boundaries explicitly define what CORE-005 DOES NOT provide. They prevent scope creep, architectural misuse, and incorrect assumptions about system capabilities.

### ❌ 1. Event Transport Layer

CORE-005 does NOT define or implement any message broker, streaming infrastructure, or event ingestion mechanism (Kafka, NATS, RabbitMQ, etc.).

**Reason**: CORE-005 operates on already-materialized `EventStreamElement` data. Transport is owned by the write-side + adapter layer.

### ❌ 2. Event Store Implementation

CORE-005 does NOT include event persistence logic, versioning storage strategy, partitioning, or append-only log implementation.

**Reason**: `EventStore` is an external system. Read-side only consumes via `ReadSideStore`.

### ❌ 3. Query API / Read Model API Layer

CORE-005 does NOT provide REST APIs, GraphQL layers, query builders, or client-facing read endpoints over projections.

**Reason**: Read models are application-specific. CORE-005 only builds them.

### ❌ 4. Cross-Projection Coordination

CORE-005 does NOT support global ordering across projections, inter-projection communication, shared transactional consistency, saga orchestration, or workflow engines.

**Reason**: Each projection is fully independent by design.

### ❌ 5. Distributed Runtime / Clustering

CORE-005 does NOT include cluster membership, leader election, distributed locking, or multi-node coordination of a single projection instance.

**Reason**: The runtime is single-process per projection instance. Scaling is handled externally at the deployment layer.

### ❌ 6. Schema Evolution System

CORE-005 does NOT provide a schema registry, automatic migration of `EventStreamElement<E>`, backward compatibility enforcement, or version negotiation inside projections.

**Reason**: Schema evolution is handled by the event contract / adapter layer, not the read-side engine.

### ❌ 7. Retry Infrastructure Outside Projection Scope

CORE-005 does NOT include global retry queues, dead-letter queues (DLQ), persistent retry scheduling outside the projection loop, or external job systems.

**Reason**: Retry is strictly internal to projection execution (`Transient`, `Fatal`, `PoisonEvent` semantics only).

### ❌ 8. Read Model Storage Engine

CORE-005 does NOT define how read models are stored (SQL, NoSQL, cache, etc.), indexing strategy, or materialized view implementation.

**Reason**: Handlers own the read model. CORE-005 only guarantees event delivery semantics.

### ❌ 9. Time-Based or Windowed Stream Processing

CORE-005 does NOT support tumbling windows, sliding windows, event-time aggregation semantics, or stream joins.

**Reason**: This is batch-based projection processing, not streaming analytics.

### ❌ 10. Security / Authorization Model

CORE-005 does NOT define access control for projections, tenant isolation enforcement policies, or authentication/authorization rules.

**Reason**: Assumed to be enforced at application boundary or infrastructure layer.

### ❌ 11. Event Transformation Logic Beyond Adapter

CORE-005 does NOT include business transformation pipelines, enrichment beyond `EventStreamElement` construction, or dynamic mapping rules inside the runtime.

**Reason**: Transformations belong to `ego-event-adapter` or application layer.

**Boundary Principle**: CORE-005 consumes events. CORE-005 does NOT produce infrastructure.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer can define a projection with a batch handler, register it, and have events delivered in batches within 10 minutes of starting implementation.
- **SC-002**: After a runtime restart, all projections resume processing from the correct offset without re-processing previously committed events (except duplicates handled by dedup).
- **SC-003**: A replay produces exactly the same read model state as the original processing run, given the same handler logic.
- **SC-004**: A rebuild clears all existing read model state, offsets, and dedup data, and produces a fresh read model from scratch.
- **SC-005**: The system can process 10,000 events per tag with zero duplicates reaching the handler, verified by event ID tracking.
- **SC-006**: Transient handler errors trigger at least one retry before the batch is considered failed; poison events are skipped without affecting other events in the same batch.
- **SC-007**: The runtime never exceeds the configured `concurrency_per_tag` or `max_in_flight` values during normal operation.
- **SC-008**: All projection behavior is testable using an in-memory backend without any external dependencies.
- **SC-009**: Event ordering is guaranteed within a single tag stream; no ordering guarantees exist across different tags.

## Assumptions

- The event store exists and provides a mechanism to fetch events filtered by tag with offset-based pagination.
- Event metadata (event ID, aggregate ID, tenant, type, version, timestamp, tags) is available at the time the event is fetched — tags are pre-computed by the event tagger.
- The projection runtime is pull-based: it actively polls or is scheduled to fetch new events.
- Dedup state retention is tied to offset lifecycle — state for offsets that have been compacted may be pruned.
- Handlers are synchronous for the purpose of batch execution; long-running handlers may block the tag stream.
- The in-memory backend is sufficient for all testing; a persistent backend (e.g., Postgres) is needed for production deployments.
- Each projection progresses independently — there is no coordination or shared state between projections.
- The runtime is deployed as a library embedded in the application process, not as a separate service.
- Events originate as protobuf types (buf-managed contracts) and are wrapped in CloudEvents as the interoperability envelope before reaching the EventStore.
- The CloudEvents SDK (or equivalent representation) is a dependency of `ego-event-adapter` only.
