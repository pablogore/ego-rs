# Tasks: Persistence SPI — MVP

**Input**: [spec.md](spec.md), [plan.md](plan.md)

**Prerequisites**: `plan.md` (required), `spec.md` (required)

**MVP Scope**: PersistenceError, EventStore, Repository, Snapshot traits + contract tests + InMemory backend.

**Deferred**: PostgreSQL backend, migration infrastructure, schema evolution (future specs).

**Design**: [plan.md](plan.md)

**Constitution**: `.speckit/constitution.md` v2.0.0

---

## Task Format

```
- [ ] TXXX [P] [USN] Short description
      Action: Create | Modify | Refactor | Delete
      File: path/to/file.rs
      Section: trait/module name
      Outcome: what exists after completion
      Validation: command that proves completion
```

---

## Phase 1: Setup — Register Persistence Modules

**Purpose**: Add persistence module declarations to existing crate roots.

**Rationale**: Constitution §H — modify before duplicate. Both `crates/domain/src/` and `crates/infrastructure/src/` already have `persistence/` directories (empty). Tasks register them in their respective `lib.rs`.

- [X] T001 [P] Register persistence module in domain crate
      Action: Modify
      File: crates/domain/src/lib.rs
      Section: module declarations
      Outcome: `pub mod persistence;` added; module table doc updated with persistence entry
      Validation: `cargo check -p ego-domain` passes

- [X] T002 [P] Register persistence module in infrastructure crate
      Action: Modify
      File: crates/infrastructure/src/lib.rs
      Section: module declarations
      Outcome: `pub mod persistence;` added
      Validation: `cargo check -p ego-infrastructure` passes

---

## Phase 2: Domain SPI — Core Contracts (US3, US1, US2, US4)

**Purpose**: Define all persistence SPI types and traits in the domain layer.

**Validation**: `cargo check -p ego-domain` passes after all Phase 2 tasks complete.

- [X] T003 Create persistence module root with re-export structure
      Action: Create
      File: crates/domain/src/persistence/mod.rs
      Section: module root
      Outcome: Module declares submodules (`error`, `event_store`, `repository`, `snapshot`) and re-exports (`PersistenceError`, `EventStore`, `Repository`, `Snapshot`)
      Validation: `cargo check -p ego-domain` passes after all Phase 2 files exist

- [X] T004 [US3] Create PersistenceError enum
      Action: Create
      File: crates/domain/src/persistence/error.rs
      Section: pub enum PersistenceError
      Outcome: `PersistenceError` enum with `NotFound { aggregate_id: String }`, `Conflict { aggregate_id: String, expected: i64, actual: i64 }`, `MissingTenant`, `Internal(String)` variants; derives `Debug, Clone, PartialEq, Eq, thiserror::Error`; implements `Display` for user-facing messages
      Validation: `cargo check -p ego-domain` passes

- [X] T005 [P] [US1] Create EventStore trait
      Action: Create
      File: crates/domain/src/persistence/event_store.rs
      Section: pub trait EventStore<E: DomainEvent>
      Outcome: `EventStore` trait with `append(&mut self, aggregate_id: &str, tenant_id: Option<&str>, expected_version: i64, events: Vec<E>) -> Result<i64, PersistenceError>`, `load(&self, aggregate_id: &str, tenant_id: Option<&str>) -> Result<Vec<E>, PersistenceError>`, `list_aggregate_ids(&self, tenant_id: Option<&str>) -> Result<Vec<String>, PersistenceError>` methods
      Validation: `cargo check -p ego-domain` passes

- [X] T006 [P] [US2] Create Repository trait
      Action: Create
      File: crates/domain/src/persistence/repository.rs
      Section: pub trait Repository<A>
      Outcome: `Repository` trait with `save(&mut self, aggregate: A, tenant_id: Option<&str>, expected_version: i64) -> Result<i64, PersistenceError>`, `load(&self, aggregate_id: &str, tenant_id: Option<&str>) -> Result<A, PersistenceError>`, `delete(&mut self, aggregate_id: &str, tenant_id: Option<&str>) -> Result<(), PersistenceError>` methods
      Validation: `cargo check -p ego-domain` passes

