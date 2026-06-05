# Research: Read Side Projections — Design Decisions

## 1. Tag Source Strategy

**Decision**: Tags are precomputed by `EventTagger` at event creation time and stored in `EventStreamElement.tags`. The runtime never recalculates tags.

**Rationale**: Deterministic tag assignment during event production ensures all consumers see the same tags for the same event. Recalculation at consumption time could produce different results if tagger logic evolves, leading to inconsistent stream membership between projections. Precomputed tags also eliminate redundant computation across N projections consuming the same event.

**Alternative considered**: Runtime recalculation via `EventTagger` on each fetch — rejected because it introduces temporal coupling between tagger version and stream ordering. If tagger logic changes mid-stream, old events might get new tags, breaking the ordering invariant.

**Consistency rule**: If a mismatch is detected between precomputed tags and the current `EventTagger` output (e.g., during validation), the precomputed tags win — no recalculation, no validation failure. The `EventTagger` is the source of truth only at event creation time.

---

## 2. Ordering Within Tag

**Decision**: `event_version` must be monotonically increasing within a tag stream but MAY have gaps. Only relative order matters — not sequential density.

**Rationale**: In event-sourced systems, version gaps are natural (concurrent aggregates, skipped versions, compaction). Enforcing gapless sequences would require global coordination per tag, which conflicts with the "no global order" constraint. Relative ordering is sufficient for correct projection semantics — if event A has version 1 and event B has version 5, A is guaranteed to have occurred before B.

**Out-of-order handling**: Events received out-of-order are rejected. The runtime tracks the last confirmed offset per tag; any event with `event_version <= last_confirmed_offset` is a duplicate (handled by dedup). Events with `event_version > last_confirmed_offset + 1` are accepted — gaps are allowed.

---

## 3. Failure Model: Transient Retry Policy

**Decision**: Exponential backoff with configurable base delay, max delay, and max retries. Default: base 100ms, max 10s, max 3 retries.

**Rationale**: Immediate retry is too aggressive for downstream failures (DB contention, network blips). Exponential backoff is standard practice and prevents thundering herd on recovery. Configurable policy allows tuning per projection without code changes.

**Poison event handling**: The offending event is skipped, logged with full metadata, and the batch continues processing. The tag stream is NOT stopped. Subsequent fetches exclude the skipped event_id from the batch (dedup marks it as seen without handler execution).

---

## 4. Handler Contract

**Decision**: Handlers MAY have side effects (external API calls, secondary storage writes). The runtime only guarantees: (a) handler receives the batch, (b) handler completion signals commit readiness, (c) handler error classification determines runtime behavior.

**Rationale**: Forcing pure handlers would prevent legitimate use cases (sending notifications, updating caches, calling external services). The atomicity guarantee covers handler writes + offset + dedup within the projection's scope. External side effects are best-effort from the runtime's perspective — if the handler commits its DB write but an external API call fails after the atomic commit, the external call is not retried.

---

## 5. Backend Contract Strictness

**Decision**: Backends must guarantee atomicity of (handler writes + offset + dedup) within a single transaction where possible. If the backend cannot provide full atomicity (e.g., in-memory without transactional storage), it must at minimum guarantee offset + dedup atomicity and document the lack of handler write atomicity.

**Rationale**: Full atomicity is the ideal (Q2 clarification). However, not all storage engines support distributed transactions spanning application state + runtime metadata. The in-memory backend uses a struct-level lock for atomicity. PostgreSQL uses a database transaction. The contract is: "at least offset + dedup atomic; handler write atomicity is backend-dependent."

---

## 6. Concurrency Model Boundaries

**Decision**:
- `max_in_flight` applies globally (across all projections and tags) — total number of batch operations executing concurrently.
- `concurrency_per_tag` applies to the number of tag streams within a single projection that can be fetched + queued simultaneously.

