# Tasks: Persistent Entity Runtime and SDK

**Input**: Design documents from `/specs/006-persistent-entity-runtime/`

**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: Included - validation tests that verify the formal model against the existing implementation.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **Crate root**: `crates/persistent-entity/`
- **Source**: `crates/persistent-entity/src/`
- **Tests**: `crates/persistent-entity/src/` (inline `#[cfg(test)] mod tests { ... }` per existing pattern)
- **Feature docs**: `specs/006-persistent-entity-runtime/`

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project initialization and basic structure

- [ ] T001 Create project structure per implementation plan
- [ ] T002 Initialize Rust project with dependencies in `crates/persistent-entity/Cargo.toml`
- [ ] T003 [P] Configure linting and formatting tools

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core infrastructure that MUST be complete before ANY user story can be implemented

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T004 Setup database schema and migrations framework
- [ ] T005 [P] Implement authentication/authorization framework
- [ ] T006 [P] Setup API routing and middleware structure
- [ ] T007 Create base models/entities that all stories depend on
- [ ] T008 Configure error handling and logging infrastructure
- [ ] T009 Setup environment configuration management

**Checkpoint**: Foundation ready - user story implementation can now begin in parallel

---

## Phase 3: User Story 1 - Activation Ordering Formal Model (Priority: P1) 🎯 MVP

**Goal**: Verify all activation ordering invariants: activation finds or creates actor, commands are processed in FIFO order, and no partial state is observable.

**Independent Test**: `test_activation_ordering` sends commands to a passivated entity and verifies exactly one actor spawns and commands process in order.

### Tests for User Story 1 (TDD) ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [ ] T010 [P] [US1] Test `test_activation_lookup_active` — entity in active registry returns `Some(sender)` from `get_active_sender` in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T011 [P] [US1] Test `test_activation_lookup_passivated` — passivated entity returns `None` from `get_active_sender` and triggers activation in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T012 [P] [US1] Test `test_activation_fifo_ordering` — send 5 commands sequentially and verify response order matches send order in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T013 [P] [US1] Test `test_no_partial_state_observable` — send command during recovery window and verify response contains fully-recovered state (all events applied) in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T014 [US1] Test `test_activation_redirect` — after activation, a concurrent caller finds active entity and sends directly without spawn in `crates/persistent-entity/tests/activation_ordering_tests.rs`

### Implementation for User Story 1

- [ ] T015 [P] [US1] Create `EntityRef` API in `crates/persistent-entity/src/entity_ref.rs`
- [ ] T016 [P] [US1] Implement `PersistentEntity` trait in `crates/persistent-entity/src/persistent_entity.rs`
- [ ] T017 [US1] Implement `CommandContext` in `crates/persistent-entity/src/command_context.rs`
- [ ] T018 [US1] Implement `EntityRuntime` lifecycle manager in `crates/persistent-entity/src/runtime.rs`
- [ ] T019 [US1] Implement `EntityActor` dedicated task + mailbox loop in `crates/persistent-entity/src/actor.rs`
- [ ] T020 [US1] Implement `LifecycleStateMachine` in `crates/persistent-entity/src/lifecycle.rs`
- [ ] T021 [US1] Implement `Bounded FIFO mailbox` in `crates/persistent-entity/src/mailbox.rs`
- [ ] T022 [US1] Implement `State recovery` with snapshot load + event replay in `crates/persistent-entity/src/recovery.rs`
- [ ] T023 [US1] Implement `Passivation policy` + registry in `crates/persistent-entity/src/passivation.rs`
- [ ] T024 [US1] Implement `SnapshotStrategy` trait + built-in strategies in `crates/persistent-entity/src/snapshot.rs`
- [ ] T025 [US1] Implement `Error types` in `crates/persistent-entity/src/error.rs`
- [ ] T026 [US1] Implement `Test helpers` in `crates/persistent-entity/src/testing.rs`

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently

---

## Phase 4: User Story 2 - No Double Actor Spawn (Priority: P1)

**Goal**: Verify single-flight activation ensures exactly one actor per entity under any concurrency level.

**Independent Test**: `test_no_double_spawn` sends 100 concurrent commands to a passivated entity and asserts `active_count() == 1` at all times.

### Tests for User Story 2 (TDD) ⚠️

- [ ] T027 [P] [US2] Test `test_no_double_spawn_concurrent` — spawn 100 concurrent tasks sending commands to the same passivated entity, verify `registry.active_count()` is exactly 1 in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T028 [P] [US2] Test `test_no_double_spawn_racing_activation` — two concurrent activations for the same entity, verify mutex serializes and exactly one spawns in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T029 [P] [US2] Test `test_activation_mutex_serializes` — verify that during the mutex-holder's spawn window, concurrent callers block and then redirect (not spawn) in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T030 [P] [US2] Test `test_no_double_spawn_multiple_entities` — 10 concurrent spawns for 10 different entities, verify each gets exactly one actor (total 10) in `crates/persistent-entity/tests/activation_ordering_tests.rs`

### Implementation for User Story 2