- [X] T007 [P] [US4] Create Snapshot trait
      Action: Create
      File: crates/domain/src/persistence/snapshot.rs
      Section: pub trait Snapshot
      Outcome: `Snapshot` trait with `save_snapshot(&mut self, aggregate_id: &str, tenant_id: Option<&str>, version: i64, payload: serde_json::Value) -> Result<(), PersistenceError>`, `load_snapshot(&self, aggregate_id: &str, tenant_id: Option<&str>) -> Result<Option<(i64, serde_json::Value)>, PersistenceError>` methods
      Validation: `cargo check -p ego-domain` passes

---

## Phase 3: Contract Tests (First-Class Test Suite)

**Purpose**: Define shared contract test helpers that every backend must pass. Tests validate all contract invariants from spec.md §Contract Invariants.

**Validation**: Test files compile. Tests will fail until Phase 4 provides backends — expected per TDD.

- [X] T008 Create shared contract test helpers
      Action: Create
      File: crates/infrastructure/tests/common/mod.rs
      Section: shared test functions
      Outcome: Three pub functions — `event_store_contract_tests()`, `repository_contract_tests()`, `snapshot_contract_tests()` — each validates all invariants from spec.md §Contract Invariants for its trait
      Validation: `cargo check -p ego-infrastructure` passes

- [X] T009 Create EventStore contract test file
      Action: Create
      File: crates/infrastructure/tests/event_store_contract.rs
      Section: integration test
      Outcome: Test file with `mod common;` that calls `common::event_store_contract_tests()` for any EventStore implementation
      Validation: `cargo test -p ego-infrastructure --test event_store_contract --no-run` compiles

- [X] T010 Create Repository contract test file
      Action: Create
      File: crates/infrastructure/tests/repository_contract.rs
      Section: integration test
      Outcome: Test file with `mod common;` that calls `common::repository_contract_tests()` for any Repository implementation
      Validation: `cargo test -p ego-infrastructure --test repository_contract --no-run` compiles

- [X] T011 Create Snapshot contract test file
      Action: Create
      File: crates/infrastructure/tests/snapshot_contract.rs
      Section: integration test
      Outcome: Test file with `mod common;` that calls `common::snapshot_contract_tests()` for any Snapshot implementation
      Validation: `cargo test -p ego-infrastructure --test snapshot_contract --no-run` compiles

---

## Phase 4: InMemory Backend (Reference Implementation)

**Purpose**: Implement in-memory versions of all SPI traits. These serve as reference implementations and pass the contract test suite.

**Validation**: `cargo test -p ego-infrastructure` — all contract tests pass.

- [X] T012 Create infrastructure persistence module root
      Action: Create
      File: crates/infrastructure/src/persistence/mod.rs
      Section: module root
      Outcome: Module declares `pub mod in_memory;` and re-exports its types
      Validation: `cargo check -p ego-infrastructure` passes

- [X] T013 Create in_memory module root
      Action: Create
      File: crates/infrastructure/src/persistence/in_memory/mod.rs
      Section: module root
      Outcome: Module declares submodules (`event_store`, `repository`, `snapshot`) and re-exports (`InMemoryEventStore`, `InMemoryRepository`, `InMemorySnapshotStore`)
      Validation: `cargo check -p ego-infrastructure` passes

- [X] T014 [P] [US1] Implement InMemoryEventStore
      Action: Create
      File: crates/infrastructure/src/persistence/in_memory/event_store.rs
      Section: pub struct InMemoryEventStore<E>
      Outcome: `InMemoryEventStore<E>` implements `EventStore<E>` with tenant-scoped isolation, optimistic concurrency via expected_version, atomic append, NotFound on missing aggregate, MissingTenant on empty Some(""), event ordering preserved
      Validation: `cargo check -p ego-infrastructure` passes

- [X] T015 [P] [US2] Implement InMemoryRepository
      Action: Create
      File: crates/infrastructure/src/persistence/in_memory/repository.rs
      Section: pub struct InMemoryRepository<A>
      Outcome: `InMemoryRepository<A>` implements `Repository<A>` with tenant-scoped isolation, optimistic concurrency, NotFound on missing aggregate, MissingTenant on empty Some("")
      Validation: `cargo check -p ego-infrastructure` passes

