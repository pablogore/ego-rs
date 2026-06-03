# Tasks: PostgreSQL Persistence Backend

**Spec**: [004-postgresql-backend](../004-postgresql-backend/spec.md)
**Design**: [plan.md](plan.md)

## Phase 2: Task Breakdown

### T001 [US1] Add sqlx dependency to infrastructure crate
- [ ] T001 [US1] Modify crates/infrastructure/Cargo.toml
      Action: Modify
      File: crates/infrastructure/Cargo.toml
      Section: [dependencies]
      Outcome: sqlx with postgres and runtime-tokio features added
      Validation: cargo check -p ego-infrastructure passes

### T002 [US1] Create PostgreSQL persistence module structure
- [ ] T002 [US1] Create crates/infrastructure/src/persistence/postgresql/mod.rs
      Action: Create
      File: crates/infrastructure/src/persistence/postgresql/mod.rs
      Section: module declarations
      Outcome: Module file with event_store, repository, snapshot submodules declared
      Validation: cargo check -p ego-infrastructure passes

- [ ] T003 [US1] Modify crates/infrastructure/src/persistence/mod.rs
      Action: Modify
      File: crates/infrastructure/src/persistence/mod.rs
      Section: pub mod declarations
      Outcome: postgresql module exported alongside in_memory
      Validation: cargo check -p ego-infrastructure passes

### T004 [US2] Implement PostgreSQL EventStore
- [ ] T004 [US2] Create crates/infrastructure/src/persistence/postgresql/event_store.rs
      Action: Create
      File: crates/infrastructure/src/persistence/postgresql/event_store.rs
      Section: pub struct PostgreSQLEventStore, impl EventStore
      Outcome: PostgreSQLEventStore with append, load, list_aggregate_ids using sqlx PgPool
      Validation: cargo check -p ego-infrastructure passes

### T005 [US2] Implement PostgreSQL Repository
- [ ] T005 [US2] Create crates/infrastructure/src/persistence/postgresql/repository.rs
      Action: Create
      File: crates/infrastructure/src/persistence/postgresql/repository.rs
      Section: pub struct PostgreSQLRepository, impl Repository
      Outcome: PostgreSQLRepository with save, load, delete using sqlx PgPool
      Validation: cargo check -p ego-infrastructure passes

### T006 [US2] Implement PostgreSQL Snapshot Store
- [ ] T006 [US2] Create crates/infrastructure/src/persistence/postgresql/snapshot.rs
      Action: Create
      File: crates/infrastructure/src/persistence/postgresql/snapshot.rs
      Section: pub struct PostgreSQLSnapshotStore, impl Snapshot
      Outcome: PostgreSQLSnapshotStore with save_snapshot, load_snapshot using sqlx PgPool
      Validation: cargo check -p ego-infrastructure passes

### T007 [US3] Verify compilation and contract tests
- [ ] T007 [US3] Run cargo check -p ego-infrastructure
      Action: Execute
      File: N/A
      Section: compilation check
      Outcome: All PostgreSQL modules compile without errors
      Validation: cargo check -p ego-infrastructure passes with no errors
