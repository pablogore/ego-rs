---

description: "CORE-007 Reactive Scheduler & Deterministic Projection Engine implementation tasks"

---

# Tasks: CORE-007 Reactive Scheduler & Deterministic Projection Engine

**Input**: Design documents from `specs/007-reactive-scheduler-projection/`

**Prerequisites**: `plan.md` (required), `spec.md` (required for user stories), `research.md`, `data-model.md`, `contracts/`

**Tests**: Test tasks are included to meet the >= 85% coverage constitutional requirement. Write tests before implementation (Red-Green-Refactor).

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to
- Include exact file paths in descriptions

## Path Conventions

- Crate root: `crates/ego-scheduler/`
- Source: `crates/ego-scheduler/src/`
- Tests: inline in each module (`#[cfg(test)] mod tests { ... }`)

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create the `ego-scheduler` crate and register it in the workspace

- [ ] T001 Create `crates/ego-scheduler/Cargo.toml` with dependencies: `ego-domain`, `tokio` (sync features), `tracing`, `thiserror`
- [ ] T002 Register `crates/ego-scheduler` in workspace `Cargo.toml` members list
- [ ] T003 [P] Add `"ego-scheduler" = "foundation"` to `layers.toml`
- [ ] T004 [P] Create `crates/ego-scheduler/src/lib.rs` with public module declarations and doc comments
- [ ] T005 Verify the crate compiles: `cargo check -p ego-scheduler`

**Checkpoint**: `ego-scheduler` crate exists and compiles within the workspace.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core types, error types, and event bus that ALL user stories depend on

**⚠️ CRITICAL**: No user story can begin until this phase is complete

- [ ] T006 Create `crates/ego-scheduler/src/error.rs` with `SchedulerError` enum (bus full, gap detected, invalid sequence, state hash mismatch variants)
- [ ] T007 Create `crates/ego-scheduler/src/types.rs` with `EntityTriple` struct `{ tenant: String, entity_type: String, entity_id: String }`, derive `Debug, Clone, Hash, Eq, PartialEq`
- [ ] T008 [P] Create `crates/ego-scheduler/src/event.rs` with `SchedulerEvent` enum (`ExecutionCompleted { entity: EntityTriple, state_version: u64 }`, `RecoveryCompleted { entity: EntityTriple, state_version: u64 }`)
- [ ] T009 [P] Create `crates/ego-scheduler/src/event.rs` with `SchedulerEventEnvelope` struct `{ event_id: [u8; 32], sequence_id: u64, event_type: EventType, payload: SchedulerEvent, source_actor: EntityTriple }` and `EventType` enum
- [ ] T010 Create `crates/ego-scheduler/src/event_bus.rs` with `EventBusConfig { capacity: usize }`, `SchedulerEventSender` with `try_send` returning `Result<(), SchedulerError>`, `SchedulerEventReceiver` with `drain_all() -> Vec<BusItem>`, `BusItem` struct, and `SchedulerTrigger` (wraps `tokio::sync::Notify`)
- [ ] T011 [P] Implement `event_bus_channel()` and `event_bus_channel_with_config()` factory functions in `crates/ego-scheduler/src/event_bus.rs`

**Checkpoint**: Core types, errors, and event bus infrastructure exist and are tested.

---

## Phase 3: User Story 1 — Reactive Scheduling (Priority: P1) 🎯 MVP

**Goal**: Scheduler consumes events, updates SchedulerState deterministically, and produces activation suggestions via SchedulingPolicy.

**Independent Test**: Feed two Scheduler instances identical event sequences; verify SchedulerState is identical. Feed SchedulerState + pending set to `suggest_activation`; verify deterministic output.

