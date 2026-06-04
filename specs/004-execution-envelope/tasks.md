# Tasks: Execution Envelope

**Input**: Design documents from `specs/004-execution-envelope/`

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

**Purpose**: Module scaffolding and identity/correlation type definitions

- [ ] T001 Create domain context module at `crates/domain/src/context.rs` with identity types: `AggregateId`, `EntityId`, `TenantId`, `CorrelationId`, `CausationId`, `RequestId` (each wrapping `String` with non-empty validation), and `Metadata` (type alias for `HashMap<String, String>`). Derive Debug, Clone, PartialEq, Eq.
- [ ] T002 [P] Add `pub mod context` and re-export all types in `crates/domain/src/lib.rs`
- [ ] T003 [P] Create `ExecutionEnvelope<P>` struct in `crates/domain/src/envelope.rs` with fields: `payload: P`, `aggregate_id: Option<AggregateId>`, `entity_id: Option<EntityId>`, `tenant_id: Option<TenantId>`, `correlation_id: Option<CorrelationId>`, `causation_id: Option<CausationId>`, `request_id: Option<RequestId>`, `metadata: Metadata`. Derive Debug, Clone, PartialEq, Eq. Reuse types from `crate::context`.
- [ ] T004 Add `pub mod envelope` and re-export in `crates/domain/src/lib.rs`

**Checkpoint**: `cargo build -p ego-domain` succeeds. Envelope struct compiles.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Envelope construction and ExecutionContext conversion trait

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T005 [US1] Add `from_envelope` constructor to domain context: implement `impl<P> From<ExecutionEnvelope<P>> for ExecutionContext` (or define the domain ExecutionContext struct/trait with accessors for all identity, correlation, and metadata fields). Define in `crates/domain/src/context.rs`.
- [ ] T006 [US1] Write and verify unit tests in `crates/domain/src/envelope.rs`: construct `ExecutionEnvelope<String>` with all fields set, assert field values match. Test `None` cases for each optional field.
- [ ] T007 [US1] Write and verify unit tests in `crates/domain/src/context.rs`: construct envelope, convert to ExecutionContext, assert all accessors return matching values.

**Checkpoint**: `cargo test -p ego-domain` passes. Envelope ↔ context conversion works.

---

## Phase 3: User Story 1 — Construct Context from Envelope (Priority: P1) 🎯 MVP

**Goal**: Runtime implementation constructs ExecutionContext from an incoming ExecutionEnvelope, wiring identity, correlation, and metadata to execution handlers.

**Independent Test**: Build a `RuntimeExecutionContext` from an `ExecutionEnvelope` with known fields, assert all accessors return matching values.

- [ ] T008 [US1] Refactor `crates/runtime/src/context.rs` — add `from_envelope<P>(envelope: ExecutionEnvelope<P>) -> Self` constructor to existing runtime `CommandContext` struct. Wire envelope identity, correlation, and metadata fields to struct fields.
- [ ] T009 [US1] Implement the domain `ExecutionContext` trait (or equivalent) on the runtime `CommandContext` struct with all accessors wired from struct fields in `crates/runtime/src/context.rs`.
- [ ] T010 [P] [US1] Write integration tests in `crates/runtime/src/context.rs`: construct envelope with all fields set, build runtime context, verify all accessors return correct values. Test `None` cases. Test round-trip: same values in → same values out.

**Checkpoint**: `cargo test -p ego-runtime` passes. Envelope → context works end-to-end.

---

## Phase 4: User Story 2 — Carry Arbitrary Payloads (Priority: P1)

**Goal**: Envelope carries any payload type (command, event, workflow message, saga message, projection message) without assuming the payload type.

**Independent Test**: Construct envelopes with different payload types (String, a custom struct, a Vec), verify the payload is preserved identically.

- [ ] T011 [P] [US2] Define a test command struct and test event struct in `crates/domain/src/envelope.rs` tests. Construct `ExecutionEnvelope` parametrized with each type. Assert payload round-trips unchanged.
- [ ] T012 [US2] Verify that `From<ExecutionEnvelope<P>>` for the runtime context works with multiple payload types without modification in `crates/runtime/src/context.rs` tests. Parameterize test with `String`, test command struct, and test event struct.

**Checkpoint**: `cargo test -p ego-domain -p ego-runtime` passes. Payload type is fully generic.

---

## Phase 5: User Story 3 — Transport Independence (Priority: P2)

**Goal**: Same envelope type works across in-process, actor, cluster, HTTP, gRPC, and messaging transports without modification.

**Independent Test**: Construct envelope in a unit test with zero transport infrastructure. Optionally test round-trip through a serialization format that the transport layer might use.

- [ ] T013 [US3] Add unit test in `crates/domain/src/envelope.rs` that constructs an `ExecutionEnvelope` in a pure unit test with no transport dependencies. Assert the envelope is fully functional (fields match, clone works, debug prints, equality works).
- [ ] T014 [US3] Add serialization round-trip test in `crates/domain/src/envelope.rs`: `ExecutionEnvelope<String>` with all identity fields, correlation fields, and metadata. Serialize and deserialize (via serde). Assert all values survive round-trip.

**Checkpoint**: `cargo test -p ego-domain` passes. Envelope is fully transport-independent.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Validation, no-regression checks

- [ ] T015 Run `cargo test --workspace` to verify no regressions across all crates
- [ ] T016 Run quickstart.md validation scenarios from `specs/004-execution-envelope/quickstart.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion — BLOCKS all user stories
- **User Story 1 (Phase 3)**: Depends on Foundational completion — requires envelope + context types
- **User Story 2 (Phase 4)**: Depends on Foundational completion — requires envelope struct; independent of US1
- **User Story 3 (Phase 5)**: Depends on Foundational completion — requires envelope struct; independent of US1 and US2
- **Polish (Phase 6)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) — No dependencies on other stories
- **User Story 2 (P1)**: Can start after Foundational (Phase 2) — Independent of US1 and US3
- **User Story 3 (P2)**: Can start after Foundational (Phase 2) — Independent of US1 and US2

### Within Each Phase

- Write failing test first (TDD — Red/Green/Refactor per `.speckit/constitution.md`)
- Implementation after failing test is verified
- Models/types before services/converters
- Tests for the types before integration code

### Parallel Opportunities

- T002 and T003 (Setup) can run in parallel — different files, no dependencies
- T011 and T012 (US2) use different modules and can be written in parallel
- US2 and US3 require only Foundational (not US1) and can proceed in parallel

---

## Parallel Example: User Story 1

```bash
# Launch all tests first:
Task: "Test envelope construction in crates/domain/src/envelope.rs"
Task: "Test context conversion in crates/domain/src/context.rs"
```

---

## Implementation Strategy

### MVP First (Phases 1-3 Complete)

1. Complete Phase 1: Setup — identity types + Envelope struct
2. Complete Phase 2: Foundational — construction + context conversion
3. Complete Phase 3: User Story 1 — runtime context refactored
4. `cargo test --workspace` passes — MVP delivered

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Runtime context from envelope (MVP!)
3. Add User Story 2 → Multiple payload types verified
4. Add User Story 3 → Transport independence verified
5. Each story adds value without breaking previous stories

---

## Notes

- Envelope is a struct (not a trait) — data carrier, no behavior
- Payload type P is generic — determined by execution model
- All identity/correlation types defined in Phase 1 — no external dependencies
- Runtime struct refactoring is additive — existing constructors continue to work
- [P] tasks = different files, no dependencies on each other
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Verify tests fail before implementing (TDD)