- [ ] T031 [P] [US2] Implement single-flight activation logic in `crates/persistent-entity/src/actor.rs`
- [ ] T032 [US2] Implement mutex-based activation guard in `crates/persistent-entity/src/actor.rs`
- [ ] T033 [US2] Implement activation redirect logic in `crates/persistent-entity/src/actor.rs`
- [ ] T034 [US2] Implement concurrent spawn detection in `crates/persistent-entity/src/actor.rs`

**Checkpoint**: At this point, User Story 2 should be fully functional and testable independently

---

## Phase 5: User Story 3 - Deterministic Recovery Ordering (Priority: P2)

**Goal**: Verify recovery completes before any command processing, event replay is deterministic, and recovery-failure transitions to FAILED with cleanup.

**Independent Test**: `test_recovery_barrier` pre-loads 100 events, activates entity, sends command during recovery, and verifies command sees version 100+1.

### Tests for User Story 3 (TDD) ⚠️

- [ ] T035 [P] [US3] Test `test_recovery_barrier` — pre-store 100 events for an entity, activate and send a command, verify command response shows version >= 100 in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T036 [P] [US3] Test `test_recovery_deterministic_replay` — two activations of the same entity with identical event streams produce identical final states in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T037 [P] [US3] Test `test_recovery_failure_transitions_to_failed` — cause recovery to fail (e.g., corrupt snapshot data), verify actor transitions to FAILED and `remove_active()` is called in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T038 [P] [US3] Test `test_recovery_retry_after_failure` — after recovery failure, send another command and verify it triggers a fresh activation attempt in `crates/persistent-entity/tests/activation_ordering_tests.rs`
- [ ] T039 [P] [US3] Test `test_recovery_commands_buffered_during_recovery` — send 5 commands during recovery (by timing or state inspection), verify all 5 are processed after recovery completes in `crates/persistent-entity/tests/activation_ordering_tests.rs`

### Implementation for User Story 3

- [ ] T040 [P] [US3] Implement recovery barrier logic in `crates/persistent-entity/src/recovery.rs`
- [ ] T041 [US3] Implement deterministic replay logic in `crates/persistent-entity/src/recovery.rs`
- [ ] T042 [US3] Implement recovery failure handling in `crates/persistent-entity/src/recovery.rs`
- [ ] T043 [US3] Implement recovery retry logic in `crates/persistent-entity/src/recovery.rs`
- [ ] T044 [US3] Implement command buffering during recovery in `crates/persistent-entity/src/recovery.rs`

**Checkpoint**: At this point, User Story 3 should be fully functional and testable independently

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Improvements that affect multiple user stories

- [ ] T045 [P] Documentation updates in docs/
- [ ] T046 Code cleanup and refactoring
- [ ] T047 Performance optimization across all stories
- [ ] T048 [P] Additional unit tests (if requested) in tests/unit/
- [ ] T049 Security hardening
- [ ] T050 Run quickstart.md validation

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - User stories can then proceed in parallel (if staffed)
  - Or sequentially in priority order (P1 → P2 → P3)
- **Polish (Final Phase)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories
- **User Story 2 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories
- **User Story 3 (P2)**: Can start after Foundational (Phase 2) - No dependencies on other stories

### Within Each User Story

- Tests (if included) MUST be written and FAIL before implementation
- Models before services
- Services before endpoints
- Core implementation before integration
- Story complete before moving to next priority

### Parallel Opportunities

- All Setup tasks marked [P] can run in parallel
- All Foundational tasks marked [P] can run in parallel (within Phase 2)
- Once Foundational phase completes, all user stories can start in parallel (if team capacity allows)
- All tests for a user story marked [P] can run in parallel
- Models within a story marked [P] can run in parallel
- Different user stories can be worked on in parallel by different team members

---

## Parallel Example: User Story 1

```bash
# Launch all tests for User Story 1 together:
Task: "Test activation_lookup_active in tests/activation_ordering_tests.rs"
Task: "Test activation_lookup_passivated in tests/activation_ordering_tests.rs"
Task: "Test activation_fifo_ordering in tests/activation_ordering_tests.rs"
Task: "Test no_partial_state_observable in tests/activation_ordering_tests.rs"
Task: "Test activation_redirect in tests/activation_ordering_tests.rs"

# Launch all models for User Story 1 together:
Task: "Create EntityRef API in crates/persistent-entity/src/entity_ref.rs"
Task: "Implement PersistentEntity trait in crates/persistent-entity/src/persistent_entity.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1 (activation ordering)
4. **STOP and VALIDATE**: Test User Story 1 independently
5. Deploy/demo if ready

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Test independently → Deploy/Demo (MVP!)
3. Add User Story 2 → Test independently → Deploy/Demo
4. Add User Story 3 → Test independently → Deploy/Demo
5. Each story adds value without breaking previous stories

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: User Story 1 (activation order)
   - Developer B: User Story 2 (no double spawn)
   - Developer C: User Story 3 (recovery ordering)
3. Stories complete and integrate independently

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Verify tests fail before implementing
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- Avoid: vague tasks, same file conflicts, cross-story dependencies that break independence