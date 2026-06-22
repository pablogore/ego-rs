# Tasks: CORE-006 Persistent Entity Runtime Implementation

**Input**: Design documents from `/specs/006-persistent-entity-runtime/final-consistency-lock/`

**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md, contracts/

**Target**: `crates/persistent-entity/` — existing implementation refinement + new modules

---

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Ensure the crate is properly integrated and types are defined

- [ ] T001 Add `ego-domain` dependency to `crates/persistent-entity/Cargo.toml` — declare path dependency on `../../crates/domain` for `DomainEvent`, `EventStore`, `Snapshot`, `Repository` traits
- [ ] T002 [P] Add `blake3` dependency to `crates/persistent-entity/Cargo.toml` for ExecutionKey hashing

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core types and traits that all user stories depend on. MUST complete before any US work.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T003 Create `crates/persistent-entity/src/types.rs` — define `EntityId`, `EntityTriple`, `ExecutionKey` (with `hash::compute()` using blake3), `TenantId` type aliases. Re-export from lib.rs. Migrate `EntityTriple` from `scheduler.rs`.
- [ ] T004 [P] Refactor `crates/persistent-entity/src/persistent_entity.rs` — verify `PersistentEntity<C,E,S>` trait matches data-model.md: `handle_command`, `apply_event`, `initial_state`. Ensure `async_trait` usage is correct. Add `DomainEvent` bound on `Event` type. Remove any identity-related types from trait bounds (FR-EI-001).
- [ ] T005 [P] Refactor `crates/persistent-entity/src/command_context.rs` — verify `CommandContext` has `tenant_id`, `correlation_id`, `causation_id`, `approval_id`, `metadata`. Add `Clone` derive needed for execution flow.
- [ ] T006 [P] Refactor `crates/persistent-entity/src/command_envelope.rs` — ensure `CommandEnvelope<C>` includes `EntityId`, `command: C`, `context: CommandContext`, `expected_version: u64`. Add `Debug` derive.
- [ ] T007 [P] Refine `crates/persistent-entity/src/error.rs` — ensure all error variants exist: `EntityNotFound`, `VersionConflict`, `MailboxFull`, `EntityPassivating`, `ReentrancyNotAllowed`, `HandlerError`, `ApplierError`, `PersistenceError`, `BackendError`, `UnknownEntityType`.
- [ ] T008 Refactor `crates/persistent-entity/src/lifecycle.rs` — verify `LifecycleStateMachine` has 5 states (`Recovering`, `Active`, `Passivating`, `Passivated`, `Failed`) with transitions per canonical spec Section 4: `Recovering→Active` (auto), `Active→Passivating` (policy), `Passivating→Passivated` (irreversible drain), `Passivated→Recovering` (reactivation), `Active→Failed` (irrecoverable error), `Failed→Recovering` (admin). Forbid `Passivating→Active`.

**Checkpoint**: Foundation ready — types and traits stable. User stories can now begin.

---

## Phase 3: User Story 1 — Define an Event-Sourced Entity (Priority: P1) 🎯 MVP

**Goal**: Developer can define a Counter entity implementing `PersistentEntity` and see it compile with the framework.

**Independent Test**: Define `TestCounter` entity with `Increment`/`Decrement` commands, verify `handle_command` produces `Incremented`/`Decremented` events, verify `apply_event` produces correct state, verify business rule (decrement below zero) returns error. Run `cargo test -- test_entity_definition`.

### Implementation for User Story 1

- [ ] T009 [P] [US1] Refactor `crates/persistent-entity/src/test_entity.rs` — update `TestCommand`, `TestEvent`, `TestState` to match the Counter entity from canonical spec US1: `Increment(u64)`/`Decrement(u64)` commands, `Incremented(u64)`/`Decremented(u64)` events, `u64` state. Add `TestError` for business rule violation (decrement below zero).
- [ ] T010 [US1] Implement `PersistentEntity` for Counter in `crates/persistent-entity/src/test_entity.rs` — `handle_command` produces events or error, `apply_event` updates state, `initial_state` returns 0. Handler Safety Contract: no async I/O, pure function.
- [ ] T011 [US1] Add unit test `test_counter_handler_produces_events` in `crates/persistent-entity/tests/common/mod.rs` — verify `Increment(1)` produces `vec![Incremented(1)]`, verify `Decrement(10)` at state 5 returns error.
- [ ] T012 [US1] Add unit test `test_counter_applier_evolves_state` in `crates/persistent-entity/tests/common/mod.rs` — verify applying `Incremented(1)` to state 0 → 1, applying `Decremented(1)` to state 5 → 4.
- [ ] T013 [US1] Add unit test `test_counter_initial_state_is_zero` in `crates/persistent-entity/tests/common/mod.rs`

