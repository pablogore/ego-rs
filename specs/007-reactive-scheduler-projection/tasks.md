# Tasks: CORE-007 Reactive Scheduler & Deterministic Projection Engine

**Input**: Design documents from `specs/007-reactive-scheduler-projection/`
**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md, contracts/

**Tests**: Included — TDD mandatory per constitution (coverage >= 85%, deterministic tests, mock-based isolation)

**Organization**: Tasks grouped by user story for independent implementation and testing. MVP = US1 only.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Maps to user story (US1-US6 from spec.md §7)
- Exact file paths included in all task descriptions

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create crate skeleton and register in workspace

- [x] T001 Create crate directory structure `crates/ego-scheduler/src/`
- [x] T002 [P] Create `crates/ego-scheduler/Cargo.toml` with dependencies: `ego-domain`, `tokio` (sync, mpsc, Notify), `tracing`, `thiserror`, `sha2`
- [x] T003 [P] Register `ego-scheduler` in workspace `Cargo.toml` members, `layers.toml` as `foundation`, and `scripts/verify-layers.sh`

**Checkpoint**: Crate builds with `cargo build -p ego-scheduler`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core types and traits that ALL user stories depend on

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [x] T004 Define `SchedulerError` enum in `crates/ego-scheduler/src/error.rs` (using `thiserror::Error`)
- [x] T005 [P] Define core types in `crates/ego-scheduler/src/event_bus.rs`: `EntityTriple` (tenant, entity_type, entity_id — derive Hash, Eq, PartialEq, Ord for deterministic sorting), `SchedulerEvent` enum (ExecutionCompleted, RecoveryCompleted — each with entity + state_version), `EventType` enum (derive PartialEq, Eq), `SchedulerEventEnvelope` struct (event_id: [u8;32], sequence_id: u64, event_type, payload, source_actor), `BusItem` struct (sequence: u64, event: SchedulerEventEnvelope)
- [x] T006 [P] Define `SchedulingPolicy` trait in `crates/ego-scheduler/src/policy.rs` per `contracts/scheduling-policy.md` — pure function, deterministic, bounded time, returns Option<EntityTriple>. Signature: `fn suggest_activation(&self, state: &SchedulerState, pending: &BTreeSet<EntityTriple>) -> Option<EntityTriple>`. Document allowed fields (total_events_consumed, last_suggestion), forbidden fields (replay_buffer, detected_gaps, last_sequence_id, state_hash), and advisory-only semantics (I3/I7). BTreeSet ensures deterministic iteration — HashSet is forbidden for scheduling decisions
- [x] T007 [P] Define `SchedulerState` struct in `crates/ego-scheduler/src/state.rs` — single-stream model, pure data container. Semantic fields: `total_events_consumed: u64`, `last_sequence_id: Option<u64>` (per-actor scoped), `detected_gaps: u64` (per-actor scoped), `last_suggestion: Option<EntityTriple>`, `state_hash: Option<[u8;32]>`. Diagnostic: `replay_buffer: VecDeque<(u64, SchedulerEvent)>` bounded at 1024. Implement `apply()` — pure projection from event: `(Event, SchedulerState) → SchedulerState`. `apply()` does NO entity switch detection, NO reset logic beyond field updates. Entity switch detection and per-entity field resets are performed by Scheduler BEFORE calling `apply()`. Manually implement PartialEq to exclude `replay_buffer` (I4)
- [x] T008 Wire module declarations and public API re-exports in `crates/ego-scheduler/src/lib.rs` — declare modules: `error`, `event_bus`, `policy`, `state`, `scheduler` (with sub-modules `ingest`, `route`, `reduce`, `detect`, `evaluate`, `emit`), `metric`, `gap`

**Checkpoint**: Foundation ready — all types and traits compile; `cargo build -p ego-scheduler` passes

---

## Phase 3: User Story 1 — Reactive Scheduling (Priority: P1) 🎯 MVP

**Goal**: Events consumed → SchedulerState updated → SchedulingPolicy evaluated → advisory Suggestion produced. Determinism: two instances fed identical observed streams produce identical state (I1). Single-stream model: per-entity tracking resets on entity switch (I2).

**Independent Test**: `cargo test -p ego-scheduler -- test_deterministic_projection`

### Tests for User Story 1

> **TDD**: Write these FIRST, ensure they FAIL before implementation