**Rationale**: Global `max_in_flight` prevents resource exhaustion (too many concurrent handler executions). Per-tag concurrency prevents a projection with many tags from flooding the system. The two controls operate at different levels: `concurrency_per_tag` controls tag stream dispatch; `max_in_flight` caps total concurrent work.

---

## 7. Dedup Persistence Between Restarts

**Decision**: Dedup state MUST persist between restarts when using a persistent backend (Postgres). For the in-memory backend, dedup state is lost on restart (acceptable for testing).

**Rationale**: Production systems require idempotent recovery after crash. In-memory backend is for testing only, where session-level isolation is sufficient.

---

## 8. Rebuild Semantics

**Decision**: Rebuild clears: read model data + offsets + dedup state + any internal runtime caches (e.g., in-flight batch tracking).

**Rationale**: A rebuild must produce a fresh read model as if no processing ever occurred. Leaving any state behind risks inconsistency. Runtime caches (not specified in the spec but implied by the runtime) must be cleared to avoid stale references.

---

## 9. EventStreamElement Ownership

**Decision**: `EventStreamElement` is immutable. It is created by the fetch layer as a snapshot of the stored event + precomputed tags. The runtime and handlers receive read-only references.

**Rationale**: Immutability eliminates accidental cross-tag stream interference. If a handler mutates an element, it could affect another tag stream processing the same event. The functional programming principle (`.speckit/constitution.md`) strongly prefers immutable data.

---

## 10. ReadSideStore vs EventStore Separation

**Decision**: `ReadSideStore` is a separate trait from `EventStore`, defined in `ego-domain/read_side/store.rs`. It exposes `fetch(tag, offset, batch_size)` for tag-based consumption queries.

**Rationale**: The existing `EventStore` trait is optimized for aggregate-based append/load operations in the command side. Read-side queries have different access patterns (sequential scan by tag, offset streaming). Merging them would couple the command-side interface to read concerns. A separate trait keeps each interface focused and prevents unintended cross-layer coupling.

**Backend implications**: Both `InMemoryReadSideStore` and `PostgresReadSideStore` implement the new trait alongside existing `InMemoryEventStore` and `PostgresEventStore`.

---

## 11. CloudEvents Integration Pattern

**Decision**: CloudEvents is the standard interoperability envelope on the write path only. The `ego-event-adapter` crate owns the conversion chain: protobuf → CloudEvent → EventStore record → EventStreamElement. The domain crate never imports CloudEvents types.

**Rationale**: CloudEvents provides a vendor-neutral envelope standard with schema versioning, required attributes (id, source, specversion, type, time), and extension points. Using it as the canonical transport format decouples the protobuf contract layer from storage. The read-side remains agnostic because the adapter handles conversion before the event reaches `ReadSideStore`.

**Adapter responsibilities**:
1. `protobuf_to_ce`: maps buf-generated event → `CloudEventBuilder` (sets id, source, type, dataschema, time)
2. `ce_to_eventstore`: serializes CloudEvent to the storage format (e.g., JSON columns for attributes + binary payload)
3. `eventstore_to_ese`: reconstructs `EventStreamElement<ProtoEvent>` from stored record, applying `EventTagger` and version normalization

---

## 12. Polling Runtime Ownership

**Decision**: The ReadSide async polling and scheduling lives in `ego-runtime/src/read_side/` as a new module, not a separate crate.

**Rationale**: `ego-runtime` already owns the actor system's scheduling infrastructure (mailbox, supervision, timers). The read-side polling loop follows the same pattern — periodic fetch, dispatch, collect. Creating a separate runtime crate would duplicate scheduling infrastructure without justification. The module is cleanly separated from existing actor scheduling to respect single responsibility.

**Components**:
- `scheduler.rs`: `TagScheduler` — manages per-projection polling intervals, tag stream dispatch respecting `concurrency_per_tag`
- `batch_executor.rs`: orchestrates `ReadSideStore.fetch()` → dedup filter → `ReadSideSession` creation → handler execution → atomic commit
- `backpressure.rs`: enforces `max_in_flight` global semaphore

