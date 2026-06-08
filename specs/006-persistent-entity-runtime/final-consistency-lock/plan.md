# Implementation Plan: CORE-006 Persistent Entity Runtime

**Branch**: `008-scheduling-policy` | **Date**: 2026-06-07 | **Spec**: [spec.md](spec.md)

**Input**: Final Consistency Lock specification from `/specs/006-persistent-entity-runtime/final-consistency-lock/spec.md`

## Summary

CORE-006 is a fully specified deterministic event-sourced actor-per-entity runtime kernel. The system has reached FINAL CONSISTENCY LOCK: all 6 architectural gaps resolved, all sub-specs compose coherently. The existing `persistent-entity` crate provides a working foundation (EntityActor, EntityRegistry, BoundedMailbox, LifecycleFSM, EntityRuntime) but needs refactoring to align with the finalized model — specifically introducing the ExecutionBackend trait, event-driven Scheduler policy engine, ExecutionKey deduplication model, and activation-guard-level concurrency budget enforcement.

## Technical Context

**Language/Version**: Rust 2024 edition (stable, >= 1.85)

**Primary Dependencies**: tokio (async runtime), tokio::sync::mpsc (mailbox), async-trait, serde/serde_json, thiserror, tracing

**Storage**: Event Store SPI (PostgreSQL via `ego-persistence`, InMemory via `ego-infrastructure`); Snapshot Store SPI

**Testing**: cargo test, mockall for mocks, deterministic integration tests with in-memory stores

**Target Platform**: Linux server (primary), WASM (optional via portable ExecutionBackend), macOS (development)

**Project Type**: Library crate embedded in application process (not standalone service)

**Performance Goals**: 10k+ commands/sec per entity under sequential access; 1k+ concurrent entities under budget

**Constraints**: <2ms p99 command processing latency (execution only, excluding I/O); <1GB memory for 10k active entities; deterministic replay == live execution

**Scale/Scope**: 10k entities, 100B event streams per entity, snapshot-based recovery

## Constitution Check

*GATE: Constitution file is a placeholder template. No specific constraints to enforce. All gates pass by default.*

## Project Structure

### Source Code

```text
crates/persistent-entity/
├── Cargo.toml
├── src/
│   ├── lib.rs                        # Public API, re-exports
│   ├── types.rs                      # EntityId, EntityTriple, ExecutionKey, CommandEnvelope (NEW: ExecutionKey)
│   ├── persistent_entity.rs          # PersistentEntity<C,E,S> trait
│   ├── entity_ref.rs                 # EntityRef<C,E,S> sender handle
│   ├── entity_actor.rs               # EntityActor::run() async loop (REFACTOR)
│   ├── lifecycle.rs                  # LifecycleStateMachine (REFINE)
│   ├── mailbox.rs                    # BoundedMailbox (mpsc) (KEEP)
│   ├── scheduler.rs                  # Scheduler policy engine (REWRITE as event-driven)
│   ├── scheduler_policy.rs           # NEW: FairnessWindow, CircuitBreaker, SchedulingPolicy trait
│   ├── scheduler_event.rs            # NEW: SchedulerEvent enum, Notify channel wiring
│   ├── execution_backend.rs          # NEW: ExecutionBackend trait (sync)
│   ├── execution_backend_tokio.rs    # NEW: TokioExecutionBackend (default)
│   ├── activation.rs                 # SharedActivation guard + budget enforcement (REFACTOR)
│   ├── registry.rs                   # EntityRegistry (REFINE)
│   ├── passivation.rs                # PassivationPolicy (KEEP)
│   ├── persistence.rs                # PersistenceFacade<E> (IMPLEMENT, not stub)
│   ├── publisher.rs                  # EventPublisher<E> (KEEP)
│   ├── snapshot.rs                   # SnapshotStrategy (KEEP)
│   ├── recovery.rs                   # StateRecovery (REFINE for sync)
│   ├── command_context.rs            # CommandContext (KEEP)
│   ├── command_envelope.rs           # CommandEnvelope (KEEP)
│   ├── supervisor.rs                 # Failure hooks (KEEP)
│   ├── runtime.rs                    # EntityRuntime top-level (REFACTOR)
│   ├── builder.rs                    # EntityRuntimeBuilder (UPDATE)
│   ├── error.rs                      # EntityError (UPDATE)
│   ├── test_entity.rs                # TestEntity (KEEP)
│   └── testing.rs                    # InMemoryEventStore, InMemorySnapshotStore (KEEP)
├── tests/
│   ├── common/mod.rs                 # Test helpers
│   ├── activation_ordering_tests.rs  # Existing tests (update)
│   ├── scheduler_tests.rs            # NEW: deterministic scheduling tests
│   ├── backend_tests.rs              # NEW: ExecutionBackend integration tests
│   ├── recovery_tests.rs             # NEW: replay equivalence tests
│   └── lifecycle_tests.rs            # NEW: lifecycle FSM tests
```

### Documentation (this feature)

```text
specs/006-persistent-entity-runtime/final-consistency-lock/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
└── tasks.md             # Phase 2 output (/speckit.tasks)
```

**Structure Decision**: All implementation lives in the existing `crates/persistent-entity` crate. No new crates are created. The ExecutionBackend trait and Scheduler policy engine are new modules within the same crate. The backend is a sync trait with a default Tokio wrapper implementation — no separate backend crate needed at this stage.

## Complexity Tracking

> No constitution violations to justify. All decisions follow the finalized architecture model.
