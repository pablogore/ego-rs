# Data Model: Read Side Projections

## Entities

### EventStreamElement

The universal consumption unit for the read side. Immutable snapshot of a stored event plus precomputed metadata.

| Field | Type | Description | Validation |
|-------|------|-------------|------------|
| `event_id` | String | Globally unique event identifier | Non-empty, unique across store |
| `aggregate_id` | String | The aggregate that produced this event | Non-empty |
| `tenant_id` | String | Multi-tenant scope | Non-empty |
| `event_type` | String | Discriminant for routing (e.g., "OrderPlaced") | Non-empty |
| `payload` | E | The event data (generic) | Backend-dependent serialization |
| `event_version` | i64 | Monotonic version within tag stream | >= 1, may have gaps |
| `occurred_at` | DateTime<Utc> | Wall-clock timestamp | Must be valid UTC |
| `tags` | Vec<EventTag> | Precomputed partition keys | At least one tag required |

**Immutability**: Once created, all fields are read-only. Handlers receive shared references.

---

### EventTag

A partition key defining a logical event stream.

| Field | Type | Description |
|-------|------|-------------|
| `value` | String | The tag value (e.g., "order-123", "payment") |

**Rules**:
- Tags are precomputed by `EventTagger` at event creation time (see research.md §1)
- Runtime never recalculates tags
- An event belongs to all tag streams it is tagged with (fan-out)
- Ordering is guaranteed ONLY within the same tag value

---

### Offset

Tracks how far a projection has progressed within a tag stream.

| Variant | Payload | Semantics |
|---------|---------|-----------|
| `Sequence(i64)` | last confirmed event_version | Resume from `offset + 1` after restart |

**Rules**:
- Only `Sequence` variant is allowed (FR-014)
- Represents the **last confirmed** event_version post-atomic-commit (Q5 clarification)
- Monotonically increasing within a tag + projection scope
- Independent per (projection_id, tag, tenant)

---

### ProgressReporter

Trait-based callback interface for runtime observability. Host injects implementation at runner construction.

| Method | Signature | Description |
|--------|-----------|-------------|
| `on_batch_completed` | `(projection_id, tag, count, offset)` | Called after each successful batch commit |
| `on_error` | `(projection_id, error)` | Called on transient, fatal, and poison event errors |
| `on_state_transition` | `(projection_id, from, to)` | Called on every state change |

**Implementation contract**: All methods MUST be non-blocking (or as fast as possible). Implementations MAY log, emit metrics, or both. The runtime MUST NOT depend on the reporter for correctness — it is observability-only.

---

### ReadSideConfig

Runtime configuration for backpressure and concurrency control.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `batch_size` | usize | 20 | Max events per batch delivered to handler |
| `max_in_flight` | usize | 10 | Max concurrent batch operations globally |
| `concurrency_per_tag` | usize | 4 | Max concurrent tag streams per projection |

**Enforcement**:
- `batch_size` — runtime MUST split larger event sets into multiple batches
- `max_in_flight` — global throttle across all projections
- `concurrency_per_tag` — per-projection tag stream dispatch limit

---

### ProjectionError

Classification of handler failures.

| Variant | Runtime Action |
|---------|----------------|
| `Transient` | Retry batch with exponential backoff (max 3 retries, 100ms base, 10s max) |
| `Fatal` | Stop projection immediately, raise alert |
| `PoisonEvent` | Log and skip the offending event, continue processing rest of batch |

---

### ReadSideStore

Read-optimized event store interface for tag-based projection consumption. Separate from `EventStore`.

| Method | Signature | Description |
|--------|-----------|-------------|
| `fetch` | `(tag, offset, batch_size) → Vec<EventStreamElement>` | Fetch up to `batch_size` events for a tag starting after `offset` |

**Rules**:
- Returns events with `event_version > offset` in ascending version order
- If offset is `None`, returns from the beginning (used in replay)
- Guaranteed ordering by `event_version` within a single tag
- Does NOT expose EventStore internals to the read side

