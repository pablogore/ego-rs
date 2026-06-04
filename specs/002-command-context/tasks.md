# Tasks: Execution Context

**Input**: Design documents from `specs/002-command-context/`

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

**Purpose**: Module scaffolding and crate configuration

- [x] T001 Create identity and correlation types (AggregateId, EntityId, TenantId, CorrelationId, CausationId, RequestId) following existing ActorId pattern, and Metadata type alias, in `crates/domain/src/context.rs`

- [x] T002 [P] Add ego-domain dependency to `crates/runtime/Cargo.toml`

- [x] T003 Add context module to `crates/domain/src/lib.rs` with `pub mod context` and re-exports

**Checkpoint**: `cargo build -p ego-domain` succeeds. Identity types compile.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: ExecutionContext trait definition — read-only execution context

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [x] T004 Define `ExecutionContext` trait in `crates/domain/src/context.rs` with seven read-only accessors: aggregate_id, entity_id, tenant_id, correlation_id, causation_id, request_id (all returning `Option<&...>`), and metadata (returning `&Metadata`). Trait uses `&self` exclusively — no `&mut self` methods. Follow contract in `contracts/execution_context.md`.

- [x] T005 Add unit tests for identity type construction (following existing patterns in `crates/domain/src/actor.rs`) in `crates/domain/src/context.rs`

**Checkpoint**: `cargo build -p ego-domain` succeeds. `cargo test -p ego-domain` passes.

---

## Phase 3: User Story 1 - Identity context (Priority: P1) 🎯 MVP

**Goal**: Developers can read aggregate_id, entity_id, and tenant_id from the context inside an execution handler.

**Independent Test**: Create a runtime ExecutionContext with identity fields set, call a handler that reads them, verify the returned values match.

### Implementation for User Story 1

- [x] T006 [US1] Refactor `crates/runtime/src/context.rs` — remove local `CorrelationId` type (moved to domain), add identity fields (aggregate_id, entity_id, tenant_id) to the existing runtime context struct; implement domain `ExecutionContext` trait with identity accessor methods wired from struct fields

- [x] T007 [P] [US1] Add identity integration tests in `crates/runtime/src/context.rs` verifying aggregate_id, entity_id, tenant_id round-trip correctly and absent fields return None

**Checkpoint**: `cargo build -p ego-runtime` succeeds. `cargo test -p ego-runtime` passes. Identity access works end-to-end.

---

## Phase 4: User Story 2 - Correlation context (Priority: P1) 🎯 MVP

**Goal**: Developers can read correlation_id, causation_id, and request_id from the context.

**Independent Test**: Create a runtime ExecutionContext with correlation fields set, verify the handler reads the correct values.

### Implementation for User Story 2

- [x] T008 [US2] Add correlation fields to runtime context struct in `crates/runtime/src/context.rs`; implement correlation accessor methods (`correlation_id`, `causation_id`, `request_id`) using domain `CorrelationId` type (migrate existing `correlation_id` field from local `CorrelationId` to domain `CorrelationId`)

- [x] T009 [P] [US2] Add correlation integration tests in `crates/runtime/src/context.rs` verifying all three correlation fields round-trip correctly and absent fields return None

**Checkpoint**: `cargo test -p ego-runtime` passes. Correlation access works.

---

## Phase 5: User Story 3 - Metadata access (Priority: P1) 🎯 MVP

**Goal**: Developers can read arbitrary key/value metadata from the incoming request through the context.

**Independent Test**: Attach metadata to a message, verify the handler reads the correct key/value pairs.

### Implementation for User Story 3

- [x] T010 [US3] Add metadata field to runtime context struct in `crates/runtime/src/context.rs`; implement `metadata` accessor returning `&Metadata`

- [x] T011 [P] [US3] Add metadata integration tests in `crates/runtime/src/context.rs` verifying populated metadata and empty metadata cases

**Checkpoint**: `cargo test -p ego-runtime` passes. Metadata access works. MVP complete.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Validation, portability verification, and documentation updates

- [x] T012 Run `cargo test --workspace` to verify no regressions across all crates