- [x] T009 [P] [US1] Deterministic projection integration test — two instances fed identical observed streams → identical semantic SchedulerState (I1) in `crates/ego-scheduler/tests/determinism.rs`
- [x] T010 [P] [US1] SchedulingPolicy contract test — empty set → None, deterministic output, returns member, no side effects, does NOT read forbidden fields (I7) in `crates/ego-scheduler/src/policy.rs` `#[cfg(test)]` module
- [x] T011 [P] [US1] Scheduler integration test — event loop produces advisory suggestion, total_events_consumed increments in `crates/ego-scheduler/tests/scheduler.rs`

### Implementation for User Story 1

- [x] T012 [US1] Implement `RoundRobin` policy in `crates/ego-scheduler/src/policy.rs` — BTreeSet provides deterministic iteration order (no manual sorting needed). Index by `state.total_events_consumed % pending.len()`. Event-driven fairness: cursor advances on every consumed event. Document skewed-distribution behavior. Must be pure, deterministic, O(pending)
- [x] T013 [US1] RoundRobin determinism property test — 1000 random (state, pending) pairs produce identical output in `crates/ego-scheduler/tests/round_robin.rs`
- [x] T014 [US1] Implement Scheduler pipeline in `crates/ego-scheduler/src/scheduler/` — 6 pure components composed by thin orchestrator `crates/ego-scheduler/src/scheduler.rs`:
  - `ingest.rs`: EventIngestor — drains event bus only, returns Vec<BusItem>, no logic
  - `route.rs`: EntityRouter — detects entity switch (`current_active_entity != event.source_actor`); on switch, resets per-entity fields externally before passing to reducer
  - `reduce.rs`: StateReducer — wraps `SchedulerState::apply()`, pure function, no branching
  - `detect.rs`: GapDetector — structural only: `sequence_id != last + 1` → increment `detected_gaps`, no policy interaction
  - `evaluate.rs`: PolicyEvaluator — calls `SchedulingPolicy::suggest_activation(state, pending)` where pending is BTreeSet, no side effects
  - `emit.rs`: SuggestionEmitter — writes `last_suggestion` only, no logic
  Scheduler (`scheduler.rs`) is composition-only: calls pipeline stages in order, no business logic
- [x] T015 [US1] Implement `total_events_consumed` counter increment and advisory suggestion output in `crates/ego-scheduler/src/metric.rs` using `tracing` macros (info/debug level)

**Checkpoint**: US1 fully functional — `cargo test -p ego-scheduler` passes determinism, RoundRobin, scheduler tests. MVP ready.

---

## Phase 4: User Story 2 — Control Plane Isolation (Priority: P1)

**Goal**: CORE-006 execution path is independent of CORE-007 output. Suggestions are advisory only. I3 invariant upheld. Execution authority belongs exclusively to CORE-006.

**Independent Test**: `cargo test -p ego-scheduler -- test_advisory_only` + compile-time verification

- [x] T016 [P] [US2] Integration test: advisory-only output — verify Scheduler never blocks or modifies CORE-006 execution path in `crates/ego-scheduler/tests/advisory_only.rs`
- [x] T017 [P] [US2] Compile-time verification: CORE-006 crates (`crates/runtime/`, `crates/persistent-entity/`, `crates/domain/`) contain zero `use ego_scheduler` imports. Verify with `rg "ego.scheduler" crates/runtime crates/persistent-entity crates/domain/` returning empty
- [x] T018 [US2] Document I3 invariant enforcement — SchedulingPolicy output is NEVER a command, Scheduler MUST NOT influence execution directly or indirectly. Verify in code review: no CORE-006 code reads CORE-007 output for control decisions. `suggest_activation` return type is `Option<EntityTriple>`, not a command or message

**Checkpoint**: Control plane isolation verified — CORE-006 has zero dependency on CORE-007

---

## Phase 5: User Story 3 — Per-Actor Ordering Only (Priority: P1)

**Goal**: `sequence_id` values never compared across entities. `pending` is unordered. Policy selects by entity identity only. No cross-entity ordering exists or is inferred. Single-stream model: SchedulerState tracks one entity at a time.

**Independent Test**: `cargo test -p ego-scheduler -- test_per_entity_ordering`

- [x] T019 [P] [US3] Integration test: no cross-entity sequence_id comparison in `crates/ego-scheduler/tests/per_entity_ordering.rs` — verify zero cross-entity sequence_id comparisons exist in source (static analysis: `rg "sequence_id"` across all code paths must never compare values from different entities)
- [x] T020 [US3] Verify `pending` set uses deterministic iteration — confirm `BTreeSet<EntityTriple>` usage; policy selection by entity identity only, never by cross-entity sequence_id in `crates/ego-scheduler/src/policy.rs` and `crates/ego-scheduler/src/scheduler.rs`
- [x] T021 [P] [US3] Integration test: entity stream isolation — mixed-entity streams produce same per-entity state as isolated streams; verify SchedulerState per-entity tracking resets on entity switch in `crates/ego-scheduler/tests/entity_isolation.rs`

