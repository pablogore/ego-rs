---

description: "Task list for CORE-005 Read Side Projections feature"
---

# Tasks: CORE-005 Read Side Projections

**Input**: Design documents from `specs/005-read-side-projections/`

**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: TDD per `.speckit/constitution.md`. Each story phase includes test tasks. Write tests FIRST (they MUST fail), then implement.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on sibling tasks)
- **[Story]**: Which user story this task belongs to (US1, US2, US3, US4, US5, US6)
- Include exact file paths in descriptions

## Path Conventions

- Crate root: `crates/<name>/src/`
- All paths relative to repository root (`/Users/pablogore/workspace/pablogore/ego-rs/`)
- Domain types/traits → `crates/domain/src/read_side/`
- In-memory backends → `crates/infrastructure/src/persistence/in_memory/`
- Postgres backends → `crates/infrastructure/src/persistence/postgres/`
- Event adapter → `crates/event-adapter/` (new crate)
- Runtime orchestration → `crates/runtime/src/read_side/`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Module declarations, workspace entries, and skeleton module files

- [ ] T001 Add `pub mod read_side;` to `crates/domain/src/lib.rs`
- [ ] T002 [P] Add `crates/event-adapter` to `[workspace].members` in `Cargo.toml`
- [ ] T003 [P] Create `crates/domain/src/read_side/mod.rs` with module declarations and re-exports
- [ ] T004 [P] Add `pub mod read_side;` to `crates/runtime/src/lib.rs`
- [ ] T005 [P] Create `crates/runtime/src/read_side/mod.rs` with module declarations

**Checkpoint**: Domain and runtime modules declared; workspace knows about event-adapter crate

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Domain types, SPI traits, and in-memory backends that ALL user stories depend on

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

### Types

- [ ] T006 [P] Create `EventStreamElement<E>` struct in `crates/domain/src/read_side/event_stream.rs` (fields: event_id, aggregate_id, tenant_id, event_type, payload, event_version, occurred_at, tags; immutable, generic `<E>`)
- [ ] T007 [P] Create `EventTag` struct in `crates/domain/src/read_side/event_tag.rs` (field: value: String; with Display, FromStr, PartialEq, Eq, Hash, Clone)
- [ ] T008 [P] Create `Offset` enum (Sequence(i64) variant only) in `crates/domain/src/read_side/offset.rs`
- [ ] T009 [P] Create `ProjectionError` enum (Transient, Fatal, PoisonEvent) in `crates/domain/src/read_side/error.rs` — implements `std::error::Error`
- [ ] T010 [P] Create `ReadSideConfig` struct in `crates/domain/src/read_side/config.rs` (fields: batch_size, max_in_flight, concurrency_per_tag; all with sensible defaults per data-model.md)
- [ ] T011 [P] Create `ProjectionState` enum (Running, Replaying, Rebuilding, Paused, Failed) in `crates/domain/src/read_side/state.rs` — with Serialize/Deserialize, Display, PartialEq, Clone
- [ ] T012 [P] Create `EventTagger<E>` trait with `fn tags(&self, event: &E, aggregate_id: &str) -> Vec<EventTag>` in `crates/domain/src/read_side/tagger.rs`

### Storage SPI Traits

- [ ] T013 [P] Create `ReadSideStore` trait in `crates/domain/src/read_side/store.rs` with `fn fetch(&self, tag: &EventTag, offset: Option<&Offset>, batch_size: usize) -> Result<Vec<EventStreamElement<E>>, Error>`
- [ ] T014 [P] Create `OffsetStore` trait in `crates/domain/src/read_side/offset.rs` with `fn read_offset(&self, projection_id: &str, tag: &EventTag, tenant: &str) -> Result<Option<Offset>, Error>` and `fn write_offset(&self, projection_id: &str, tag: &EventTag, tenant: &str, offset: &Offset) -> Result<(), Error>`
- [ ] T015 [P] Create `DedupStore` trait in `crates/domain/src/read_side/dedup.rs` with `fn seen(&self, projection_id: &str, tag: &EventTag, event_id: &str) -> Result<bool, Error>` and `fn mark_seen(&self, projection_id: &str, tag: &EventTag, event_id: &str) -> Result<(), Error>`
- [ ] T016 [P] Create `ProgressReporter` trait in `crates/domain/src/read_side/progress.rs` with methods: `on_batch_completed(projection_id, tag, count, offset)`, `on_error(projection_id, error)`, `on_state_transition(projection_id, from, to)` — all with default no-op impl

