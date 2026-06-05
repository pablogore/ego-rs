# Implementation Plan: Read Side Projections

**Branch**: `005-read-side-projections` | **Date**: 2026-06-04 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/005-read-side-projections/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/plan-template.md` for the execution workflow.

## Summary

Implement a Lagom-inspired read-side projection engine in ego-rs. The engine consumes `EventStreamElement<E>` exclusively via `ReadSideStore` (pull-based, tag+offset queries, separate from `EventStore`), groups events by tag into batches, executes handlers via `ReadSideSession` with metadata atomic commit (offset + dedup — handler side effects excluded), and supports replay/rebuild. The runtime exposes a formal state machine (RUNNING, REPLAYING, REBUILDING, PAUSED, FAILED) and reports progress/errors via a `ProgressReporter` trait. Events originate from protobuf contracts (buf) wrapped in CloudEvents and flow through `ego-event-adapter` on the write path. The read-side core is fully agnostic to protobuf, CloudEvents, and EventStore — it only depends on `EventStreamElement`.

## Technical Context

**Language/Version**: Rust, edition 2021 (MSRV matches workspace)

**Primary Dependencies**: serde + serde_json (serialization), chrono (timestamps), thiserror (error types) — all already in `ego-domain`. mockall for test mocking. `ego-event-adapter` additionally depends on: protobuf (via buf-generated code), cloudevents SDK (Rust).

**Storage**: Four separate storage SPIs in domain, each with in-memory + Postgres backends:
- `ReadSideStore` — event fetch by tag + offset (separate from `EventStore`, FR-025)
- `OffsetStore` — per (projection_id, tag, tenant) offset persistence (3-tuple storage key)
- `DedupStore` — per (projection_id, tag, event_id) dedup tracking
- `ProgressReporter` — trait-based callback for progress/error/state-transition observability (FR-028, host injects implementation at runner construction)

In-memory backends in `ego-infrastructure/persistence/in_memory/`; Postgres in `ego-infrastructure/persistence/postgres/`.

**Testing**: `cargo test` with mockall for trait mocking. All unit tests MUST use in-memory backends — no real databases. Tests MUST be deterministic and offline.

**Target Platform**: Linux / macOS server (same as existing workspace).

**Project Type**: Library (domain SPI traits in `ego-domain`, backend implementations in `ego-infrastructure`, runtime orchestration in `ego-runtime`).

**Performance Goals**: Process 10k events per tag with zero duplicates reaching handler (SC-005). Batch throughput optimized — handlers receive `Vec<EventStreamElement>`, not single events.

**Constraints**:
- Domain crate MUST remain runtime-neutral (no async, no Tokio) per `docs/architecture.md` §D
- Read-side MUST NOT access EventStore directly (FR-020) — uses `ReadSideStore` instead
- Domain MUST NOT depend on protobuf, CloudEvents, or gRPC types (FR-026)
- CORE-005 engine consumes `EventStreamElement<E>` exclusively — never raw EventStore, protobuf, or CloudEvents
- All public APIs MUST have rustdoc per `.speckit/constitution.md`
- Constructor injection, trait-based design per constitution's Testability by Design principle
- No global mutable state; pure functions preferred per Functional Programming principle

**Scale/Scope**: Multi-tenant event processing with per-tag ordering guarantees. Each projection maintains independent offset + dedup state per tag. No cross-tag coordination.

**Runtime State Machine**: Each projection transitions through RUNNING → REPLAYING/REBUILDING/PAUSED/FAILED states. Automatic transitions: RUNNING→REPLAYING (replay call), RUNNING→REBUILDING (rebuild call), RUNNING→FAILED (fatal error), REPLAYING/REBUILDING→RUNNING (completes). Manual: RUNNING↔PAUSED (pause/resume API). New events during REPLAYING/REBUILDING are queued and processed after completion.

**Observability**: `ProgressReporter` trait in domain crate with `on_batch_completed`, `on_error`, `on_state_transition` methods. Host injects implementation at runner construction — follows existing SPI pattern.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

**Pre-Phase 0**: ✅ All principles pass. Design aligns with constitutional requirements.

**Post-Phase 1**: ✅ Re-check passed. Design artifacts (research.md, data-model.md, contracts/) introduce no new violations. All traits use constructor injection, no global mutable state, no async in domain crate.

| Principle | Status | Justification |
|-----------|--------|---------------|
| Test First Development | ✅ PASS | All domain traits designed for mockability; in-memory backend enables deterministic TDD |
| Minimum Coverage >= 85% | ✅ PASS | Achievable via unit tests on domain contracts + in-memory backend integration tests |
| No Real Infrastructure in Unit Tests | ✅ PASS | Backend trait allows InMemoryReadSideStore + InMemoryOffsetStore + InMemoryDedupStore; no Postgres in unit tests |
| Mock-Based Isolation | ✅ PASS | OffsetStore, DedupStore, EventTagger are all traits — mockable with mockall |
| Deterministic Test Execution | ✅ PASS | No time-based flakiness; all deps injectable; in-memory backend is deterministic |
| Testability by Design | ✅ PASS | Constructor injection on ReadSideSession; traits for all SPI boundaries |
| Functional Programming | ✅ PASS | EventStreamElement is immutable; handlers receive immutable batch references |
| Deterministic Business Logic | ✅ PASS | No hidden state; time is injected via EventStreamElement.occurred_at |
| Rustdoc Documentation | ✅ PASS | All public traits, structs, enums, and functions require rustdoc |
| Dependency Direction | ✅ PASS | Domain traits in ego-domain, backends in ego-infrastructure, runtime in ego-runtime |

No violations. The design aligns with all constitutional principles.

## Project Structure

### Documentation (this feature)

```text
specs/005-read-side-projections/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)

