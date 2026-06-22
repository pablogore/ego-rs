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

**Purpose**: Module scaffolding, dependency addition, and build verification

- [X] T001 Add `serde` with `derive` feature to `crates/domain/Cargo.toml` dependencies
- [X] T002 [P] Create `ExecutionEnvelope<P>` struct with all fields and derives in `crates/domain/src/envelope.rs` per `contracts/envelope.md`. Field types reuse `crate::context` identity/correlation types. Derive: Debug, Clone, PartialEq, Eq, Serialize, Deserialize.
- [X] T003 [P] Add `pub mod envelope` and re-export `ExecutionEnvelope` in `crates/domain/src/lib.rs`
- [X] T004 Verify setup: `cargo build -p ego-domain`

**Checkpoint**: `cargo build -p ego-domain` succeeds. Envelope struct compiles with serde derives.

---

## Phase 2: Foundational — Envelope Construction & Context Conversion (Blocking Prerequisites)

**Purpose**: Core envelope behavior and domain-owned context conversion. Blocking for all user stories.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete. Follow TDD: write failing tests first.

### Tests (write FIRST, verify they FAIL)

- [X] T005 [P] [US1] Write unit tests for envelope construction in `crates/domain/src/envelope.rs` (`#[cfg(test)] mod tests`): construct `ExecutionEnvelope<String>` with all fields set (`payload`, `aggregate_id`, `entity_id`, `tenant_id`, `correlation_id`, `causation_id`, `request_id`, `metadata`). Assert each field via direct access. Test `None` cases for each optional identity/correlation field. Test `ExecutionEnvelope<()>` construction for payload-less model. **Verify test failure before implementation**.
- [X] T006 [P] [US1] Write unit test for `From<ExecutionEnvelope<P>> for DomainExecutionContext` in `crates/domain/src/context.rs` (`#[cfg(test)] mod tests`): construct envelope with all fields set, convert via `DomainExecutionContext::from(envelope)`, assert all context accessors return matching values. Test `None` propagation for absent fields. **Verify test failure before implementation**.

### Implementation

- [X] T007 Implement `impl<P> From<ExecutionEnvelope<P>> for DomainExecutionContext` in `crates/domain/src/context.rs` — infallible conversion mapping envelope identity, correlation, and metadata fields to `DomainExecutionContext` struct fields. Conversion drops the payload (context does not carry payload).
- [X] T008 Verify foundational tests pass: `cargo test -p ego-domain`

**Checkpoint**: `cargo test -p ego-domain` passes. Envelope ↔ `DomainExecutionContext` conversion works for all field combinations.

---

## Phase 3: User Story 1 — Construct Context from Envelope (Priority: P1) 🎯 MVP

**Goal**: Runtime implementation constructs `ExecutionContext` from an incoming `ExecutionEnvelope`, wiring identity, correlation, and metadata to execution handlers.

**Independent Test**: Build a `RuntimeExecutionContext` from an `ExecutionEnvelope` with known fields, assert all accessors return matching values.

### Tests (write FIRST, verify they FAIL)

- [X] T009 [US1] Write integration test for `RuntimeExecutionContext::from_envelope()` in `crates/runtime/src/context.rs` (`#[cfg(test)] mod tests`): construct `ExecutionEnvelope<String>` with all fields set, call `RuntimeExecutionContext::from_envelope(envelope)`, assert all context accessors return correct values. Test `None` cases. Test round-trip: same values in envelope → same values out from context accessors. **Verify test failure before implementation**.

### Implementation

- [X] T010 [US1] Add `pub fn from_envelope<P>(envelope: ExecutionEnvelope<P>) -> Self` constructor to `RuntimeExecutionContext` in `crates/runtime/src/context.rs`. Map envelope identity (`aggregate_id`, `entity_id`, `tenant_id`), correlation (`correlation_id`, `causation_id`, `request_id`), and `metadata` fields to struct fields. Drop the payload.
- [X] T011 [US1] Implement `ExecutionContext` trait on `RuntimeExecutionContext` in `crates/runtime/src/context.rs` with accessors for all identity, correlation, and metadata fields wired from struct fields, if not already implemented.
- [X] T012 Verify US1 tests pass: `cargo test -p ego-runtime`

**Checkpoint**: `cargo test -p ego-runtime` passes. Envelope → runtime context works end-to-end. MVP delivered.

---

## Phase 4: User Story 2 — Carry Arbitrary Payloads (Priority: P1)

**Goal**: Envelope carries any payload type (command, event, workflow message, saga message, projection message) without assuming the payload type.

**Independent Test**: Construct envelopes with different payload types, verify the payload is preserved.

### Tests (write FIRST, verify they FAIL)