### In-Memory Backends

- [ ] T017 [P] Implement `InMemoryReadSideStore` in `crates/infrastructure/src/persistence/in_memory/read_side_store.rs` — stores `Vec<EventStreamElement>`, supports fetch by tag + offset with version ordering
- [ ] T018 [P] Implement `InMemoryOffsetStore` in `crates/infrastructure/src/persistence/in_memory/offset_store.rs` — HashMap-based, scoped by (projection_id, tag, tenant)
- [ ] T019 [P] Implement `InMemoryDedupStore` in `crates/infrastructure/src/persistence/in_memory/dedup_store.rs` — HashSet-based, scoped by (projection_id, tag, event_id)
- [ ] T020 [P] Implement `InMemoryProgressReporter` spy in `crates/infrastructure/src/persistence/in_memory/progress_reporter.rs` — records all calls in a thread-safe Vec for test assertions
- [ ] T021 [P] Export new in-memory backends from `crates/infrastructure/src/persistence/in_memory/mod.rs`
- [ ] T022 Update `crates/domain/src/read_side/mod.rs` to re-export all types and traits from the read_side module

**Checkpoint**: All domain types, SPI traits, and in-memory test backends exist and compile. User story implementations can proceed.

---

## Phase 3: User Story 1 — Define and Run a Tagged Projection (Priority: P1) 🎯 MVP

**Goal**: A developer defines a projection with tags + batch handler, and the runtime fetches events grouped by tag, assembles batches, and delivers them via `ReadSideSession` with metadata-atomic commit (offset + dedup)

**Independent Test**: Register handler for tag `"order"`, insert 2 tagged events, run projection, verify handler received both events in one batch (cargo test -p ego-domain test::read_side::basic_batch_delivery)

### Tests for User Story 1 ⚠️ (Write FIRST — MUST fail)

- [ ] T023 [P] [US1] Test: basic batch delivery — 5 events with tag `"test"` → handler receives single batch of 5 in `crates/domain/tests/read_side/us1_basic_batch.rs`
- [ ] T024 [P] [US1] Test: batch splitting — batch_size=10 with 25 events → handler invoked 3 times (10, 10, 5) in `crates/domain/tests/read_side/us1_batch_splitting.rs`
- [ ] T025 [P] [US1] Test: multi-tag fan-out — event with tags `["order", "payment"]` → handler invoked per tag stream in `crates/domain/tests/read_side/us1_multi_tag_fanout.rs`
- [ ] T026 [P] [US1] Test: processor registration with no events → run_once succeeds, handler not invoked in `crates/domain/tests/read_side/us1_empty_batch.rs`
- [ ] T027 [P] [US1] Test: ProgressReporter spy receives on_batch_completed after successful batch in `crates/domain/tests/read_side/us1_progress_reporter.rs`

### Implementation for User Story 1