```text
# DOMAIN — contracts / traits / types
crates/domain/src/
├── lib.rs                                         # Add `pub mod read_side;`
└── read_side/                                     # NEW module
    ├── mod.rs                                     # Re-exports
    ├── event_stream.rs                            # EventStreamElement<E>
    ├── event_tag.rs                               # EventTag
    ├── tagger.rs                                  # EventTagger trait
    ├── session.rs                                 # ReadSideSession<'a>, ReadSideConfig
    ├── dedup.rs                                   # DedupStore trait
    ├── offset.rs                                  # Offset enum + OffsetStore trait
    ├── store.rs                                   # ReadSideStore trait (tag+offset fetch)
    ├── handler.rs                                 # Handler trait (Vec<EventStreamElement>)
    ├── processor.rs                               # ReadSideProcessor trait
    ├── error.rs                                   # ProjectionError enum
    ├── progress.rs                                # ProgressReporter trait (on_batch_completed, on_error, on_state_transition)
    └── runner.rs                                  # ReadSideRunner trait

# ADAPTER — protobuf → CloudEvent → EventStore
crates/event-adapter/                              # NEW crate
├── Cargo.toml                                     # Deps: protobuf, cloudevents-sdk, ego-domain
└── src/
    ├── lib.rs
    ├── protobuf_to_ce.rs                          # buf types → CloudEvent
    ├── ce_to_eventstore.rs                        # CloudEvent → EventStore record
    ├── eventstore_to_ese.rs                       # EventStore → EventStreamElement
    └── tagger_exec.rs                             # EventTagger application

# INFRASTRUCTURE — concrete backends
crates/infrastructure/src/persistence/
├── mod.rs                                         # Unchanged
├── in_memory/
│   ├── mod.rs                                     # Existing
│   ├── read_side_store.rs                         # InMemoryReadSideStore
│   ├── offset_store.rs                            # InMemoryOffsetStore
│   └── dedup_store.rs                             # InMemoryDedupStore
└── postgres/
    ├── mod.rs                                     # Existing
    ├── read_side_store.rs                         # PostgresReadSideStore
    ├── offset_store.rs                            # PostgresOffsetStore
    └── dedup_store.rs                             # PostgresDedupStore

# RUNTIME — async polling + batch execution
crates/runtime/src/
└── read_side/                                     # NEW module
    ├── mod.rs                                     # ReadSideRunner impl
    ├── scheduler.rs                               # TagScheduler, polling loop
    ├── batch_executor.rs                          # Batch execution + session lifecycle
    └── backpressure.rs                            # Concurrency control (max_in_flight, concurrency_per_tag)
```

**Structure Decision**: Follow existing crate boundaries per `docs/architecture.md` §B:
- Domain contracts (traits + types) → `crates/domain/src/read_side/`
- Event adapter (protobuf/CE conversion) → new `crates/event-adapter/`
- Backend implementations → `crates/infrastructure/src/persistence/`
- Async runtime orchestration → `crates/runtime/src/read_side/`

## Clarifications Incorporated

- **Commit atomicity**: Session commit guarantees atomicity for metadata only (offset + dedup). Handler side effects are excluded from the transaction boundary — they are best-effort from the runtime's perspective.
- **Failure semantics**: Failure before commit → full retry allowed. Failure after handler success but before commit → dedup prevents duplicates on next fetch.
- **Runtime state machine**: 5 explicit states (RUNNING, REPLAYING, REBUILDING, PAUSED, FAILED) with defined transitions. New events during REPLAYING/REBUILDING are queued.
- **ProgressReporter trait**: Domain crate owns the trait. Host injects implementation. Methods: `on_batch_completed`, `on_error`, `on_state_transition`.

## Complexity Tracking

No constitution violations to justify. All complexity is warranted by spec requirements.