**Checkpoint**: Entity definition trait and test entity fully functional. Developer can define any event-sourced entity.

---

## Phase 4: User Story 2 — Send Commands to an Entity (Priority: P1) 🎯 MVP

**Goal**: Developer sends commands via `EntityRef`, commands flow through mailbox, Actor processes them, response returned.

**Independent Test**: Build an `EntityRuntime` with in-memory backend, send `Increment(1)` to a Counter entity, verify response. Send two sequential commands, verify second sees first's state. Send to full mailbox, verify `MailboxFull`. Run `cargo test -- test_command_send`.

### Implementation for User Story 2

- [ ] T014 [US2] Create `crates/persistent-entity/src/execution_backend.rs` — define `ExecutionBackend` sync trait per `contracts/execution-backend.md`: `fn execute<C,E,S>(&self, entity, state, command, context) -> Result<(Vec<E>, S), EntityError>`. Trait must be `Debug + Send + Sync`. No async, no backend-specific types.
- [ ] T015 [P] [US2] Create `crates/persistent-entity/src/execution_backend_tokio.rs` — implement `TokioExecutionBackend` (default) as zero-overhead wrapper. Use `futures::executor::block_on` to bridge async trait to sync call. Re-export from lib.rs.
- [ ] T016 [P] [US2] Refactor `crates/persistent-entity/src/mailbox.rs` — ensure `BoundedMailbox` uses `tokio::sync::mpsc::channel(capacity)` with exposed `Sender` (cloned for EntityRef) and `Receiver` (owned by Actor). `try_send` must return `Result<(), MailboxFull>`. Channel close provides stale sender detection.
- [ ] T017 [P] [US2] Refine `crates/persistent-entity/src/activation.rs` — `SharedActivation` must be `Arc<tokio::sync::Mutex<ActivationState>>`. Add `try_activate()` returning `Result<ActivationGuard, AlreadyActivating>`. Add `register_sender()` and `remove_sender()` for mailbox handle lifecycle. This is the single-flight spawn guard per entity.
- [ ] T018 [P] [US2] Refine `crates/persistent-entity/src/registry.rs` — `EntityRegistry` must hold: `active: HashMap<EntityTriple, mpsc::Sender<CommandEnvelope>>`, `passivated: HashMap<EntityTriple, u64>` (last known version), `activations: HashMap<EntityTriple, Arc<SharedActivation>>`. Add `get_or_create_activation()` — atomically insert or return existing `SharedActivation` for single-flight guarantee. Add `register_active()`, `register_passivated()`, `remove_active()`.
- [ ] T019 [US2] Create `crates/persistent-entity/src/scheduler_policy.rs` — define `SchedulingPolicy` trait per `contracts/scheduling-policy.md`: `fn select_next(pending, budget_available) -> Option<EntityTriple>`, `fn budget_size()`, `fn fairness_window()`. Implement `RoundRobinPolicy::new(budget_size, fairness_window)` — per research.md decision. Policy is stateless (state held externally by Scheduler).
- [ ] T020 [US2] Create `crates/persistent-entity/src/scheduler_event.rs` — define `SchedulerEvent` enum: `SlotFreed`, `CommandArrived(EntityTriple)`, `CircuitBreakerExpired(EntityTriple)`. Define `SchedulerTrigger` wrapping `tokio::sync::Notify` for event-driven wakeup.
- [ ] T021 [US2] Rewrite `crates/persistent-entity/src/scheduler.rs` — `Scheduler` struct holds: `activation_queue: VecDeque<EntityTriple>`, `fairness_tracker: HashMap<EntityTriple, u64>`, `active_count: usize`, `policy: Box<dyn SchedulingPolicy>`, `trigger: Arc<SchedulerTrigger>`, `registry: Arc<EntityRegistry>`. Implement `on_command_arrived(entity)` — adds to queue, fires trigger. Implement `on_slot_freed()` — fires trigger. Implement `run_decision_cycle()` — woken by trigger, calls `policy.select_next()`, updates fairness tracker (increments all pending entities' wait count, resets on activation). Implement `try_activate_next()` — calls `registry.get_or_create_activation()`, calls `try_activate()` on SharedActivation (budget check at guard), spawns `EntityActor::run()` on success. Scheduler NEVER executes — only proposes.
- [ ] T022 [US2] Refactor `crates/persistent-entity/src/entity_actor.rs` — restructure `EntityActor<C,E,S>` struct per data-model.md: fields include `entity_id`, `lifecycle: LifecycleStateMachine`, `mailbox: BoundedMailboxReceiver`, `backend: Arc<dyn ExecutionBackend>`, `persistence: Arc<dyn PersistenceFacade<E>>`, `handler: Arc<dyn PersistentEntity<...>>`, `snapshot_strategy`, `seen_keys: HashSet<ExecutionKey>`, `_budget_guard: OwnedSemaphorePermit`. Implement `EntityActor::run()`: (1) set lifecycle to `Recovering`, (2) call `recover()` synchronously, (3) set lifecycle to `Active`, (4) enter `process_commands()` loop `while let Some(env) = rx.recv().await`, (5) for each command: compute `ExecutionKey`, check dedup, call `backend.execute()` → if events: persist → apply → snapshot → publish → respond; if zero events: respond immediately, (6) on passivation trigger: set `Passivating`, drain mailbox, serialize state, register passivated, drop _budget_guard (frees slot), task completes. (7) On mailbox close: task exits.
- [ ] T023 [US2] Refine `crates/persistent-entity/src/persistence.rs` — replace stub `PersistenceFacade<E>` with real implementation delegating to `ego-domain::persistence::EventStore<E>` and `ego-domain::persistence::Snapshot`. Implement `load_for_recovery(entity_id) -> (Option<(State, u64)>, Vec<E>)` — loads snapshot + version, loads events with seq > version. Implement `persist_events(entity_id, version, events) -> Result<u64, PersistenceError>` — delegates to `EventStore::append()` with optimistic concurrency. Implement `store_snapshot(entity_id, state, version)`.
- [ ] T024 [US2] Refine `crates/persistent-entity/src/recovery.rs` — implement `EntityActor::recover()`: (1) call `persistence.load_for_recovery()`, (2) if snapshot exists: deserialize state, extract snapshot version, (3) iterate events with `seq > snapshot_version` in order, (4) for each: call `handler.apply_event()` synchronously, (5) return reconstructed `(state, version)`. No async, no scheduler, no backend. Recovery is pure synchronous replay inside Actor.
- [ ] T025 [P] [US2] Refactor `crates/persistent-entity/src/entity_ref.rs` — `EntityRef<C,E,S>` holds `sender: mpsc::Sender<CommandEnvelope<C>>`, `registry: Arc<EntityRegistry>`, `entity_id: EntityId`. Method `send(command, context) -> Result<Response, EntityError>`: (1) check `registry` for active sender, (2) if found: `try_send` → `MailboxFull` on full, (3) if not found (stale/closed): check passivated registry → trigger reactivation via `registry.get_or_create_activation()` → spawn via Scheduler → retry send, (4) if not in passivated: `EntityNotFound`. Add `responder: oneshot::Sender` pattern in `CommandEnvelope` for async response.
- [ ] T026 [US2] Refactor `crates/persistent-entity/src/runtime.rs` — `EntityRuntime` top-level holds: `registry: Arc<EntityRegistry>`, `scheduler: Arc<Scheduler>`, `backend: Arc<dyn ExecutionBackend>`, `persistence: Arc<dyn PersistenceFacade>`, `publisher: Arc<dyn EventPublisher>`, `passivation_policy: PassivationPolicy`. `run()` starts the Scheduler event loop. `entity_ref(entity_id) -> EntityRef` creates a sender handle.
- [ ] T027 [US2] Refactor `crates/persistent-entity/src/builder.rs` — `EntityRuntimeBuilder` configures: `backend`, `persistence`, `publisher`, `mailbox_capacity`, `budget_size`, `fairness_window`, `passivation_timeout`, `snapshot_strategy`. `build()` creates `EntityRegistry`, `Scheduler` with `RoundRobinPolicy`, spawns Scheduler event loop, returns `EntityRuntime`.
- [ ] T028 [US2] Add integration test `test_full_command_lifecycle` in `crates/persistent-entity/tests/common/mod.rs` — build runtime with in-memory stores, register Counter entity, send `Increment(5)`, verify response state = 5, send `Increment(3)`, verify state = 8, send `Decrement(10)`, verify error.
- [ ] T029 [US2] Add integration test `test_mailbox_full_rejection` in `crates/persistent-entity/tests/common/mod.rs` — set mailbox_capacity=1, send 3 commands without awaiting, verify third receives `MailboxFull`.
- [ ] T030 [US2] Add integration test `test_entity_not_found` in `crates/persistent-entity/tests/common/mod.rs` — send command to non-existent entity, verify `EntityNotFound`.
- [ ] T031 [US2] Add integration test `test_reentrancy_prevention` in `crates/persistent-entity/tests/common/mod.rs` — handler attempts to send command to own entity, verify `ReentrancyNotAllowed`.
- [ ] T032 [US2] Add integration test `test_zero_event_query` in `crates/persistent-entity/tests/common/mod.rs` — send read-only command, verify version unchanged, no events persisted, no publication.
- [ ] T033 [US2] Add integration test `test_execution_deduplication` in `crates/persistent-entity/tests/common/mod.rs` — send same (command, version) twice, verify ExecutionKey-based dedup prevents double execution. Verify zero-event commands are NOT deduped (re-executed).
- [ ] T034 [US2] Register in `crates/persistent-entity/src/lib.rs` — add `pub mod execution_backend;`, `pub mod execution_backend_tokio;`, `pub mod scheduler_policy;`, `pub mod scheduler_event;`, `pub mod types;`. Re-export key types: `EntityRuntime`, `EntityRuntimeBuilder`, `EntityRef`, `PersistentEntity`, `ExecutionBackend`, `TokioExecutionBackend`, `SchedulingPolicy`, `RoundRobinPolicy`, `EntityId`, `EntityTriple`, `ExecutionKey`, `CommandContext`, `CommandEnvelope`, `EntityError`.

**Checkpoint**: Full command lifecycle working — entity definition → send → execute → persist → respond. Actor-per-entity, mailbox FIFO, bounded backpressure, execution deduplication all functional.

---

## Phase 5: User Story 3 — Entity Recovery After Restart (Priority: P2)

**Goal**: Entity state recovers correctly after restart via snapshot + event replay.

**Independent Test**: Process commands to build state, simulate restart (clear in-memory state), send command, verify state matches pre-restart. Run `cargo test -- test_recovery_after_restart`.

### Implementation for User Story 3

- [ ] T035 [US3] Add integration test `test_recovery_with_snapshot` in `crates/persistent-entity/tests/recovery_tests.rs` — process 50 Increment commands, snapshot at version 50, restart, verify state = 50.
- [ ] T036 [US3] Add integration test `test_recovery_without_snapshot` in `crates/persistent-entity/tests/recovery_tests.rs` — process 100 events, no snapshot, restart, verify full replay from version 0 produces correct state.
- [ ] T037 [US3] Add integration test `test_recovery_replay_equals_live` in `crates/persistent-entity/tests/recovery_tests.rs` — compare state after 100 live commands with state after recovery replay of those 100 events. Verify identical.
- [ ] T038 [US3] Add integration test `test_corrupted_snapshot_fallback` in `crates/persistent-entity/tests/recovery_tests.rs` — corrupt snapshot data, restart, verify fallback to full event replay.
- [ ] T039 [US3] Add integration test `test_recovery_during_command_arrival` in `crates/persistent-entity/tests/recovery_tests.rs` — entity recovering, commands arrive during recovery, verify queued in mailbox, processed after ACTIVE transition.

**Checkpoint**: Recovery works — snapshot + replay, full replay fallback, corrupted snapshot handling, commands during recovery.

---

## Phase 6: User Story 4 — Configure Snapshot Strategy (Priority: P2)

**Goal**: Developer configures snapshot strategy (never, every N events, custom).

**Independent Test**: Configure strategy "every 10 events", process 15 events, verify snapshot stored at version 10. Run `cargo test -- test_snapshot_strategy`.

### Implementation for User Story 4

- [ ] T040 [US4] Refine `crates/persistent-entity/src/snapshot.rs` — `SnapshotStrategy` enum: `Never`, `Every(u64)` (every N events), `Custom(Box<dyn Fn(u64, u64) -> bool>)`. Implement `should_snapshot(current_version, last_snapshot_version) -> bool` for each variant.
- [ ] T041 [US4] Add integration test `test_snapshot_every_n` in `crates/persistent-entity/tests/common/mod.rs` — strategy `Every(10)`, process 15 commands, verify snapshot at version 10, not at 15.
- [ ] T042 [US4] Add integration test `test_snapshot_never` in `crates/persistent-entity/tests/common/mod.rs` — strategy `Never`, process 100 commands, verify no snapshot stored.
- [ ] T043 [US4] Add integration test `test_snapshot_custom` in `crates/persistent-entity/tests/common/mod.rs` — custom strategy: snapshot on even versions, verify snapshots at versions 2, 4, 6 but not 1, 3, 5.

**Checkpoint**: All three snapshot strategy types work.

---

## Phase 7: User Story 5 — Multi-Tenant Entity Isolation (Priority: P2)

**Goal**: Same entity ID in different tenants = independent entities.

**Independent Test**: Create "acc-1" in tenant A and tenant B, send different commands, verify independent states. Run `cargo test -- test_multi_tenant`.

### Implementation for User Story 5

- [ ] T044 [US5] Add integration test `test_tenant_isolation` in `crates/persistent-entity/tests/common/mod.rs` — entity "e1" in tenant "A" with state 10, entity "e1" in tenant "B" with initial state, verify no cross-tenant state leakage.
- [ ] T045 [US5] Add integration test `test_tenant_independent_concurrency` in `crates/persistent-entity/tests/common/mod.rs` — concurrent commands to same entity ID in different tenants, verify both proceed without blocking each other.
- [ ] T046 [US5] Add integration test `test_single_tenant_default_scope` in `crates/persistent-entity/tests/common/mod.rs` — no tenant specified, verify entities operate in default scope.

**Checkpoint**: Multi-tenant isolation verified.

---

## Phase 8: User Story 6 — Event Publication (Priority: P3)

**Goal**: Events published via SPI after persistence confirms.

**Independent Test**: Send command producing events, verify events persisted before published, verify consumer receives events. Run `cargo test -- test_event_publication`.

### Implementation for User Story 6

- [ ] T047 [US6] Refine `crates/persistent-entity/src/publisher.rs` — `EventPublisher<E>` trait: `async fn publish(&self, events: &[E], entity_id: &EntityId) -> Result<(), PublishError>`. Add `InMemoryEventPublisher` for testing — records published events in a `Vec<Arc<Mutex<Vec<E>>>>`.
- [ ] T048 [US6] Add integration test `test_events_published_after_persist` in `crates/persistent-entity/tests/common/mod.rs` — send command, verify EventPublisher is called AFTER persistence confirms. Verify publication does NOT happen if persist fails.
- [ ] T049 [US6] Add integration test `test_persist_before_publish_ordering` in `crates/persistent-entity/tests/common/mod.rs` — instrument persist and publish calls, verify persist timestamp < publish timestamp.

**Checkpoint**: Event publication SPI works. Persist-before-publish ordering enforced.

---

## Phase 9: User Story 7 — Concurrent Modification Conflicts (Priority: P3)

**Goal**: VersionConflict returned when optimistic concurrency check fails.

**Independent Test**: Two concurrent command streams to same entity, verify one succeeds, other gets VersionConflict. Run `cargo test -- test_version_conflict`.

### Implementation for User Story 7

- [ ] T050 [US7] Verify `PersistenceFacade::persist_events()` in `crates/persistent-entity/src/persistence.rs` implements optimistic concurrency: expects `expected_version`, append fails if current version != expected, returns `VersionConflict`.
- [ ] T051 [US7] Add integration test `test_version_conflict` in `crates/persistent-entity/tests/common/mod.rs` — two command streams with same expected_version, verify exactly one succeeds, other gets `VersionConflict`.
- [ ] T052 [US7] Add integration test `test_version_conflict_retry_succeeds` in `crates/persistent-entity/tests/common/mod.rs` — command gets VersionConflict, retry with updated version, verify retry succeeds.

**Checkpoint**: Optimistic concurrency works. VersionConflict handling verified.

---

## Phase 10: Polish & Cross-Cutting Concerns

**Purpose**: System-level verification, edge cases, and cleanup

- [ ] T053 [P] Add integration test `test_concurrent_entities_parallel` in `crates/persistent-entity/tests/common/mod.rs` — commands to 10 different entities, verify all process in parallel (different tasks, independent mailboxes).
- [ ] T054 [P] Add integration test `test_concurrency_budget_enforcement` in `crates/persistent-entity/tests/common/mod.rs` — budget=2, activate 5 entities, verify at most 2 processing concurrently, all 5 eventually complete.
- [ ] T055 [P] Add integration test `test_fairness_no_starvation` in `crates/persistent-entity/tests/common/mod.rs` — entity A gets 1000 commands/sec, entity B gets 1 command. Verify B processes before A's 1001st command under RoundRobinPolicy with fairness window.
- [ ] T056 [P] Add integration test `test_passivation_reactivation` in `crates/persistent-entity/tests/common/mod.rs` — let entity passivate, send new command, verify transparent reactivation (single-flight, no duplicate spawn).
- [ ] T057 [P] Add integration test `test_command_during_passivating` in `crates/persistent-entity/tests/common/mod.rs` — entity is PASSIVATING, send command, verify `EntityPassivating` error. Send again after passivation, verify success.
- [ ] T058 [P] Add integration test `test_deterministic_applier_bug` in `crates/persistent-entity/tests/common/mod.rs` — entity with buggy applier at event #50, persist 100 events, verify entity → FAILED. Recovery replays events 1-50, reproduces identical applier panic.
- [ ] T059 [P] Add integration test `test_backend_determinism` in `crates/persistent-entity/tests/common/mod.rs` — execute same commands through TokioBackend and a test backend, verify identical events and state.
- [ ] T060 [P] Add integration test `test_single_flight_activation` in `crates/persistent-entity/tests/common/mod.rs` — 100 concurrent commands to PASSIVATED entity, verify exactly 1 actor spawned, all 100 processed sequentially.
- [ ] T061 Verify ALL existing tests pass: `cargo test -p persistent-entity`
- [ ] T062 Update `crates/persistent-entity/src/lib.rs` final re-exports — ensure public API is clean: `EntityRuntime`, `EntityRuntimeBuilder`, `EntityRef`, `PersistentEntity`, `ExecutionBackend`, `TokioExecutionBackend`, `SchedulingPolicy`, `RoundRobinPolicy`, `SnapshotStrategy`, `EntityError`, `CommandContext`, `EntityId`, `EntityTriple`, `ExecutionKey`.
- [ ] T063 [P] Run quickstart.md validation — all 10 scenarios from `specs/006-persistent-entity-runtime/final-consistency-lock/quickstart.md` pass with `cargo test`.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — start immediately
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories
- **US1 (Phase 3)**: Depends on Foundational. Entity definition is prerequisite for sending commands.
- **US2 (Phase 4)**: Depends on Foundational + US1 (needs test entity). Core command lifecycle.
- **US3 (Phase 5)**: Depends on US2 (needs persistence). Recovery testing.
- **US4 (Phase 6)**: Depends on US2 (needs snapshot infrastructure).
- **US5 (Phase 7)**: Depends on US2 (needs multi-tenant entity_id scope).
- **US6 (Phase 8)**: Depends on US2 (needs command lifecycle with publish step).
- **US7 (Phase 9)**: Depends on US2 (needs persistence with optimistic concurrency).
- **Polish (Phase 10)**: Depends on all desired user stories being complete.

### User Story Dependency Graph

```
Phase 1 (Setup)
    ↓
Phase 2 (Foundational)
    ↓
Phase 3 (US1: Entity Definition)
    ↓
Phase 4 (US2: Command Lifecycle) ← ALL subsequent stories depend here
    ├── Phase 5 (US3: Recovery)
    ├── Phase 6 (US4: Snapshots)
    ├── Phase 7 (US5: Multi-Tenant)
    ├── Phase 8 (US6: Publication)
    └── Phase 9 (US7: Concurrency)
         ↓
Phase 10 (Polish)
```

### Parallel Opportunities

- T001 + T002 can run in parallel (different Cargo.toml sections)
- T003-T007 all in Phase 2 are parallel (different files)
- T014, T015, T016 in US2 are parallel (different files)
- T035-T039 in US3 are parallel (different test functions, same test file)
- T040-T043 in US4 — T040 is a prerequisite, T041-T043 can run in parallel
- US5 (T044-T046) and US6 (T047-T049) and US7 (T050-T052) can run in parallel after US2 is complete (different concerns)
- T053-T058 in Polish are all parallel (different test files/functions)

### Within Phase 4 (US2) — Critical Path

T014 (ExecutionBackend trait) → T019 (SchedulingPolicy trait) → T020 (SchedulerEvent) → T021 (Scheduler rewrite) → T026 (EntityRuntime) → T027 (EntityRuntimeBuilder) → T028-T033 (integration tests)

T018 (EntityRegistry) must complete before T021 (Scheduler) and T025 (EntityRef)

T023 (PersistenceFacade) must complete before T024 (Recovery) and before T022 (EntityActor)

---

## Parallel Example: Phase 4 (US2) Core Modules

```bash
# Launch core trait definitions in parallel:
Task T014: "Create ExecutionBackend trait in execution_backend.rs"
Task T015: "Create TokioExecutionBackend in execution_backend_tokio.rs"  
Task T016: "Refine BoundedMailbox in mailbox.rs"
Task T017: "Refine SharedActivation in activation.rs"
Task T018: "Refine EntityRegistry in registry.rs"
Task T019: "Create SchedulingPolicy trait in scheduler_policy.rs"
Task T020: "Create SchedulerEvent in scheduler_event.rs"

# Then (sequential):
Task T021: "Rewrite Scheduler in scheduler.rs" (depends on T018, T019, T020)
Task T023: "Refine PersistenceFacade in persistence.rs"
Task T024: "Refine EntityActor recovery in recovery.rs" (depends on T023)
Task T022: "Refactor EntityActor in entity_actor.rs" (depends on T014, T016, T023)
Task T025: "Refactor EntityRef in entity_ref.rs" (depends on T018)
Task T026: "Refactor EntityRuntime in runtime.rs" (depends on T021)
Task T027: "Refactor EntityRuntimeBuilder in builder.rs" (depends on T026)
```

---

## Implementation Strategy

### MVP First (US1 + US2)

1. Complete Phase 1: Setup → Phase 2: Foundational
2. Complete Phase 3: US1 (Entity Definition)
3. Complete Phase 4: US2 (Command Lifecycle) — this is the largest phase
4. **STOP and VALIDATE**: Run all US1 + US2 tests. Verify full command lifecycle works.
5. This is a deployable/demo-able MVP: define entity, send commands, get results.

### Incremental Delivery

1. Setup + Foundational + US1 + US2 → MVP: basic entity runtime works
2. Add US3 (Recovery) → entities survive restarts
3. Add US4 (Snapshots) → recovery performance optimization
4. Add US5 (Multi-Tenant) → production tenant isolation
5. Add US6 (Publication) → downstream consumers react
6. Add US7 (Concurrency) → version conflict handling
7. Polish → stress tests, edge cases, quickstart validation

### Estimated Task Times

- Phase 1-2 (Setup + Foundational): ~2-3 hours (mostly refinement of existing code)
- Phase 3 (US1): ~1 hour
- Phase 4 (US2): ~8-12 hours (largest phase — new trait + Scheduler rewrite + Actor refactor + EntityRuntime wiring + integration tests)
- Phase 5-9 (US3-US7): ~1-2 hours each (mostly integration tests)
- Phase 10 (Polish): ~2-3 hours

**Total**: ~20-30 hours

---

## Notes

- [P] tasks = different files, no dependencies — can run in parallel
- [Story] label maps task to specific user story for traceability
- All paths relative to `/Users/pablogore/workspace/pablogore/ego-rs/`
- Existing code in `crates/persistent-entity/` MUST be refactored, not rewritten
- `PersistentEntity::handle_command` is `async_trait` for Rust compatibility — the ExecutionBackend bridges via `block_on`
- The Scheduler rewrite (T021) is the highest-risk task — most interdependencies
- `cargo test -p persistent-entity` after every phase to catch regressions early