---

### ReadSideSession

Execution context for a single batch. Groups events, executes handler, persists offset, commits dedup. Atomic scope is metadata only (offset + dedup) — handler side effects excluded.

| Field | Type | Description |
|-------|------|-------------|
| `events` | `Vec<EventStreamElement>` | The batch of events to process |
| `offset_store` | `&dyn OffsetStore` | Store for reading/writing offsets per tag |
| `dedup_store` | `&dyn DedupStore` | Store for checking/marking seen event IDs per tag |

**Lifecycle**:
1. Fetch events for tag up to `batch_size`
2. Filter out dedup'd events
3. Create session with filtered batch
4. Execute handler (receives `&[EventStreamElement]`)
5. Commit: offset persistence + dedup state atomically (metadata only — handler side effects excluded from transaction boundary)

**Failure semantics**:
- Failure before commit → full retry allowed (no metadata persisted)
- Failure after handler success but before commit → dedup prevents duplicates on next fetch

---

## Relationships

```
protobuf event (buf generated)
    │
    │ ego-event-adapter: protobuf_to_ce
    ▼
CloudEvent (standard envelope)
    │
    │ ego-event-adapter: ce_to_eventstore
    ▼
EventStore (write-optimized log)
    │
    │ (write path — outside CORE-005)
    │
    ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
    │
    │ ReadSideStore.fetch(tag, offset, batch_size)
    ▼
EventStreamElement[]  (from ego-event-adapter: eventstore_to_ese)
    │
    │ apply EventTagger (precomputed tags)
    ▼
(fan-out by tag) Vec<EventStreamElement>[]
    │
    │ batch by batch_size
    ▼
Batch (up to batch_size elements)
    │
    │ dedup filter (per projection_id, tag, event_id)
    ▼
ReadSideSession
    │
    │ handler execution
    ▼
Handler → ReadModel (user-managed)
    │
    │ ProgressReporter.on_batch_completed
    │ ProgressReporter.on_error
    │ ProgressReporter.on_state_transition
    ▼
(atomically) offset + dedup  (metadata only — handler side effects excluded)
```

## State Transitions

### Runtime (Projection Lifecycle)

```
                    ┌─────────────────────────────────────┐
                    │              RUNNING                 │
                    └──┬──────────┬──────────┬─────────────┘
                       │          │          │
                       ▼          ▼          ▼
                 ┌─────────┐ ┌──────────┐ ┌───────┐
                 │REPLAYING│ │REBUILDING│ │PAUSED │
                 └────┬────┘ └────┬─────┘ └───┬───┘
                      │           │            │
                      └───────┬───┘            │
                              │                │
                              ▼                │
                         ┌────────┐            │
                         │ FAILED │◄───────────┘
                         └────────┘
```

**Transitions**:
- RUNNING → REPLAYING: `ReadSideRunner::replay()` call
- RUNNING → REBUILDING: `ReadSideRunner::rebuild()` call
- RUNNING → PAUSED: manual pause or transient threshold exceeded
- PAUSED → RUNNING: manual resume
- RUNNING → FAILED: `ProjectionError::Fatal` or unrecoverable runtime error
- REPLAYING → RUNNING: automatic on completion
- REBUILDING → RUNNING: automatic on completion

### Tag Stream Lifecycle
```
INITIAL (no offset)
  │
  │ fetch events from beginning
  ▼
PROCESSING (offset = N)
  │
  │ normal fetch from offset+1
  ▼
PROCESSING (offset = M, M > N)
  │
  ├── replay triggered → ignore offset, fetch from beginning
  │
  └── rebuild triggered → clear all state, replay from beginning
```

### Batch Execution
```
READSIDESTORE_FETCH → TAG_GROUP → DEDUP_FILTER → SESSION_CREATE → HANDLER_EXEC → ATOMIC_COMMIT
                                          │
                                          ├── Transient → RETRY (up to 3×)
                                          ├── Fatal → STOP
                                          └── PoisonEvent → SKIP & CONTINUE
```