- [ ] T012 Create `crates/ego-scheduler/src/state.rs` with `SchedulerState` struct `{ total_events_consumed: u64, last_sequence_id: Option<u64>, detected_gaps: u64, replay_buffer: VecDeque<(u64, SchedulerEventEnvelope)>, last_suggestion: Option<EntityTriple>, state_hash: Option<[u8; 32]> }` with `new()` constructor
- [ ] T013 Implement `SchedulerState::apply()` pure function in `crates/ego-scheduler/src/state.rs`: takes `&self` and `&SchedulerEventEnvelope`, returns new `SchedulerState` with incremented counters and updated fields
- [ ] T014 [P] [US1] Create `crates/ego-scheduler/src/policy.rs` with `SchedulingPolicy` trait: `fn suggest_activation(&self, state: &SchedulerState, pending_entities: &HashSet<EntityTriple>) -> Option<EntityTriple>`
- [ ] T015 [US1] Implement `RoundRobin` policy in `crates/ego-scheduler/src/policy.rs`: sorts `pending_entities` lexicographically, indexes by `total_events_consumed % len`
- [ ] T016 Create `crates/ego-scheduler/src/scheduler.rs` with `Scheduler` struct holding `SchedulerState`, `SchedulerEventReceiver`, `Box<dyn SchedulingPolicy>`
- [ ] T017 [US1] Implement `Scheduler::drain_and_apply()` in `crates/ego-scheduler/src/scheduler.rs`: drains event bus, applies each event to state
- [ ] T018 [US1] Implement `Scheduler::suggest_activation()` in `crates/ego-scheduler/src/scheduler.rs`: calls `policy.suggest_activation(state, pending)`
- [ ] T019 [US1] Write determinism tests in `crates/ego-scheduler/src/scheduler.rs` `#[cfg(test)]`: two Scheduler instances, identical events, assert identical state
- [ ] T020 [US1] Write policy determinism test in `crates/ego-scheduler/src/policy.rs` `#[cfg(test)]`: property-based, 1000 random inputs, assert identical output for identical inputs
- [ ] T021 [US1] Create `crates/ego-scheduler/src/suggestion.rs` with `Suggestion` struct and helper functions

**Checkpoint**: Scheduler can consume events, maintain state, and produce deterministic suggestions. MVP complete.

---

## Phase 4: User Story 2 — Backpressure Under Load (Priority: P2)

**Goal**: Event bus applies configured drop policy; Block mode prevents loss, DropNewest mode prevents Actor blocking.

**Independent Test**: Fill event bus beyond capacity in DropNewest mode; verify events dropped and Actor never blocks. Fill in Block mode; verify sender blocks and no events lost.

- [ ] T022 [P] [US2] Implement `DropPolicy` enum (`Block`, `DropNewest`, `DropOldest`) in `crates/ego-scheduler/src/event_bus.rs`
- [ ] T023 [US2] Integrate `DropPolicy` into `SchedulerEventSender`: `try_send` with `DropNewest` silently drops on full, returns `Ok(true)`
- [ ] T024 [US2] Implement high-water mark (default 90%) blocking in `SchedulerEventSender`: when capacity exceeds threshold, sender blocks via `tokio::sync::Notify` until drain
- [ ] T025 [US2] Write backpressure tests in `crates/ego-scheduler/src/event_bus.rs` `#[cfg(test)]`: capacity 10, emit 100 with DropNewest, verify `detected_gaps > 0` on consumer side
- [ ] T026 [US2] Write Block mode test: capacity 10, emit 15 with Block, verify all 15 consumed without loss

**Checkpoint**: Backpressure policies work correctly. Both lossless and lossy modes tested.

---

## Phase 5: User Story 3 — Diagnostic Replay Verification (Priority: P3)

**Goal**: Replay buffer stores recent events for diagnostic replay. Full state reconstruction is NOT supported via buffer.

**Independent Test**: Feed 2000 events, verify buffer cap at 1024. Replay last N events on fresh state, verify state matches original for those N.

- [ ] T027 [US3] Implement replay buffer logic in `SchedulerState::apply()` in `crates/ego-scheduler/src/state.rs`: push `(sequence_id, event)` to `replay_buffer`, truncate when `> 1024`
- [ ] T028 [US3] Implement `SchedulerState::replay_last_n(n: usize)` in `crates/ego-scheduler/src/state.rs`: creates a fresh state, replays last N events from buffer, returns new state
- [ ] T029 [US3] Write replay buffer tests in `crates/ego-scheduler/src/state.rs` `#[cfg(test)]`: 2000 events → buffer length <= 1024; replay N → state matches original for those N

**Checkpoint**: Replay buffer bounded, diagnostic-only, and correctly replayable.

---

## Phase 6: User Story 4 — Gap Detection and Monitoring (Priority: P3)

**Goal**: Scheduler detects missing sequence_ids and exposes gap metrics. System continues under gaps.

**Independent Test**: Feed events with sequence_ids [1, 2, 4, 5]; verify `detected_gaps >= 1` and gap range recorded. Verify `suggest_activation` still produces output under gaps.