- [ ] T028 [P] [US1] Create `Handler` trait in `crates/domain/src/read_side/handler.rs` — `fn handle(&self, events: &[EventStreamElement]) -> Result<(), ProjectionError>`
- [ ] T029 [P] [US1] Create `ReadSideProcessor` trait in `crates/domain/src/read_side/processor.rs` — methods: `processor_name(&self) -> &str`, `tags(&self) -> Vec<EventTag>`, `handler(&self) -> &dyn Handler<E>`
- [ ] T030 [US1] Create `ReadSideSession` in `crates/domain/src/read_side/session.rs` — holds events batch + offset store + dedup store + handler; lifecycle: create → handler_exec → commit (atomically persists offset + dedup — handler side effects excluded from transaction boundary per Session 4 clarification)
- [ ] T031 [US1] Create `ReadSideRunner` trait + default implementation in `crates/domain/src/read_side/runner.rs` — `fn run_once(&self, processor: &dyn ReadSideProcessor, reporter: &dyn ProgressReporter) -> Result<(), ProjectionError>`; fetches events per tag, assembles batches, creates ReadSideSession, executes handler, commits; initial state = Running
- [ ] T032 [US1] Wire module re-exports in `crates/domain/src/read_side/mod.rs` — export Handler, ReadSideProcessor, ReadSideSession, ReadSideRunner, ProjectionState, ProgressReporter

**Checkpoint**: US1 fully functional — developer can register a projection, run it, and receive events in metadata-atomic batches with progress callbacks. Tests pass. MVP deliverable.

---

## Phase 4: User Story 3 — Offset-Controlled Resumable Processing (Priority: P1) 🎯 MVP

**Goal**: The runtime tracks last processed offset per (projection_id, tag, tenant) and resumes from stored offset after restart

**Independent Test**: Process 10 events → store offset 10 → insert 5 more → "restart" with fresh runner → verify only 5 new events delivered (cargo test -p ego-domain test::read_side::offset_resume_after_restart)

### Tests for User Story 3 ⚠️ (Write FIRST — MUST fail)

- [ ] T033 [P] [US3] Test: offset-based resume — process batch, persist offset, simulate restart, new events correctly start from offset+1 in `crates/domain/tests/read_side/us3_offset_resume.rs`
- [ ] T034 [P] [US3] Test: offset monotonicity — offset writes for same (projection_id, tag, tenant) are monotonically increasing; writing a lower offset is rejected in `crates/domain/tests/read_side/us3_offset_monotonic.rs`
- [ ] T035 [P] [US3] Test: fresh projection (no offset) starts from beginning in `crates/domain/tests/read_side/us3_fresh_start.rs`

### Implementation for User Story 3

- [ ] T036 [US3] Add persistence logic to `ReadSideRunner` impl: read stored offset before fetch per tag, write offset as part of session commit
- [ ] T037 [US3] Add offset validation in runner layer — reject writes where new offset <= current offset (monotonicity enforcement)
- [ ] T038 [P] [US3] Implement `PostgresOffsetStore` in `crates/infrastructure/src/persistence/postgres/offset_store.rs` — SQL-based, transactional writes, scoped by (projection_id, tag, tenant) (skeleton)

**Checkpoint**: US3 complete — projections survive restarts and resume from correct offsets. Combined with US1, this is the full MVP.

---

## Phase 5: User Story 2 — Idempotent Processing with Deduplication (Priority: P2)

**Goal**: Duplicate event IDs are detected and skipped before handler invocation, ensuring idempotent processing

**Independent Test**: Insert events `["a", "b", "c"]` → run → insert `["a", "d", "e"]` → run → verify only `["d", "e"]` delivered (cargo test -p ego-domain test::read_side::dedup_skips_duplicates)

### Tests for User Story 2 ⚠️ (Write FIRST — MUST fail)

- [ ] T039 [P] [US2] Test: dedup skips duplicates — same event_id in different batches, second occurrence filtered in `crates/domain/tests/read_side/us2_dedup_skip.rs`
- [ ] T040 [P] [US2] Test: dedup across restarts — in-memory dedup lost (acceptable for testing), Postgres dedup survives restart in `crates/domain/tests/read_side/us2_dedup_persistence.rs`
- [ ] T041 [P] [US2] Test: dedup independent per projection — same event filtered in projection A but processed in projection B in `crates/domain/tests/read_side/us2_dedup_per_projection.rs`

