# Tasks: Activation Ordering Model for Persistent Entity Runtime

**Input**: Design documents from `specs/007-activation-ordering-model/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: Included — validation tests that verify the formal model against the existing implementation.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **Crate root**: `crates/persistent-entity/`
- **Source**: `crates/persistent-entity/src/`
- **Tests**: `crates/persistent-entity/src/` (inline `#[cfg(test)] mod tests { ... }` per existing pattern)
- **Feature docs**: `specs/007-activation-ordering-model/`

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Validation scaffold and test infrastructure

- [ ] T001 Add `tokio-test` and `uuid` dev-dependencies to `crates/persistent-entity/Cargo.toml`
- [ ] T002 [P] Add test module `tests/activation_ordering_tests.rs` at `crates/persistent-entity/tests/activation_ordering_tests.rs` with `#[cfg(test)]` and tokio test harness

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Shared test helpers and fixtures needed by all user story tests

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T003 Create `TestEntity` — a minimal `PersistentEntity<String, TestEvent, TestState>` implementation in `crates/persistent-entity/src/testing.rs` with command-to-event mapping and deterministic `apply_event`
- [ ] T004 Create `TestEvent` enum with variants `Incremented(u64)`, `Decremented(u64)`, `Reset` in `crates/persistent-entity/src/testing.rs` implementing `DomainEvent`
- [ ] T005 Create `TestState` struct with `value: u64` and `version: u64` in `crates/persistent-entity/src/testing.rs` implementing `Serialize + DeserializeOwned`
- [ ] T006 [P] Add helper function `spawn_concurrent_commands(count: usize, entity: &EntityRef<...>)` in `crates/persistent-entity/tests/common/mod.rs` that sends commands from concurrent tasks and collects results

**Checkpoint**: Foundation ready — user story test implementation can now begin in parallel

## Phase 3: User Story 1 - Activation Ordering Formal Model (Priority: P1) 🎯 MVP

**Goal**: Verify all activation ordering invariants: activation finds or creates actor, commands are processed in FIFO order, and no partial state is observable.

**Independent Test**: `test_activation_ordering` sends commands to a passivated entity and verifies exactly one actor spawns and commands process in order.

### Tests for User Story 1 (TDD) ⚠️

- [ ] T007 [P] [US1] Test `test_activation_lookup_active` — entity in active registry returns `Some(sender)` from `get_active_sender` in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T008 [P] [US1] Test `test_activation_lookup_passivated` — passivated entity returns `None` from `get_active_sender` and triggers activation in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T009 [P] [US1] Test `test_activation_fifo_ordering` — send 5 commands sequentially and verify response order matches send order in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T010 [P] [US1] Test `test_no_partial_state_observable` — send command during recovery window and verify response contains fully-recovered state (all events applied) in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T011 [US1] Test `test_activation_redirect` — after activation, a concurrent caller finds active entity and sends directly without spawn in `crates/persistent-entity/tests/activation_ordering_tests.rs`

**Checkpoint**: User Story 1 fully validated — activation ordering model confirmed correct

## Phase 4: User Story 2 - No Double Actor Spawn (Priority: P1)

**Goal**: Verify single-flight activation ensures exactly one actor per entity under any concurrency level.

**Independent Test**: `test_no_double_spawn` sends 100 concurrent commands to a passivated entity and asserts `active_count() == 1` at all times.

### Tests for User Story 2 (TDD) ⚠️

- [ ] T012 [P] [US2] Test `test_no_double_spawn_concurrent` — spawn 100 concurrent tasks sending commands to the same passivated entity, verify `registry.active_count()` is exactly 1 in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T013 [P] [US2] Test `test_no_double_spawn_racing_activation` — two concurrent activations for the same entity, verify mutex serializes and exactly one spawns in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T014 [P] [US2] Test `test_activation_mutex_serializes` — verify that during the mutex-holder's spawn window, concurrent callers block and then redirect (not spawn) in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T015 [P] [US2] Test `test_no_double_spawn_multiple_entities` — 10 concurrent spawns for 10 different entities, verify each gets exactly one actor (total 10) in `crates/persistent-entity/tests/activation_ordering_tests.rs`

**Checkpoint**: User Story 2 validated — single-flight guarantee confirmed under concurrency