- [x] T013 Verify runtime portability — confirm the same handler code works with different runtime configurations (trait is domain-owned, handler code needs only `use ego_domain::ExecutionContext`)

  ```yaml
  evidence:
    command: cargo test -p ego-runtime
    exit_code: 0
  ```
  trait defined in `crates/domain/src/context.rs`, re-exported from `ego_domain`. `test_trait_impl` demonstrates trait-object compatibility — handler code relies solely on `use ego_domain::context::ExecutionContext`.

- [x] T014 Update `crates/runtime/src/lib.rs` exports if needed to maintain backward compatibility

- [x] T015 Run quickstart.md validation scenarios from `specs/002-command-context/quickstart.md`

  ```yaml
  evidence:
    command: cargo test -p ego-domain -p ego-runtime
    exit_code: 0
  ```
  Scenario 1: `cargo build -p ego-domain` → success
  Scenario 2: identity access → `test_identity_fields_round_trip`, `test_identity_fields_none` pass
  Scenario 3: correlation access → `test_correlation_fields_round_trip`, `test_correlation_fields_none` pass
  Scenario 4: metadata access → `test_metadata_populated`, `test_metadata_empty` pass
  Scenario 5: runtime portability → `test_trait_impl` passes (18/18 tests pass)

---

## Future Follow-up Specs

The following capabilities were extracted from the original ExecutionContext scope and are deferred to future specifications. They are NOT implemented in this spec.

### Effect Runtime

Model side effects (persist events, send replies, no-op) as a value type that handlers return.

```rust
// Conceptual — not implemented here
enum Effect<E: DomainEvent, R> {
    Persist(Vec<E>),
    Reply(R),
    PersistAndReply(Vec<E>, R),
    None,
}
```

**Future spec**: Effect API for execution handlers. Includes:
- Effect value type (enum)
- Runtime interpreter for effects
- Persist integration with EventStore SPI
- Reply integration with transport layer

### Scheduling Runtime

Model delayed and recurring execution as a separate scheduling abstraction.

**Future spec**: Scheduling API. Includes:
- One-shot scheduling
- Recurring scheduling
- Runtime-agnostic timer abstraction
- Integration with runtime backends (Tokio, test, etc.)

### Command Reply Model

Model command replies as part of the Effect API, not as a method on ExecutionContext.

**Future spec part of Effect API above.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion — BLOCKS all user stories
- **User Stories (Phase 3-5)**: All depend on Foundational phase completion
- **Polish (Phase 6)**: Depends on all user stories being complete

### User Story Dependencies

- **US1 (P1)**: Can start after Foundational (Phase 2) — No dependencies on other stories
- **US2 (P1)**: Can start after Foundational (Phase 2) — Independent from US1
- **US3 (P1)**: Can start after Foundational (Phase 2) — Independent from US1/US2

### Within Each User Story

- Types/fields before accessor methods
- Write failing test first (TDD — Red/Green/Refactor per `.speckit/constitution.md`)
- Implementation after failing test is verified
- Story complete before moving to next priority

### Parallel Opportunities

- T002 and T003 can run in parallel
- T007 (US1 tests) can run in parallel with T006 implementation
- T009 (US2 tests) can run in parallel with T008 implementation
- T011 (US3 tests) can run in parallel with T010 implementation
- US1, US2, US3 can all start in parallel after Foundational phase

---

## Implementation Strategy

### MVP (Phases 1-5 Complete)

1. Complete Phase 1: Setup — module scaffolding
2. Complete Phase 2: Foundational — trait definition
3. Complete Phase 3-5: US1 + US2 + US3 — identity, correlation, metadata
4. `cargo test --workspace` passes — MVP delivered

### Incremental Delivery

1. Setup + Foundational → Core trait ready
2. Add US1 → Identity accessible → Deploy/Demo
3. Add US2 → Correlation accessible → Deploy/Demo
4. Add US3 → Metadata accessible → Deploy/Demo (MVP!)

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: US1 (identity)
   - Developer B: US2 (correlation)
   - Developer C: US3 (metadata)
3. No merge conflicts — each story works on different struct fields

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story is independently completable and testable
- No side-effect methods — ExecutionContext is read-only `&self`
- Future specs documented for Effect API, Scheduling API, Reply Model
