# Read Side Projections — SPI Contracts

This directory documents the trait interfaces ("Service Provider Interfaces") that define the contract between the domain layer and infrastructure/runtime layers.

## Traits Overview

| Trait | Layer | Responsibility | File (domain) |
|-------|-------|---------------|---------------|
| `EventTagger<E>` | domain | Compute tags from event payload + aggregate ID | `read_side/tagger.rs` |
| `ReadSideStore` | domain | Fetch events by tag + offset for projection consumption | `read_side/store.rs` |
| `OffsetStore` | domain | Read/write offset per (projection_id, tag, tenant) | `read_side/offset.rs` |
| `DedupStore` | domain | Check/mark seen event IDs scoped by (projection_id, tag, event_id) | `read_side/dedup.rs` |
| `ProgressReporter` | domain | Callback for progress/error/state-transition observability | `read_side/progress.rs` |
| `ReadSideProcessor` | domain | Register tags + provide batch handler | `read_side/processor.rs` |
| `ReadSideRunner` | domain | Orchestrate fetch → session → commit loop | `read_side/runner.rs` |

## Contract Details

### EventTagger<E>

```
fn tags(event: &E, aggregate_id: &str) -> Vec<EventTag>
```

- **Input**: Event payload + aggregate_id
- **Output**: Zero or more tags for stream partitioning
- **Deterministic**: Same inputs → same tags (pure function)
- **Side effects**: None
- **Thread safety**: Required (shared across tag streams)

### ReadSideStore

```
fn fetch(tag: &EventTag, offset: Option<&Offset>, batch_size: usize) -> Result<Vec<EventStreamElement<E>>, Error>
```

- **Separation**: Independent from `EventStore` — read-optimized interface for tag-based streaming
- **Ordering**: Returns events in ascending `event_version` order within the tag
- **Offset semantics**: If `offset` is `Some`, returns events with `event_version > offset`; if `None`, returns from beginning (replay)
- **Backends**: InMemory (tests), Postgres (production)

### OffsetStore

```
fn read_offset(projection_id: &str, tag: &EventTag, tenant: &str) -> Result<Option<Offset>, Error>
fn write_offset(projection_id: &str, tag: &EventTag, tenant: &str, offset: &Offset) -> Result<(), Error>
```

- **Atomicity**: `write_offset` is part of the session commit transaction
- **Consistency**: Must be monotonic per (processor, tag, tenant)
- **Backend contract**: See research.md §5 for strictness guarantees

### DedupStore

```
fn seen(projection_id: &str, tag: &EventTag, event_id: &str) -> Result<bool, Error>
fn mark_seen(projection_id: &str, tag: &EventTag, event_id: &str) -> Result<(), Error>
```

- **Scope**: `(projection_id, tag, event_id)` — independent per projection per tag stream
- **Persistence**: MUST survive restarts for persistent backends (research.md §7)
- **Usage**: Called before handler execution; `mark_seen` is part of session commit

### ProgressReporter

```
fn on_batch_completed(projection_id: &str, tag: &EventTag, count: usize, offset: &Offset)
fn on_error(projection_id: &str, error: &ProjectionError)
fn on_state_transition(projection_id: &str, from: &ProjectionState, to: &ProjectionState)
```

- **Purpose**: Observability-only callback — runtime correctness MUST NOT depend on the reporter
- **Thread safety**: Methods MAY be called from concurrent tag streams; implementation must be thread-safe
- **Performance**: Methods MUST be non-blocking or fast — they are called inline in the hot path
- **Injection**: Host provides implementation at `ReadSideRunner` construction time
- **Default**: A no-op default implementation exists for convenience

### ReadSideProcessor

```
fn processor_name(&self) -> &str
fn tags(&self) -> Vec<EventTag>
fn handler(&self) -> &dyn Fn(&[EventStreamElement]) -> Result<(), ProjectionError>
```

### ProjectionState

```
enum ProjectionState {
    Running,
    Replaying,
    Rebuilding,
    Paused,
    Failed,
}
```

- **Representation**: Enum in domain crate, `Serialize + Deserialize` for persistence
- **Default**: Every projection starts in `Running` on first registration
- **Transitions**: Governed by `ReadSideRunner` — see runtime state machine in spec.md

### ReadSideRunner

```
fn run_once(processor: &dyn ReadSideProcessor) -> Result<(), ProjectionError>
fn replay(processor: &dyn ReadSideProcessor) -> Result<(), ProjectionError>
fn rebuild(processor: &dyn ReadSideProcessor) -> Result<(), ProjectionError>
```

- `run_once`: Single pass — fetch unprocessed events per tag, execute, commit
- `replay`: Ignore offsets, process all events, dedup ON by default (configurable OFF)
- `rebuild`: Clear read model + offsets + dedup state, then full replay from scratch