## Phase 5: User Story 3 - Deterministic Recovery Ordering (Priority: P2)

**Goal**: Verify recovery completes before any command processing, event replay is deterministic, and recovery-failure transitions to FAILED with cleanup.

**Independent Test**: `test_recovery_barrier` pre-loads 100 events, activates entity, sends command during recovery, and verifies command sees version 100+1.

### Tests for User Story 3 (TDD) ⚠️

- [ ] T016 [P] [US3] Test `test_recovery_barrier` — pre-store 100 events for an entity, activate and send a command, verify command response shows version >= 100 in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T017 [P] [US3] Test `test_recovery_deterministic_replay` — two activations of the same entity with identical event streams produce identical final states in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T018 [P] [US3] Test `test_recovery_failure_transitions_to_failed` — cause recovery to fail (e.g., corrupt snapshot data), verify actor transitions to FAILED and `remove_active()` is called in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T019 [P] [US3] Test `test_recovery_retry_after_failure` — after recovery failure, send another command and verify it triggers a fresh activation attempt in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T020 [P] [US3] Test `test_recovery_commands_buffered_during_recovery` — send 5 commands during recovery (by timing or state inspection), verify all 5 are processed after recovery completes in `crates/persistent-entity/tests/activation_ordering_tests.rs`

**Checkpoint**: All recovery ordering guarantees validated

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Edge case tests, panic safety, and documentation

- [ ] T021 [P] Test `test_actor_panic_during_recovery` — force a panic inside `recover_state()`, verify next command triggers clean activation in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T022 [P] Test `test_actor_panic_during_command` — force panic during `execute_command()`, verify stale sender detection triggers re-activation on next command in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T023 [P] Test `test_activation_guard_released_on_panic` — simulate panic inside activation mutex scope, verify guard releases (no permanent lockout) in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T024 [P] Test `test_passivation_drains_and_snapshots` — process commands until passivation timeout, verify final snapshot stored and entity marked passivated in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T025 Run `cargo test --package ego-persistent-entity` and confirm all 12 existing + ~20 new tests pass
- [ ] T026 Update `specs/007-activation-ordering-model/quickstart.md` with test scenarios and expected outcomes

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion — BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - US1 and US2 are both P1 and can proceed in parallel
  - US3 (P2) can start after Foundational or in parallel with US1/US2
- **Polish (Final Phase)**: Depends on all user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational — no dependencies on other stories
- **User Story 2 (P1)**: Can start after Foundational — no dependencies on other stories
- **User Story 3 (P2)**: Can start after Foundational — no dependencies on other stories

### Within Each User Story

- Tests MUST be written and FAIL before implementation
- Test-only phase — all tasks are test writing tasks

### Parallel Opportunities

- All Phase 1 Setup tasks marked [P] can run in parallel
- All Phase 2 Foundational tasks marked [P] can run in parallel
- Once Foundational completes, all three user stories can start in parallel
- All tests within a story marked [P] can run in parallel
- US1 and US2 (both P1) can be developed simultaneously

## Parallel Example: User Story 1

```bash
# Launch all tests for User Story 1 together:
Task: "Test activation_lookup_active in tests/activation_ordering_tests.rs"
Task: "Test activation_lookup_passivated in tests/activation_ordering_tests.rs"
Task: "Test activation_fifo_ordering in tests/activation_ordering_tests.rs"
Task: "Test no_partial_state_observable in tests/activation_ordering_tests.rs"
Task: "Test activation_redirect in tests/activation_ordering_tests.rs"
```

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational
3. Complete Phase 3: User Story 1 (activation ordering)
4. **STOP and VALIDATE**: `cargo test` passes all US1 tests
5. MVP achieved — activation ordering model validated

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Test independently → **MVP!**
3. Add User Story 2 → Test independently → Full concurrency validation
4. Add User Story 3 → Test independently → Full recovery validation
5. Add Polish tests → Edge case coverage complete

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: User Story 1 (activation order)
   - Developer B: User Story 2 (no double spawn)
   - Developer C: User Story 3 (recovery ordering)
3. All tests target separate file sections — no conflicts

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Verify tests fail before implementing (expected: tests fail because model features already exist but need formal test coverage)
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