---

## 13. Commit Boundary Semantics

**Decision**: The atomic unit of work is metadata state only — `OffsetStore` update and `DedupStore` `mark_seen`. Handler side effects (external DB writes, API calls) are explicitly excluded from the transaction boundary.

**Rationale**: Handler side effects are external and cannot be rolled back by the runtime (e.g., a sent email cannot be unsent). Including them in the transaction scope would require distributed transaction support (XA, 2PC), adding unacceptable complexity for a library-level runtime. The runtime guarantees offset + dedup atomicity; handler side effects are best-effort from the runtime's perspective.

**Alternative considered**: Full distributed transaction scope — rejected because it would couple the runtime to infrastructure capabilities that not all backends support, violating FR-013 (interchangeable backends).

---

## 14. Failure Semantics

**Decision**: Two distinct failure scenarios:
- **Before atomic commit**: The full batch MAY be retried — no metadata has been persisted, so replaying the batch is idempotent-safe.
- **After handler success but before atomic commit**: The handler has completed but offset + dedup were not persisted. On next fetch, the same events will be fetched again, but `DedupStore::seen()` returns true, so they are filtered out before handler invocation. No duplicates reach the handler.

**Rationale**: This design eliminates the need for distributed transactions while maintaining exactly-once processing semantics from the handler's perspective. The dedup store serves as the safety net for the window between handler completion and commit persistence.

---

## 15. Runtime State Machine

**Decision**: Each projection transitions through 5 explicit states: RUNNING, REPLAYING, REBUILDING, PAUSED, FAILED. Transitions are defined at the `ReadSideRunner` level.

**States**:
- `RUNNING` — normal processing: fetch → batch → execute → commit loop
- `REPLAYING` — reprocessing all events from beginning; dedup ON by default (configurable OFF)
- `REBUILDING` — clears read model, offsets, and dedup state, then full replay
- `PAUSED` — processing suspended; projection accepts no new batches
- `FAILED` — terminal state after unrecoverable error

**Transitions**:
- RUNNING → REPLAYING: `ReadSideRunner::replay()` call
- RUNNING → REBUILDING: `ReadSideRunner::rebuild()` call
- RUNNING → PAUSED: manual pause API or transient threshold exceeded
- PAUSED → RUNNING: manual resume API
- RUNNING → FAILED: `ProjectionError::Fatal` or unrecoverable runtime error
- REPLAYING → RUNNING: automatic on completion
- REBUILDING → RUNNING: automatic on completion

**Rationale**: Explicit states prevent ambiguous runtime behavior (e.g., should a replay request be accepted while a rebuild is in progress?). State transitions enforce guard conditions that would otherwise require ad-hoc checks throughout the codebase. The FAILED state provides a clear terminal signal for operators.

---

## 16. ProgressReporter Observability Pattern

**Decision**: Observability is provided through a `ProgressReporter` trait in the domain crate. The runtime calls trait methods at key lifecycle points. The host application injects the concrete implementation at runner construction.

**Trait methods**:
- `on_batch_completed(projection_id, tag, count, offset)` — called after each successful batch commit
- `on_error(projection_id, error)` — called on transient, fatal, and poison event errors
- `on_state_transition(projection_id, from, to)` — called on every state change

**Rationale**: A trait-based approach keeps the domain crate runtime-neutral (no logging framework dependency, no metrics library coupling) while giving the host full flexibility — a single implementation can log, emit metrics, or both. This follows the existing SPI pattern used by OffsetStore and DedupStore.

**Alternatives considered**:
- Log-based only — rejected because it couples domain to a specific logging framework.
- Channel-based (tokio::mpsc) — rejected because it introduces async into the domain crate.
- Metrics-only — rejected because it loses per-event error context needed for debugging.