- [X] T013 [P] [US2] Write multi-payload test in `crates/domain/src/envelope.rs` (`#[cfg(test)] mod tests`): define test structs `TestCommand { data: String }` and `TestEvent { id: u64, name: String }`. Construct `ExecutionEnvelope<TestCommand>` and `ExecutionEnvelope<TestEvent>`. Assert payload field preserves values. Construct `ExecutionEnvelope<i32>`, `ExecutionEnvelope<Vec<String>>`, assert generic payload round-trips unchanged. Construct `ExecutionEnvelope<()>` for payload-less model. **Verify test failure before implementation**.
- [X] T014 [P] [US2] Write multi-payload test in `crates/runtime/src/context.rs` (`#[cfg(test)] mod tests`): construct `RuntimeExecutionContext` from envelopes with different payload types (`TestCommand`, `TestEvent`, `i32`). Assert context fields are populated regardless of `P`. **Verify test failure before implementation**.

### Implementation

- [X] T015 [US2] Verify multi-payload tests pass: `cargo test -p ego-domain -p ego-runtime -- envelope` (the payload-generic struct already supports this — confirm no code changes needed).

**Checkpoint**: `cargo test -p ego-domain -p ego-runtime` passes with multiple payload types. Payload type is fully generic.

---

## Phase 5: User Story 3 — Transport Independence (Priority: P2)

**Goal**: Same envelope type works across in-process, actor, cluster, HTTP, gRPC, and messaging transports without modification.

**Independent Test**: Construct envelope in a unit test with zero transport infrastructure, plus serde round-trip.

### Tests (write FIRST, verify they FAIL)

- [X] T016 [P] [US3] Write transport-free test in `crates/domain/src/envelope.rs` (`#[cfg(test)] mod tests`): construct `ExecutionEnvelope<String>` in a pure unit test. Assert no transport, actor, Tokio, or runtime imports needed. Assert envelope is fully functional: fields match, Clone works, Debug prints, PartialEq compares correctly, Eq holds for equal envelopes. **Verify test failure before implementation**.
- [X] T017 [P] [US3] Write serde round-trip test in `crates/domain/src/envelope.rs` (`#[cfg(test)] mod tests`): construct `ExecutionEnvelope<String>` with all identity fields, correlation fields, and metadata. Serialize to JSON via `serde_json::to_string`, deserialize back via `serde_json::from_str`. Assert all values survive round-trip. **Verify test failure before implementation**.

### Implementation

- [X] T018 [US3] Verify US3 tests pass: `cargo test -p ego-domain -- envelope` (serde derives already on struct — confirm tests pass).

**Checkpoint**: `cargo test -p ego-domain` passes. Envelope is fully transport-independent with verified serde round-trip.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Validation, no-regression checks, quickstart verification

- [X] T019 Run `cargo test --workspace` to verify no regressions across all crates
- [X] T020 Run `cargo clippy --workspace` to verify no warnings
- [X] T021 Run quickstart.md validation scenarios from `specs/004-execution-envelope/quickstart.md`

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
- Verify test failure before implementation
- Implementation after failing test is verified
- Models/types before services/converters
- Tests for the types before integration code

### Parallel Opportunities

- T002 and T003 (Setup) can run in parallel — different files, no dependencies
- T005 and T006 (Foundational tests) can run in parallel — different files
- T013 and T014 (US2 tests) can run in parallel — different files
- T016 and T017 (US3 tests) can run in parallel — different files
- US2 and US3 require only Foundational (not US1) and can proceed in parallel

---

## Parallel Example: User Story 3

```bash
# Launch all US3 tests together (different sections in same file):
Task: "Write transport-free test in crates/domain/src/envelope.rs"
Task: "Write serde round-trip test in crates/domain/src/envelope.rs"
```

---

## Implementation Strategy

### MVP First (Phases 1-3 Complete)

1. Complete Phase 1: Setup — envelope struct compiles with serde
2. Complete Phase 2: Foundational — envelope construction + `DomainExecutionContext` conversion (TDD)
3. Complete Phase 3: User Story 1 — `RuntimeExecutionContext` from envelope (TDD)
4. `cargo test --workspace` passes — MVP delivered

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Runtime context from envelope (MVP!)
3. Add User Story 2 → Multiple payload types verified
4. Add User Story 3 → Transport independence verified (incl. serde round-trip)
5. Each story adds value without breaking previous stories

---

## Notes

- Envelope is a struct (not a trait) — data carrier, no behavior
- Payload type P is generic and mandatory (`payload: P`); payload-less execution models use `ExecutionEnvelope<()>`
- `DomainExecutionContext` implements `From<ExecutionEnvelope<P>>` (infallible, domain-owned)
- `RuntimeExecutionContext` provides named `from_envelope()` constructor (runtime-owned)
- All identity/correlation types defined in Phase 1 — no external dependencies beyond serde
- Runtime struct refactoring is additive — existing constructors continue to work
- [P] tasks = different files, no dependencies on each other
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Verify tests fail before implementing (TDD per constitution)
- All derives include `Serialize, Deserialize` from serde