### Implementation for User Story 2

- [ ] T042 [US2] Add dedup filter to `ReadSideSession::create` flow: check `DedupStore::seen()` for each event before including in handler batch; skipped events are logged
- [ ] T043 [US2] Add `DedupStore::mark_seen()` call to `ReadSideSession::commit` — atomically persisted alongside offset
- [ ] T044 [P] [US2] Implement `PostgresDedupStore` in `crates/infrastructure/src/persistence/postgres/dedup_store.rs` — SQL-based, scoped by (projection_id, tag, event_id) (skeleton)

**Checkpoint**: US2 complete — duplicate detection works, read model stays consistent across retries and restarts.

---

## Phase 6: User Story 4 — Replay and Rebuild (Priority: P2)

**Goal**: Developer can reprocess all events from beginning (replay) or reset + reprocess (rebuild) with explicit state machine transitions

**Independent Test**: Process events to offset 100 → trigger replay → handler receives all events from offset 1 (cargo test -p ego-domain test::read_side::replay_ignores_offsets)

### Tests for User Story 4 ⚠️ (Write FIRST — MUST fail)

- [ ] T045 [P] [US4] Test: replay ignores stored offsets and processes all events in `crates/domain/tests/read_side/us4_replay.rs`
- [ ] T046 [P] [US4] Test: rebuild clears read model, offsets, and dedup state before full replay in `crates/domain/tests/read_side/us4_rebuild.rs`
- [ ] T047 [P] [US4] Test: rerunning replay produces identical read model state (idempotent replay) in `crates/domain/tests/read_side/us4_replay_idempotent.rs`
- [ ] T048 [P] [US4] Test: state machine transitions — replay() transitions Running→Replaying→Running; rebuild() transitions Running→Rebuilding→Running; ProgressReporter.on_state_transition called for each hop in `crates/runtime/tests/read_side/us4_state_transitions.rs`

### Implementation for User Story 4

- [ ] T049 [US4] Implement `ReadSideRunner::replay()` — sets state to Replaying, clears offset trackers, fetches from beginning, processes all events with dedup ON by default (accepts `dedup: bool` flag to disable), transitions to Running on completion
- [ ] T050 [US4] Implement `ReadSideRunner::rebuild()` — sets state to Rebuilding, clears offsets + dedup state + handler-visible read model (via callback), then runs replay from scratch, transitions to Running on completion
- [ ] T051 [US4] Ensure new events arriving during Replaying/Rebuilding are queued and processed after state transitions back to Running

**Checkpoint**: US4 complete — replay and rebuild work correctly with explicit state machine transitions and ProgressReporter notifications.

---

## Phase 7: User Story 5 — Failure Classification and Retry (Priority: P3)

**Goal**: Handler errors are classified (Transient → retry, Fatal → stop projection + FAILED state, PoisonEvent → skip + continue). Failure semantics follow Session 4: before-commit → retry; after-handler-but-before-commit → dedup safety net.

**Independent Test**: Handler returns Transient on first call, success on second → verify 2 handler invocations and eventual commit (cargo test -p ego-domain test::read_side::transient_retry)

### Tests for User Story 5 ⚠️ (Write FIRST — MUST fail)

- [ ] T052 [P] [US5] Test: transient retry with exponential backoff — configurable base delay, max delay, max retries in `crates/domain/tests/read_side/us5_transient_retry.rs`
- [ ] T053 [P] [US5] Test: fatal error transitions projection to Failed state — on_state_transition(Running, Failed) called via ProgressReporter in `crates/domain/tests/read_side/us5_fatal_stop.rs`
- [ ] T054 [P] [US5] Test: poison event skips offending event, marks it as seen in DedupStore, rest of batch continues in `crates/domain/tests/read_side/us5_poison_skip.rs`
- [ ] T055 [P] [US5] Test: failure before commit allows full retry; failure after handler success but before commit is caught by dedup on next fetch in `crates/domain/tests/read_side/us5_failure_semantics.rs`