- [X] T016 [P] [US4] Implement InMemorySnapshotStore
      Action: Create
      File: crates/infrastructure/src/persistence/in_memory/snapshot.rs
      Section: pub struct InMemorySnapshotStore
      Outcome: `InMemorySnapshotStore` implements `Snapshot` with save_snapshot, load_snapshot returning highest version, no-snapshot returns `Ok(None)`, tenant-scoped isolation
      Validation: `cargo check -p ego-infrastructure` passes

- [X] T017 Wire InMemory backends into contract test files
      Action: Modify
      File: crates/infrastructure/tests/event_store_contract.rs
      Section: #[test] functions
      Outcome: `event_store_contract.rs` instantiates `InMemoryEventStore` and calls `common::event_store_contract_tests()`; same pattern for `repository_contract.rs` (with a concrete aggregate type) and `snapshot_contract.rs`
      Validation: `cargo test -p ego-infrastructure --test event_store_contract --test repository_contract --test snapshot_contract` — all pass

---

## Phase 5: Validation

**Purpose**: Confirm full MVP works end-to-end.

- [X] T018 Run full workspace compilation check
      Action: Validate
      File: workspace root
      Section: workspace
      Outcome: Full workspace compiles without warnings
      Validation: `cargo check --workspace 2>&1` — exits 0, no warnings

- [X] T019 Execute all contract tests against InMemory backends
      Action: Validate
      File: crates/infrastructure/tests/
      Section: contract test suite
      Outcome: All 3 contract test files pass (event_store_contract, repository_contract, snapshot_contract)
      Validation: `cargo test -p ego-infrastructure` — all tests pass

- [X] T020 Run quickstart.md validation
      Action: Validate
      File: specs/001-persistence-spi/quickstart.md
      Section: validation scenarios 1, 2, 4
      Outcome: SPI trait compilation passes, InMemory contract tests pass, multi-tenancy toggle verified per quickstart.md §1, §2, §4
      Validation: Manual verification against quickstart.md checklist — all applicable scenarios pass

---

## Dependencies & Execution Order

```
Phase 1 (Setup) → Phase 2 (Domain SPI) → Phase 3 (Contract Tests) → Phase 4 (InMemory) → Phase 5 (Validation)
```

### Parallel Opportunities

| Phase | [P] Tasks | Rationale |
|-------|-----------|-----------|
| Phase 1 | T001, T002 | Different crates, no shared state |
| Phase 2 | T005, T006, T007 | Different files, depend only on T004 (PersistenceError) |
| Phase 3 | T009, T010, T011 | Different test files, share T008 (common helpers) |
| Phase 4 | T014, T015, T016 | Different implementations, no cross-dependencies |

### Execution Strategy

1. **T001–T002** in parallel (different crates)
2. **T003** (module root) then **T004** (PersistenceError) — sequential, T003 declares T004's file
3. **T005, T006, T007** in parallel after T004
4. **T008** (helpers) then **T009, T010, T011** in parallel
5. **T012–T013** (module roots) then **T014, T015, T016** in parallel
6. **T017** (wire InMemory into tests) — sequential, depends on all InMemory implementations
7. **T018–T020** — sequential validation

---

## Definition of Done (MVP)

- [X] `cargo check --workspace` passes (2 pre-existing warnings in `ego-runtime`/`ego-runtime-tokio`, unrelated to this spec)
- [X] `cargo test -p ego-infrastructure` — all contract tests pass for InMemory backends
- [X] `PersistenceError`, `EventStore`, `Repository`, `Snapshot` exist in `ego-domain`
- [X] `InMemoryEventStore`, `InMemoryRepository`, `InMemorySnapshotStore` exist in `ego-infrastructure`
- [X] Contract test suite validates ordering, atomicity, concurrency, consistency, error translation, empty-state behavior, tenant isolation per spec.md §Contract Invariants
- [X] No PostgreSQL, migration, or schema evolution code exists in this spec's scope
- [ ] `StoredEvent<E>` wrapper exists in `ego-domain::persistence`
- [ ] EventStore trait uses `Vec<StoredEvent<E>>` for append/load
- [ ] Correlation ID contract tests cover: preservation, None default, mixed batch