**Checkpoint**: Per-actor ordering invariant enforced — zero cross-entity comparisons, streams fully isolated, state resets on switch

---

## Phase 6: User Story 4 — Backpressure (Priority: P2)

**Goal**: Bounded event bus (4096) with configurable DropPolicy. Single-consumer, multi-producer (I6). DropNewest drops without blocking Actor. Block prevents loss. DropPattern deterministic under identical arrival order (I5). Scheduler owns receiver exclusively.

**Independent Test**: `cargo test -p ego-scheduler -- test_backpressure_block`

- [x] T022 [P] [US4] Define `DropPolicy` enum in `crates/ego-scheduler/src/event_bus.rs` — Block (default, high-water mark at 90%), DropNewest (counter incremented), DropOldest (oldest evicted). Doc-comment: all variants fully deterministic per I5
- [x] T023 [P] [US4] Unit test: DropPolicy determinism in `crates/ego-scheduler/src/event_bus.rs` `#[cfg(test)]` module — same arrival order + same policy → same dropped events
- [x] T024 [US4] Implement bounded event bus in `crates/ego-scheduler/src/event_bus.rs` using `tokio::sync::mpsc::channel(4096)`. `SchedulerEventSender` with `try_send()` returning `Result<(), SendError>` — must derive `Clone` (multi-producer, I6). `try_send()` is fire-and-forget: `SendError` is final, no retry logic. Each `try_send` is atomic per-event (no batch send). DropPolicy applies strictly at enqueue time. `SchedulerEventReceiver` with `drain_all()` returning `Vec<BusItem>` — single consumer, no Clone. `SchedulerTrigger` wrapping `tokio::sync::Notify` for async wakeup. Factory functions `event_bus_channel()` and `event_bus_channel_with_config(capacity, policy)`. Dropping receiver closes channel (I6 lifecycle). No retry orchestration in Scheduler — it only drains
- [x] T025 [P] [US4] Integration test: Block backpressure — sender blocks at capacity, all events consumed in `crates/ego-scheduler/tests/backpressure_block.rs`
- [x] T026 [P] [US4] Integration test: DropNewest backpressure — drops newest without blocking, deterministic drop pattern, drop counter increments in `crates/ego-scheduler/tests/backpressure_drop_newest.rs`
- [x] T027 [US4] Integration test: DropPolicy determinism under varying load — same arrival order produces same drops regardless of load/concurrency timing in `crates/ego-scheduler/tests/drop_policy_determinism.rs`

**Checkpoint**: Backpressure fully functional — all 3 policies working, I6 enforced, determinism verified under load

---

## Phase 7: User Story 5 — Diagnostic Replay (Priority: P3)

**Goal**: Replay buffer bounded to 1024. Diagnostic inspection only — never reconstruction. ReplayBuffer differences MUST NOT affect state equivalence (I4). PartialEq excludes replay_buffer.

**Independent Test**: `cargo test -p ego-scheduler -- test_replay_buffer_bounded`

- [x] T028 [P] [US5] Integration test: replay buffer bounded at 1024 — oldest events evicted on overflow in `crates/ego-scheduler/tests/replay_buffer.rs`
- [x] T029 [US5] Wire ReplayBuffer push on each event consumed in `crates/ego-scheduler/src/scheduler.rs` `apply_events()` — push (sequence_id, event) to state.replay_buffer; evict oldest if len > 1024
- [x] T030 [P] [US5] Integration test: ReplayBuffer non-semantic — verify zero code paths for state reconstruction, determinism validation, or recovery using ReplayBuffer in `crates/ego-scheduler/tests/replay_buffer_non_semantic.rs`
- [x] T031 [US5] Integration test: ReplayBuffer equivalence — two SchedulerState instances with identical semantic fields but different replay buffer contents MUST be equal via PartialEq (I4) in `crates/ego-scheduler/tests/replay_buffer_equivalence.rs`

**Checkpoint**: Replay buffer diagnostic-only — bounded, non-semantic, PartialEq excludes it, equivalence preserved

---

## Phase 8: User Story 6 — Gap Detection (Priority: P3)