### Implementation for User Story 5

- [ ] T056 [US5] Add retry loop to batch execution in `ReadSideRunner`: on `ProjectionError::Transient`, retry up to `max_retries` (default 3) with exponential backoff (base 100ms, max 10s)
- [ ] T057 [US5] Add fatal error handling: on `ProjectionError::Fatal`, transition to Failed state, call `ProgressReporter::on_state_transition`, stop projection, skip remaining tags
- [ ] T058 [US5] Add poison event handling: on `ProjectionError::PoisonEvent`, log skipped event metadata, call `DedupStore::mark_seen()` for the offending event, continue processing rest of batch
- [ ] T059 [US5] Document failure semantics in `ReadSideSession` rustdoc: failure before commit allows retry; failure after handler success before commit is caught by dedup on next fetch

**Checkpoint**: US5 complete — production-grade error handling with state machine integration and documented failure semantics.

---

## Phase 8: User Story 6 — Concurrent Processing with Backpressure (Priority: P3)

**Goal**: Runtime respects concurrency_per_tag and max_in_flight limits to prevent resource exhaustion

**Independent Test**: Configure concurrency_per_tag=2, 4 tags with pending events → verify at most 2 processed simultaneously (cargo test -p ego-domain test::read_side::backpressure_enforced)

### Tests for User Story 6 ⚠️ (Write FIRST — MUST fail)

- [ ] T060 [P] [US6] Test: concurrency_per_tag limits simultaneous tag streams for one projection in `crates/domain/tests/read_side/us6_concurrency_per_tag.rs`
- [ ] T061 [P] [US6] Test: max_in_flight limits total in-flight batches across all projections in `crates/domain/tests/read_side/us6_max_in_flight.rs`
- [ ] T062 [P] [US6] Test: batch_size never exceeded even when more events available in `crates/domain/tests/read_side/us6_batch_size_limit.rs`

### Implementation for User Story 6

- [ ] T063 [P] [US6] Implement `TagScheduler` in `crates/runtime/src/read_side/scheduler.rs` — manages per-projection polling intervals, dispatches tag streams respecting `concurrency_per_tag`
- [ ] T064 [US6] Implement `backpressure.rs` in `crates/runtime/src/read_side/backpressure.rs` — global semaphore enforcing `max_in_flight`, acquire/release on batch start/complete
- [ ] T065 [P] [US6] Implement `BatchExecutor` in `crates/runtime/src/read_side/batch_executor.rs` — orchestrates ReadSideStore.fetch → dedup filter → session create → handler exec → atomic commit within backpressure constraints; calls ProgressReporter callbacks at each lifecycle point
- [ ] T066 [P] [US6] Export runtime read_side module from `crates/runtime/src/read_side/mod.rs` — expose TagScheduler, BatchExecutor

**Checkpoint**: US6 complete — backpressure and concurrency controls operational, preventing resource exhaustion.

---

## Phase 9: Polish & Cross-Cutting Concerns

**Purpose**: Production hardening, event adapter, and final validation

### Postgres Backends (production)

- [ ] T067 [P] Implement `PostgresReadSideStore` in `crates/infrastructure/src/persistence/postgres/read_side_store.rs` — SQL fetch by tag + offset, paginated, version-ordered
- [ ] T068 [P] Implement `PostgresProgressReporter` in `crates/infrastructure/src/persistence/postgres/progress_reporter.rs` — logs batch completions, errors, and state transitions to a `projection_events` table
- [ ] T069 [P] Export Postgres read-side backends from `crates/infrastructure/src/persistence/postgres/mod.rs`

### Event Adapter Crate

