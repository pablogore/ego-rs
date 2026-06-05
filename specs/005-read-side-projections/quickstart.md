# Quickstart: Read Side Projections

## Prerequisites

- Rust toolchain (edition 2021)
- `cargo test` working (no external dependencies required)

## Setup

```bash
cargo build -p ego-domain
cargo build -p ego-event-adapter   # protobuf → CloudEvent → EventStore adapter
cargo build -p ego-runtime          # polling + scheduling + session execution
```

## Validation Scenarios

### Scenario 1: Basic Batch Delivery

**Objective**: Verify that a registered projection receives events in batches.

1. Create an `InMemoryReadSideBackend`
2. Register a processor with tag `"test"` and a handler that records received batches
3. Insert 5 `EventStreamElement` instances with tag `"test"` into the event source
4. Call `ReadSideRunner::run_once`
5. **Expected**: Handler received a single batch with 5 events in insertion order

```bash
cargo test -p ego-domain test::read_side::basic_batch_delivery -- --nocapture
```

---

### Scenario 2: Deduplication

**Objective**: Verify that duplicate event IDs are filtered out.

1. Create backend with `InMemoryReadSideBackend`
2. Register a processor for tag `"test"`
3. Insert 3 events with IDs `["a", "b", "c"]`, tag `"test"`
4. Run `run_once` — handler receives 3 events
5. Insert 3 more events with IDs `["a", "d", "e"]`, tag `"test"`
6. Run `run_once` — handler receives 2 events (`"d"`, `"e"`); `"a"` is skipped

```bash
cargo test -p ego-domain test::read_side::dedup_skips_duplicates -- --nocapture
```

---

### Scenario 3: Offset-Based Resume

**Objective**: Verify that after restart, processing resumes from the correct offset.

1. Create backend, register processor, insert 10 events with tag `"test"`
2. Run `run_once` — handler receives all 10
3. Check stored offset: `Sequence(10)`
4. Insert 5 more events with tag `"test"`
5. Simulate restart: create fresh runner with same backend
6. Run `run_once` — handler receives only the 5 new events

```bash
cargo test -p ego-domain test::read_side::offset_resume_after_restart -- --nocapture
```

---

### Scenario 4: Replay

**Objective**: Verify replay ignores stored offsets.

1. Process events as in Scenario 3
2. Insert 5 new events (total 15)
3. Call `replay()` — handler receives all 15 events (ignores offset)
4. Stored offset is updated to latest after replay completes

```bash
cargo test -p ego-domain test::read_side::replay_ignores_offsets -- --nocapture
```

---

### Scenario 5: Rebuild

**Objective**: Verify rebuild clears all state and reprocesses from scratch.

1. Process events, verify offset and dedup state exist
2. Call `rebuild()` 
3. **Expected**: All offsets reset, dedup state cleared, handler invoked for all events from beginning
4. Post-rebuild, `run_once` starts from the new offset

```bash
cargo test -p ego-domain test::read_side::rebuild_clears_and_replays -- --nocapture
```

---

### Scenario 6: Multi-Tag Fan-Out

**Objective**: Verify events with multiple tags appear in each tag stream.

1. Register processor with tags `["order", "payment"]`
2. Insert event with tags `["order", "payment"]`
3. Run `run_once`
4. **Expected**: Handler invoked twice — once for `"order"` stream (with the event), once for `"payment"` stream (with the same event)

```bash
cargo test -p ego-domain test::read_side::multi_tag_fan_out -- --nocapture
```

---

### Scenario 7: Failure Handling

**Objective**: Verify transient retry, fatal stop, and poison event semantics.

1. Register a handler that returns `ProjectionError::Transient` on first call, success on second
2. Run `run_once`
3. **Expected**: Handler called twice (retry), second call succeeds, batch committed
4. Register a handler that returns `ProjectionError::Fatal`
5. **Expected**: Projection stops, no further tags processed
6. Register a handler with 3 events that returns `ProjectionError::PoisonEvent` for the middle event
7. **Expected**: First event processed, middle skipped + logged, third processed

```bash
cargo test -p ego-domain test::read_side::transient_retry -- --nocapture
cargo test -p ego-domain test::read_side::fatal_stops_projection -- --nocapture
cargo test -p ego-domain test::read_side::poison_event_skips_and_continues -- --nocapture
```

---

### Scenario 8: ReadSideStore Fetch Semantics

**Objective**: Verify that `ReadSideStore.fetch()` returns events ordered by version within a tag, respecting offset boundaries.

1. Insert events for tag `"order"` with versions `[1, 2, 3, 4, 5]`
2. Call `ReadSideStore.fetch("order", Some(Offset::Sequence(2)), 2)`
3. **Expected**: Returns events with versions `[3, 4]` (up to batch_size=2, starting after offset 2)
4. Call `ReadSideStore.fetch("order", None, 10)` (replay mode)
5. **Expected**: Returns events with versions `[1, 2, 3, 4, 5]` (all, ignoring offset)

```bash
cargo test -p ego-domain test::read_side::readside_store_fetch_semantics -- --nocapture
```

---

### Scenario 9: Runtime State Machine Transitions

**Objective**: Verify projection state transitions (RUNNING, REPLAYING, REBUILDING, PAUSED, FAILED).

1. Register processor, verify initial state is `Running`
2. Call `replay()` → verify state transitions to `Replaying`, then back to `Running` on completion
3. Call `rebuild()` → verify state transitions to `Rebuilding`, then back to `Running` on completion
4. Inject a fatal handler → verify state transitions to `Failed`
5. Verify that events arriving during `Replaying` or `Rebuilding` are queued and processed after completion

```bash
cargo test -p ego-runtime test::read_side::state_machine_transitions -- --nocapture
cargo test -p ego-runtime test::read_side::state_machine_replay_transition -- --nocapture
cargo test -p ego-runtime test::read_side::state_machine_rebuild_transition -- --nocapture
```

---

### Scenario 10: ProgressReporter Callbacks

**Objective**: Verify ProgressReporter is invoked at expected lifecycle points.

1. Register a `ProgressReporter` spy that records all calls
2. Run `run_once` with 3 events
3. **Expected**: `on_batch_completed(projection_id, tag, 3, Sequence(3))` called once
4. Trigger a fatal error from handler
5. **Expected**: `on_error(projection_id, Fatal(…))` and `on_state_transition(projection_id, Running, Failed)` called
6. Run `replay()` → **Expected**: `on_state_transition(projection_id, Running, Replaying)` then `(Replaying, Running)` on completion

```bash
cargo test -p ego-domain test::read_side::progress_reporter_batch -- --nocapture
cargo test -p ego-domain test::read_side::progress_reporter_error -- --nocapture
cargo test -p ego-domain test::read_side::progress_reporter_state_transition -- --nocapture
```

---

### Scenario 11: Backpressure

**Objective**: Verify concurrency limits are respected.

1. Configure `concurrency_per_tag = 2`, `max_in_flight = 3`
2. Register a processor with 4 tags, each with pending events and a slow handler (100ms sleep)
3. Run processing
4. **Expected**: No more than 2 tag streams processed simultaneously; no more than 3 batches in-flight globally

```bash
cargo test -p ego-domain test::read_side::backpressure_enforced -- --nocapture
```

## Test Patterns

All validation scenarios use in-memory backends (`InMemoryReadSideStore` + `InMemoryOffsetStore` + `InMemoryDedupStore`) — no external databases required. The `ProgressReporter` spy is injected to verify callback behavior. Tests run deterministically and offline, compliant with `.speckit/constitution.md`.