**Goal**: Gaps detected per-actor when consumed `sequence_id != last + 1` within current entity's stream segment. Uniform treatment — no gap-type classification. Metrics exposed. System continues under gaps. No recovery attempted.

**Independent Test**: `cargo test -p ego-scheduler -- test_gap_detection`

- [x] T032 [P] [US6] Define `GapInfo` struct in `crates/ego-scheduler/src/gap.rs` — start_seq: u64, end_seq: u64, source_actor: EntityTriple. Per-actor scoped only, no cross-entity gap inference
- [x] T033 [P] [US6] Unit test: gap detection logic — missing sequence_ids detected per-actor within current entity stream; no cross-entity gap inference; state resets gap tracking on entity switch in `crates/ego-scheduler/src/gap.rs` `#[cfg(test)]` module
- [x] T034 [US6] Implement gap detection in `crates/ego-scheduler/src/gap.rs` — `detect_gap(last_seq: u64, current_seq: u64) -> Option<GapInfo>`. Called per event; increments `state.detected_gaps` on detection. Uniform treatment: no gap-type classification, no per-cause metrics (DropPolicy loss vs sequence discontinuity are indistinguishable per clarified uniform-treatment model). No recovery, no reconciliation, no signals.
- [x] T035 [US6] Integration test: gap detection — system continues under gaps, detected_gaps increments per-actor, uniform treatment (no per-cause classification) in `crates/ego-scheduler/tests/gap_detection.rs`
- [x] T036 [US6] Expose `detected_gaps` observable in `crates/ego-scheduler/src/metric.rs` using `tracing::debug` for gap events. Single counter only — no per-cause attribution

**Checkpoint**: Gap detection functional — per-actor gaps detected, uniform treatment, metrics exposed, no recovery attempted

---

## Phase 9: Polish & Cross-Cutting Concerns

**Purpose**: Observability, concurrency validation, invariant verification, final checks

- [x] T037 [P] Implement observability metrics in `crates/ego-scheduler/src/metric.rs` — `total_events_consumed` gauge, `detected_gaps` counter, `last_suggestion` info log, DropPolicy drop counter (DropNewest/DropOldest only). All via `tracing` macros.
- [x] T038 [P] Concurrency equivalence test: concurrent drain produces same SchedulerState as sequential drain in `crates/ego-scheduler/tests/concurrency.rs` — validates I1 (concurrency is implementation detail, single-threaded equivalence). No dependency on Tokio ordering semantics for correctness.
- [x] T039 [P] Integration test: no-polling verification — code review confirms zero polling loops exist in `crates/ego-scheduler/src/` (static analysis: no `loop {}`, `while {}`, `tokio::time::interval`, `sleep` in scheduler)
- [x] T040 Verify CORE-006 files unmodified — `git diff main...HEAD -- crates/runtime/ crates/persistent-entity/ crates/domain/` returns empty
- [x] T041 Run full quickstart validation per `specs/007-reactive-scheduler-projection/quickstart.md` — all 9 scenarios pass
- [x] T042 Coverage check — `cargo test -p ego-scheduler --coverage` (or tarpaulin) must reach >= 85%. Address gaps.
- [x] T043 Verify all 7 invariants (I1-I7) enforced — determinism, per-entity ordering, no execution authority, ReplayBuffer non-semantic, deterministic DropPolicy, single-consumer bus, policy field access

**Checkpoint**: CORE-007 complete — all 7 invariants verified, all quickstart scenarios pass, coverage >= 85%

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — start immediately
- **Foundational (Phase 2)**: Depends on Setup (Phase 1) — BLOCKS all user stories
- **User Story 1 (Phase 3)**: Depends on Foundational (Phase 2) — 🎯 MVP
- **User Story 2 (Phase 4)**: Depends on US1 (Phase 3) — verifies properties of US1 implementation
- **User Story 3 (Phase 5)**: Depends on US1 (Phase 3) — verifies properties of US1 implementation; can run parallel to US2
- **User Story 4 (Phase 6)**: Depends on Foundational (Phase 2) — event bus types ready; can run parallel to US1
- **User Story 5 (Phase 7)**: Depends on US1 (Phase 3) — needs Scheduler and SchedulerState
- **User Story 6 (Phase 8)**: Depends on Foundational (Phase 2) — gap types ready; can run parallel to US1-US5
- **Polish (Phase 9)**: Depends on all user stories (Phase 3-8)

### User Story Dependencies