- [ ] T070 Create `crates/event-adapter/Cargo.toml` with dependencies on protobuf, cloudevents-sdk, ego-domain
- [ ] T071 [P] Implement `protobuf_to_ce` in `crates/event-adapter/src/protobuf_to_ce.rs` — maps buf-generated event → CloudEventBuilder (sets id, source, type, dataschema, time)
- [ ] T072 [P] Implement `ce_to_eventstore` in `crates/event-adapter/src/ce_to_eventstore.rs` — serializes CloudEvent to EventStore storage format
- [ ] T073 [P] Implement `eventstore_to_ese` in `crates/event-adapter/src/eventstore_to_ese.rs` — reconstructs EventStreamElement<ProtoEvent> from stored record, applies EventTagger + version normalization
- [ ] T074 [P] Implement tagger executor in `crates/event-adapter/src/tagger_exec.rs` — applies EventTagger at event creation time on write path

### Cross-Cutting

- [ ] T075 [P] Rustdoc audit: verify all public APIs in `crates/domain/src/read_side/` have rustdoc documentation per constitution
- [ ] T076 [P] Run full validation suite from `specs/005-read-side-projections/quickstart.md` — all 11 scenarios must pass
- [ ] T077 [P] Clean up dead code, unused imports, and Clippy warnings across all read_side modules

**Checkpoint**: Production-ready. All validation scenarios pass. Postgres backends operational.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Setup — **BLOCKS all user stories**
- **User Stories (Phases 3-8)**: All depend on Foundational phase completion
  - US1 (Phase 3): Depends on Foundational — **no cross-story dependencies**
  - US3 (Phase 4): Depends on US1 — builds on ReadSideSession commit mechanism
  - US2 (Phase 5): Depends on US1 — adds dedup filter to session flow
  - US4 (Phase 6): Depends on US1 + US3 + US2 — replay/rebuild uses all three stores + state machine
  - US5 (Phase 7): Depends on US1 — error handling wraps batch execution + state transitions
  - US6 (Phase 8): Depends on US1 — backpressure controls batch dispatch
- **Polish (Phase 9)**: Depends on all user stories being complete

### User Story Dependencies

```
Foundational ──▶ US1 (P1) ──▶ US3 (P1) ──▶ US4 (P2)
                   │            │
                   ├──▶ US2 (P2) ─────────▶ US4 (P2)
                   │
                   ├──▶ US5 (P3)
                   │
                   └──▶ US6 (P3)
```

### Within Each Phase

- Tests (marked [Story]) MUST be written and FAIL before implementation
- Types before traits
- Traits before backends
- Backends before runtime logic
- Implementation before integration
- Phase complete before starting next

### Parallel Opportunities

- All tasks marked [P] in same phase can run in parallel
- Phase 1 Setup: all 5 tasks [P] — independent module declarations
- Phase 2 Foundational: types T006-T012 and traits T013-T016 are [P] within their groups
- Phase 2 backends T017-T020 are [P] — independent in-memory stores
- User story implementation tasks marked [P] are independent (e.g., Handler and ReadSideProcessor traits)
- User story test tasks within a phase are [P]
- Phase 9 Polish tasks are [P]

---

## Parallel Example: Phase 2 Foundational

```bash
# Types (all [P] — different files):
Task: "Create EventStreamElement in crates/domain/src/read_side/event_stream.rs"
Task: "Create EventTag in crates/domain/src/read_side/event_tag.rs"
Task: "Create Offset in crates/domain/src/read_side/offset.rs"
Task: "Create ProjectionError in crates/domain/src/read_side/error.rs"
Task: "Create ReadSideConfig in crates/domain/src/read_side/config.rs"
Task: "Create ProjectionState in crates/domain/src/read_side/state.rs"
Task: "Create EventTagger in crates/domain/src/read_side/tagger.rs"

# Storage SPI Traits (all [P] — different files, depend on types):
Task: "Create ReadSideStore in crates/domain/src/read_side/store.rs"
Task: "Create OffsetStore in crates/domain/src/read_side/offset.rs"
Task: "Create DedupStore in crates/domain/src/read_side/dedup.rs"
Task: "Create ProgressReporter in crates/domain/src/read_side/progress.rs"

# In-memory backends (all [P] — independent stores):
Task: "Implement InMemoryReadSideStore in .../read_side_store.rs"
Task: "Implement InMemoryOffsetStore in .../offset_store.rs"
Task: "Implement InMemoryDedupStore in .../dedup_store.rs"
Task: "Implement InMemoryProgressReporter spy in .../progress_reporter.rs"
```