---

## Phase 6: Correlation ID Propagation (US7)

**Purpose**: Add optional `correlation_id` to the event envelope through the `StoredEvent<E>` wrapper. Modify EventStore trait, InMemory and PostgreSQL backends, and add contract tests for correlation preservation.

**Validation**: `cargo test -p ego-infrastructure` — all existing + new correlation tests pass.

- [ ] T021 [P] [US7] Create StoredEvent<E> wrapper type in domain persistence
      Action: Create
      File: crates/domain/src/persistence/stored_event.rs
      Section: pub struct StoredEvent<E>
      Outcome: `StoredEvent<E>` struct with two fields — `event: E` and `correlation_id: Option<String>` — derives `Debug, Clone, PartialEq, Eq`; added to `persistence/mod.rs` re-exports. No `DomainEvent` trait constraint — `E` is generic.
      Validation: `cargo check -p ego-domain` passes

- [ ] T022 [P] [US7] Update EventStore trait signature to use StoredEvent<E>
      Action: Modify
      File: crates/domain/src/persistence/event_store.rs
      Section: pub trait EventStore<E: DomainEvent>
      Outcome: `append` accepts `Vec<StoredEvent<E>>` instead of `Vec<E>`; `load` returns `Result<Vec<StoredEvent<E>>` instead of `Result<Vec<E>>`. Trait bound `E: DomainEvent` unchanged.
      Validation: `cargo check -p ego-domain` passes

- [ ] T023 [P] [US7] Update InMemoryEventStore to store and return correlation_id
      Action: Modify
      File: crates/infrastructure/src/persistence/in_memory/event_store.rs
      Section: InMemoryEventStore<E>
      Outcome: Internal storage wraps events with correlation_id. `append` preserves each event's `correlation_id`; `load` returns events with their stored `correlation_id`. `list_aggregate_ids` unchanged (aggregate-level, not event-level).
      Validation: `cargo check -p ego-infrastructure` passes

- [ ] T024 [P] [US7] Add correlation_id contract tests to shared test suite
      Action: Modify
      File: crates/infrastructure/tests/common/mod.rs
      Section: event_store_contract_tests()
      Outcome: Three new test scenarios added to `event_store_contract_tests()`: 1) append with correlation_id → load returns same correlation_id, 2) append without correlation_id → load returns None, 3) batch append with mixed correlation_ids → each preserved individually. All run for any EventStore implementation.
      Validation: `cargo test -p ego-infrastructure --test event_store_contract --no-run` compiles

- [ ] T025 [P] [US7] Run full test suite with correlation_id changes
      Action: Validate
      File: workspace root
      Section: test suite
      Outcome: All existing contract tests plus new correlation_id tests pass for InMemoryEventStore.
      Validation: `cargo test -p ego-infrastructure` — all tests pass

- [ ] T026 [P] [US7] Update quickstart.md correlation propagation scenario
      Action: Validate
      File: specs/001-persistence-spi/quickstart.md
      Section: §4
      Outcome: quickstart.md §4 (Correlation ID propagation) compiles and runs correctly.
      Validation: Manual verification — copy/paste quickstart §4 code into a test, compile and run

---

## Notes

- Per `.speckit/constitution.md` §F: every task includes Action, File, Section, Outcome, Validation
- Per `.speckit/constitution.md` §C: PostgreSQL backend, migration infrastructure, schema evolution are deferred to separate specs
- Per `.speckit/constitution.md` §A: InMemory backends use the simplest valid storage — no speculative optimizations
- Per `.speckit/constitution.md` §H: `crates/domain/src/lib.rs` and `crates/infrastructure/src/lib.rs` are modified (not replaced) — only persistence module declarations added
- Contract test suite is the quality bar: passing it = compliant backend
- Future backend implementations (PostgreSQL) must pass the same contract suite