- **US1 (P1)**: Can start after Foundational — No dependencies on other stories
- **US2 (P1)**: Can start after US1 core implementation — Independent test possible
- **US3 (P1)**: Can start after US1 core implementation — Independent test possible. Runs parallel to US2.
- **US4 (P2)**: Can start after Foundational — Independent test possible. Runs parallel to US1.
- **US5 (P3)**: Can start after US1 — Independent test possible
- **US6 (P3)**: Can start after Foundational — Independent test possible. Runs parallel to US1-US5.

### Within Each User Story

- Tests MUST be written FIRST and FAIL before implementation (TDD)
- Types before implementation
- Core logic before wiring
- Story complete before moving to next priority

### Parallel Opportunities

- T002, T003 in Setup can run in parallel
- T005, T006, T007 in Foundational can run in parallel
- T009, T010, T011 (US1 tests) can run in parallel
- US4 and US6 can run in parallel with US1 (different source files, blocking only on Foundational)
- US2 and US3 can run in parallel (both verify US1 properties)
- T016, T017 in US2 can run in parallel
- T019, T021 in US3 can run in parallel
- All [P] tasks within each phase can run in parallel
- T037, T038, T039 in Polish can run in parallel

---

## Parallel Example: User Story 1

```bash
# Launch all US1 tests together (TDD — must fail first):
Task: "T009 Deterministic projection integration test in crates/ego-scheduler/tests/determinism.rs"
Task: "T010 SchedulingPolicy contract test in crates/ego-scheduler/src/policy.rs"
Task: "T011 Scheduler integration test in crates/ego-scheduler/tests/scheduler.rs"

# After tests fail, implement:
Task: "T012 RoundRobin policy in crates/ego-scheduler/src/policy.rs"
# Then dependent tasks (can run T013 and T015 in parallel):
Task: "T013 RoundRobin determinism property test in crates/ego-scheduler/tests/round_robin.rs"
Task: "T014 Scheduler in crates/ego-scheduler/src/scheduler.rs"
Task: "T015 Metrics counter in crates/ego-scheduler/src/metric.rs"
```

## Parallel Example: Phase 2 + US1 + US4 + US6

```bash
# After Foundational phase completes, launch in parallel:
# Developer A: US1 (MVP)
Task: "T009-T015 User Story 1 — Reactive Scheduling"

# Developer B: US4 (Backpressure)
Task: "T022-T027 User Story 4 — Backpressure"

# Developer C: US6 (Gap Detection)
Task: "T032-T036 User Story 6 — Gap Detection"
```

---

## Implementation Strategy

### MVP First (US1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL — blocks all stories)
3. Complete Phase 3: US1 — Reactive Scheduling
4. **STOP and VALIDATE**: `cargo test -p ego-scheduler` — determinism, RoundRobin, scheduler all pass
5. MVP is deployable: events consumed → state updated → policy evaluated → advisory suggestion

### Incremental Delivery

1. Setup + Foundational → Foundation ready
2. US1 → MVP: Reactive Scheduling with determinism (P1 🎯)
3. US2 + US3 → Property verification: control plane isolation + per-actor ordering (P1)
4. US4 → Backpressure with deterministic DropPolicy + single-consumer bus (P2) (I5, I6)
5. US5 → Diagnostic replay buffer (P3) (I4)
6. US6 → Gap detection — uniform treatment (P3)
7. Polish → Observability, concurrency tests, coverage, all 7 invariant verification

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: US1 (MVP core)
   - Developer B: US4 (Backpressure — independent of US1, needs event_bus.rs types only)
   - Developer C: US6 (Gap detection — independent of US1, needs gap.rs types only)
3. After US1:
   - Developer A: US2 + US3 (verification properties)
   - Developer B: US5 (needs Scheduler from US1)
4. All: Phase 9 Polish (all 7 invariants verified)

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story must be independently completable and testable
- TDD: Write tests first, ensure they FAIL, then implement
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- CORE-006 crates (`crates/runtime/`, `crates/persistent-entity/`, `crates/domain/`) must remain unmodified (I3)
- Concurrency is an implementation detail — correctness defined over sequential stream application
- No dependency on Tokio or async runtime ordering semantics for correctness
- SchedulerState PartialEq MUST exclude replay_buffer (I4)
- SchedulerEventSender MUST derive Clone (multi-producer, I6); SchedulerEventReceiver MUST NOT (single consumer)
- Policy MUST only read total_events_consumed and last_suggestion from state (I7)
- Gap detection is uniform — no per-cause classification
- RoundRobin is event-driven — cursor advances on every consumed event