## Parallel Example: Phase 3 US1

```bash
# Tests (ALL [P] — independent test scenarios):
Task: "Test: basic batch delivery in us1_basic_batch.rs"
Task: "Test: batch splitting in us1_batch_splitting.rs"
Task: "Test: multi-tag fan-out in us1_multi_tag_fanout.rs"
Task: "Test: empty batch in us1_empty_batch.rs"
Task: "Test: ProgressReporter spy in us1_progress_reporter.rs"

# Implementation (Handler + Processor + State [P], then Session + Runner sequential):
Task: "Create Handler trait"
Task: "Create ReadSideProcessor trait"
# Run after Handler + Processor:
Task: "Create ReadSideSession"
Task: "Create ReadSideRunner"
```

---

## Implementation Strategy

### MVP First (Phase 3 — US1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (**CRITICAL** — blocks everything)
3. Complete Phase 3: User Story 1 (core batch processing)
4. **STOP and VALIDATE**: Run US1 tests — basic batch delivery, batch splitting, multi-tag fan-out, progress reporter
5. Deploy/demo if ready (in-memory backend, single-run processing)

### MVP Plus (Phases 3 + 4 — US1 + US3)

6. Complete Phase 4: User Story 3 (offset persistence = resumable projection)
7. **STOP and VALIDATE**: All US1 + US3 tests pass
8. This is the minimum viable production projection engine

### Incremental Delivery (P2 Stories)

9. Add Phase 5: User Story 2 (dedup)
10. Add Phase 6: User Story 4 (replay/rebuild + state machine)
11. **STOP and VALIDATE**: All P1 + P2 tests pass

### Production Hardening (P3 Stories)

12. Add Phase 7: User Story 5 (error handling + FAILED state)
13. Add Phase 8: User Story 6 (backpressure/concurrency)
14. Add Phase 9: Polish (Postgres, event adapter, docs, validation)

### Parallel Team Strategy

With multiple developers:

1. **Team**: Phase 1 + Phase 2 together (quick — module declarations + types)
2. **Once Foundational is done**:
   - **Dev A**: Phase 3 (US1 — core batch processing + ProgressReporter)
   - **Dev B**: Phase 5 (US2 — dedup) — interfaces are already in Foundational
   - **Dev C**: Phase 7 (US5 — error handling) — builds on US1 session + state machine
3. **After US1 done**:
   - **Dev A**: Phase 4 (US3 — offset persistence)
   - **Dev B**: Phase 6 (US4 — replay/rebuild + state transitions) — needs US1 + US2 + US3
   - **Dev C**: Phase 8 (US6 — backpressure) — needs US1 session framework

---

## Notes

- [P] tasks = different files, no dependencies on sibling tasks in same phase
- [Story] label maps to user story for traceability
- Each user story independently testable via its own test file
- Write tests FIRST per `.speckit/constitution.md` TDD requirement
- All public APIs require rustdoc per constitution
- Domain crate must stay runtime-neutral (no async, no Tokio) — only runtime crate has async
- No protobuf, CloudEvents, or EventStore types in domain crate
- In-memory backends must be used in all unit/integration tests (no real databases)
- Session commit guarantees metadata atomicity only (offset + dedup) — handler side effects excluded per Session 4
- Failure semantics: before-commit → retry; after-handler-before-commit → dedup safety net