- [ ] T030 [P] [US4] Implement `GapInfo` struct `{ start_seq: u64, end_seq: u64, source_actor: EntityTriple }` in `crates/ego-scheduler/src/gap.rs`
- [ ] T031 [US4] Implement gap detection in `SchedulerState::apply()`: compare `event.sequence_id` with `last_sequence_id + 1`, if gap detected increment `detected_gaps` and log gap range via `tracing::debug!`
- [ ] T032 [P] [US4] Create `crates/ego-scheduler/src/metric.rs` with core metrics: `total_events_consumed` (counter), `events_consumed_rate` (gauge), `detected_gaps_total` (counter), `last_sequence_id` (gauge), `suggestions_produced` (counter), `suggestions_consumed` (counter)
- [ ] T033 [US4] Emit metrics from `Scheduler::drain_and_apply()` and `Scheduler::suggest_activation()` in `crates/ego-scheduler/src/scheduler.rs`
- [ ] T034 [US4] Write gap detection tests in `crates/ego-scheduler/src/gap.rs` `#[cfg(test)]`: events with missing sequence_id, verify gap detected, range recorded, system continues

**Checkpoint**: Gap detection and metrics operational. System resilient under loss.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Finalize the crate — verify CORE-006 unchanged, run full workspace, finalize documentation.

- [ ] T035 [P] Add `#[cfg(test)]` module-level doc tests for all public functions in `crates/ego-scheduler/src/lib.rs`
- [ ] T036 Verify CORE-006 unchanged: `git diff --name-only main...HEAD -- crates/runtime/ crates/persistent-entity/ crates/domain/` — must be empty
- [ ] T037 Run full workspace compilation: `cargo check --workspace`
- [ ] T038 Run full workspace tests: `cargo test --workspace`
- [ ] T039 Run clippy: `cargo clippy --workspace -- -D warnings`
- [ ] T040 Update `specs/007-reactive-scheduler-projection/quickstart.md` with final commands if changed during implementation

**Checkpoint**: Crate is polished, workspace builds, all tests pass, CORE-006 is unchanged.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies — start immediately
- **Phase 2 (Foundational)**: Depends on Phase 1 — BLOCKS all user stories
- **Phase 3 (US1 - P1)**: Depends on Phase 2
- **Phase 4 (US2 - P2)**: Depends on Phase 2
- **Phase 5 (US3 - P3)**: Depends on Phase 2 (T027 depends on T012 state.rs)
- **Phase 6 (US4 - P3)**: Depends on Phase 2 (T031 depends on T012 state.rs)
- **Phase 7 (Polish)**: Depends on all user stories

### User Story Dependencies

- **US1 (P1)**: Foundational only — no story dependencies
- **US2 (P2)**: Foundational only — independently testable
- **US3 (P3)**: Foundational + US1 (replay buffer is part of SchedulerState from US1)
- **US4 (P3)**: Foundational + US1 (gap detection in SchedulerState::apply from US1)

### Within Each Phase

- Core types before services
- Models before logic
- Pure functions before integration
- Tests before implementation
- Phase complete before advancing

### Parallel Opportunities

- T003, T004: Setup parallel (different files)
- T008, T009: Event types parallel (same file but different structs)
- T014, T022, T030, T032: Policy trait, DropPolicy, GapInfo, metrics — all independent
- US2, US3, US4: Can proceed in parallel after Phase 2 + US1 core state logic

---

## Parallel Example: User Story 1

```bash
# Launch policy trait + RoundRobin together:
Task: "Create SchedulingPolicy trait + RoundRobin in policy.rs"
Task: "Create SchedulerState with apply() in state.rs"
Task: "Create Scheduler with drain_and_apply() in scheduler.rs"
```

---

## Implementation Strategy

### MVP First (US1 Only)

1. Complete Phase 1: Setup (crate creation)
2. Complete Phase 2: Foundational (types, event bus)
3. Complete Phase 3: User Story 1 (Scheduler + policy)
4. **STOP and VALIDATE**: `cargo test -p ego-scheduler`, two-instance determinism test
5. Deploy/demo if ready

### Incremental Delivery

1. Phase 1 + Phase 2 → Foundation ready
2. Phase 3 (US1) → MVP: deterministic scheduling
3. Phase 4 (US2) → Backpressure support
4. Phase 5 (US3) → Diagnostic replay
5. Phase 6 (US4) → Gap detection + metrics
6. Phase 7 → Polish and final validation

### Parallel Team Strategy

1. Foundation setup (Phase 1 + Phase 2): single developer
2. Once Foundation done:
   - Developer A: US1 (Scheduler + state)
   - Developer B: US2 (drop policy)
3. After US1 completes:
   - Developer A: US3 (replay) or US4 (gaps)
   - Developer B: US4 (gaps) or US3 (replay)
4. Polish: any available developer
