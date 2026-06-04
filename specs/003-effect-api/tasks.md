# Tasks: Effect API

**Input**: Design documents from `specs/003-effect-api/`

**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2)
- Include exact file paths in descriptions

## Path Conventions

- **Crate paths**: `crates/domain/src/`, `crates/runtime/src/` at repository root
- All paths are relative to repository root

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Module scaffolding and Effect type definition

- [x] T001 Define `Effect<E, R, S>` enum with five variants (NoEffect, StateMutation, EventEmission, Reply, Composed) in `crates/domain/src/effect.rs`. Derive Debug, Clone, PartialEq, Eq, Hash.

  ```yaml
  evidence:
    command: cargo build -p ego-domain
    exit_code: 0
  ```

- [x] T002 (moved to T002c — effect module definition)

**Checkpoint**: `cargo build -p ego-domain` succeeds. Effect enum compiles.

---

## Phase 1b: Handler Return Type

**Purpose**: Define the handler return type contract before writing handler tests

- [x] T002b Define handler return type as `type EffectResult<E, R, S> = Effect<E, R, S>` or direct use of `Effect<E, R, S>` in `crates/domain/src/effect.rs`. Document that handlers return Effects synchronously.

  ```yaml
  evidence:
    command: cargo build -p ego-domain
    exit_code: 0
  ```

- [x] T002c Add effect module to `crates/domain/src/lib.rs` with `pub mod effect` and re-exports (moved from T002)

  ```yaml
  evidence:
    command: cargo build -p ego-domain
    exit_code: 0
  ```

**Checkpoint**: `cargo build -p ego-domain` succeeds. Handler return type compiles.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Effect construction and composition logic

- [x] T003 Add constructors for each variant: `Effect::no()`, `Effect::state(s)`, `Effect::emit(events)`, `Effect::reply(r)`, `Effect::compose(children)`

  ```yaml
  evidence:
    command: cargo test -p ego-domain
    exit_code: 0
  ```

- [x] T004 Add composition helper: `and_then` or `combine` method that returns `Composed` with both effects

  ```yaml
  evidence:
    command: cargo test -p ego-domain
    exit_code: 0
  ```

- [x] T005 Add unit tests verifying:
  - All variants construct correctly
  - Composition produces correct nested structure
  - Effects are equal by value

  ```yaml
  evidence:
    command: cargo test -p ego-domain
    exit_code: 0
  ```

**Checkpoint**: `cargo test -p ego-domain` passes. Composition works.

---

## Phase 3: User Story 1 - Return reply (Priority: P1) 🎯 MVP

**Goal**: Developers can describe a reply outcome from an execution handler.

**Independent Test**: Create an Effect::reply(value), assert the returned Effect matches expected.

### Implementation for User Story 1

- [x] T006 [US1] Define test handler function that returns `Effect<String, String, String>`: `fn handle() -> Effect<String, String, String> { Effect::reply("ok".to_string()) }` in test fixture

  ```yaml
  evidence:
    command: cargo test -p ego-domain
    exit_code: 0
  ```

- [x] T007 [P] [US1] Write failing test: call handler, assert returned Effect equals `Effect::reply("ok".to_string())`. Verify test fails before implementing.

  ```yaml
  evidence:
    command: cargo test -p ego-domain
    exit_code: 0
  ```

**Checkpoint**: Test passes. Reply effect is constructable and assertable.

---

## Phase 4: User Story 2 - Emit event (Priority: P1) 🎯 MVP

**Goal**: Developers can describe event emission as an outcome.

**Independent Test**: Create an `Effect::emit(vec![event])`, assert the returned Effect contains the event.

### Implementation for User Story 2

- [x] T008 [US2] Define test handler that returns `Effect::emit(vec![event])`. Write failing test asserting the returned Effect. Verify test fails before implementing.

  ```yaml
  evidence:
    command: cargo test -p ego-domain
    exit_code: 0
  ```

- [x] T009 [P] [US2] Add multi-event test: handler returns `Effect::emit(vec![event1, event2])`, assert structure and values

  ```yaml
  evidence:
    command: cargo test -p ego-domain
    exit_code: 0
  ```

**Checkpoint**: Tests pass. Event emission effect is constructable and assertable.

---

## Phase 5: User Story 3 - Multiple outcomes (Priority: P1) 🎯 MVP

**Goal**: Developers can compose multiple outcomes in a single handler.

**Independent Test**: Return composed Effect with events AND reply, assert the structure.

### Implementation for User Story 3

- [x] T010 [US3] Define test handler returning composed Effect (events + reply). Write failing test asserting both present in composed structure. Verify test fails before implementing.

  ```yaml
  evidence:
    command: cargo test -p ego-domain
    exit_code: 0
  ```

- [x] T011 [P] [US3] Add complex composition test: handler returns `StateMutation` + `EventEmission` + `Reply`, assert flat composed structure (and_then flattens nested Composed)

  ```yaml
  evidence:
    command: cargo test -p ego-domain
    exit_code: 0
  ```

**Checkpoint**: Tests pass. Effect composition works end-to-end.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Validation, documentation, no-regression checks

- [x] T012 Run `cargo test --workspace` to verify no regressions across all crates

  ```yaml
  evidence:
    command: cargo test --workspace
    exit_code: 0
  ```

- [x] T013 Verify compile-time check: Effect variants are exhaustive — any match on Effect produces a non-exhaustive warning when new variant added (future-proofing)

  ```yaml
  evidence:
    command: cargo test -p ego-domain
    exit_code: 0
  ```

- [x] T014 Run quickstart.md validation scenarios from `specs/003-effect-api/quickstart.md`

  ```yaml
  evidence:
    command: cargo test --workspace
    exit_code: 0
  ```

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Handler Return Type (Phase 1b)**: Depends on Phase 1 — BLOCKS all user stories
- **Foundational (Phase 2)**: Depends on Setup completion — BLOCKS all user stories
- **User Stories (Phase 3-5)**: All depend on Foundational phase completion
- **Polish (Phase 6)**: Depends on all user stories being complete

### Within Each User Story

- Types/fields before constructors
- Write failing test first (TDD — Red/Green/Refactor per `.speckit/constitution.md`)
- Implementation after failing test is verified

## Implementation Strategy

### MVP (Phases 1-5 Complete)

1. Complete Phase 1: Setup — Effect type scaffolding
2. Complete Phase 2: Foundational — constructors, composition
3. Complete Phase 3-5: US1 + US2 + US3 — reply, emit, compose
4. `cargo test --workspace` passes — MVP delivered

### Incremental Delivery

1. Setup + Foundational → Core Effect enum ready
2. Add US1 → Reply effect → Demo
3. Add US2 → Event emission → Demo
4. Add US3 → Composition → MVP

## Notes

- Effects are value types — Clone, Debug, PartialEq
- No runtime, no async in Effect definitions
- Runtime interpretation is a separate concern (future implementation)
- Effect API is independent of ExecutionContext (002)
