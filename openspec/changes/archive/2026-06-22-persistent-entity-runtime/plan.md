# Implementation Plan: Persistent Entity Runtime and SDK

**Branch**: `006-persistent-entity-runtime` | **Date**: 2026-06-07 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/006-persistent-entity-runtime/spec.md`

## Summary

Define and implement an event-sourced persistent entity abstraction for EGO-RS, inspired by Lagom's PersistentEntity pattern. The runtime provides actor-per-entity execution with bounded FIFO mailbox, single-writer guarantees, snapshot support, optimistic concurrency, event publication, and Postgres-backed persistence. Built as a new `ego-persistent-entity` crate consuming `ego-domain` persistences SPIs.

## Technical Context

**Language/Version**: Rust 1.75+ (stable, edition 2021)

**Primary Dependencies**:
- `ego-domain` (core traits: EventStore, Snapshot, Repository, Effect, ExecutionContext, Actor)
- `ego-runtime` / `ego-runtime-tokio` (existing Runtime trait, TokioRuntime concrete impl)
- `ego-persistence` / `ego-infrastructure` (Postgres and in-memory persistence backends)
- `tokio` (async runtime, mpsc channels for mailboxes)
- `async-trait` (trait async method support)

**Storage**: Existing `ego-persistence::PostgreSQLEventStore` and `PostgreSQLSnapshotStore`. In-memory test backends from `ego-infrastructure`.

**Testing**: `cargo test` with in-memory persistence backends. Deterministic unit tests per constitution §8.

**Target Platform**: Linux server (macOS for development)

**Project Type**: Library (`crates/persistent-entity/` — new workspace member crate)

**Performance Goals**: Inline entity commands in <1ms p99 on cached (ACTIVE) entities with in-memory state. Recovery latency dominated by snapshot load + event replay. Snapshot-at-N strategy keeps recovery bounded.

**Constraints**:
- MUST follow EGO-RS layering: `ego-domain` → `ego-persistent-entity` → persistence/runtime adapters
- MUST NOT leak implementation types (Tokio, Postgres) into public API — per constitution §7
- CAS loops are FORBIDDEN per constitution §5. Reactivation guard uses per-entity Mutex or single-flight, never CAS
- TDD required per constitution §8 — line and branch coverage >= 85%
- No external infrastructure dependencies for tests
- Must be testable with in-memory backends exclusively

**Scale/Scope**: Single-node entity runtime. Entity count determined by available memory (each ACTIVE entity holds state in-memory). Passivation policy governed by configurable inactivity/memory thresholds.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Gate 1: Layer Compliance
The entity runtime crate (`ego-persistent-entity`) lives above `ego-domain` and uses its persistence SPIs (`EventStore`, `Snapshot`, `Repository`) and core types (`Effect`, `Actor`, `ExecutionContext`). This is consistent with the existing crate dependency hierarchy: `ego-runtime` and `ego-application` sit at the same layer.

**Verdict**: PASS — no layer violation. New crate consumes domain SPIs, does not bypass the pipeline.

### Gate 2: External Effect Isolation
Entity command handlers produce events (internal effects) and optionally describe external effects. Event persistence and effect dispatch follow the same Decision→Execution→Commit pattern. The entity runtime commits events first, then dispatches publications. This matches the constitution's EE-R1–EE-R6.

**Verdict**: PASS — external effects are described as intents, persisted atomically with events, dispatched post-commit.

### Gate 3: CAS Prohibition
The specification allows CAS-based reactivation guards in the risk document. However, constitution §5 explicitly forbids "CAS loops (AtomicUsize, compare_exchange) anywhere in the system." The implementation MUST use per-entity Mutex, single-flight pattern, or channel-based ownership — NOT CAS.

**Verdict**: CONDITIONAL PASS — implementation MUST avoid CAS. Reactivation guard uses Mutex or single-flight.

### Gate 4: TDD and Coverage
Constitution §8 mandates TDD and >= 85% line/branch coverage. The entity runtime involves async concurrency (mailbox, recovery, passivation) which is testable via in-memory backends.

**Verdict**: PASS — testability is designed-in (in-memory backends, deterministic replay, mockable SPIs).

### Gate 5: Immutability
Domain data structures (events, state, commands) are immutable. The entity runtime applies events to produce new state instances, never mutating in-place.

**Verdict**: PASS — event-sourced model natively enforces immutability.

### Gate 6: No Implementation Type Leakage
Public API (EntityRef, PersistentEntity trait, CommandContext, etc.) MUST NOT expose Tokio types, Postgres types, or any implementation-specific types.

**Verdict**: PASS — public types are domain-owned. Tokio channels, PgPool, etc. are internal.

**New violations introduced?**: None identified.

**Unjustified complexity?**: The actor-per-entity model with dedicated Tokio tasks is justified by the single-writer guarantee and deterministic replay requirements, per the spec clarification (Dedicated Actor Task model chosen over pooled workers).

## Project Structure

### Documentation (this feature)

```text
specs/006-persistent-entity-runtime/
├── plan.md              # This file
├── spec.md              # Feature specification
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
├── checklists/
│   ├── requirements.md
│   └── pre-plan.md
└── risks/
    └── passivation-reactivation.md
```

### Source Code (repository root)

```text
crates/persistent-entity/          # New crate: ego-persistent-entity
├── Cargo.toml
├── src/
│   ├── lib.rs                     # Crate root, public re-exports
│   ├── entity_ref.rs              # EntityRef API (command sender)
│   ├── persistent_entity.rs       # PersistentEntity trait (user-facing)
│   ├── command_context.rs         # CommandContext value type
│   ├── runtime.rs                 # EntityRuntime: lifecycle manager
│   ├── actor.rs                   # EntityActor: dedicated task + mailbox loop
│   ├── lifecycle.rs               # LifecycleStateMachine (5 states)
│   ├── mailbox.rs                 # Bounded FIFO mailbox (Tokio mpsc wrapper)
│   ├── recovery.rs                # State recovery: snapshot load + event replay
│   ├── passivation.rs             # Passivation policy + registry
│   ├── snapshot.rs                # SnapshotStrategy trait + built-in strategies
│   ├── error.rs                   # Error types (EntityNotFound, VersionConflict, etc.)
│   └── testing.rs                 # Test helpers (in-memory backend wiring)
└── tests/
    ├── entity_lifecycle.rs        # Smoke tests: create, mutate, recover, passivate
    ├── concurrency.rs             # Mailbox ordering, concurrent send tests
    ├── recovery.rs                # Snapshot + replay correctness tests
    ├── passivation.rs             # PASSIVATING rejection, auto-reactivation tests
    └── version_conflict.rs        # Optimistic concurrency conflict tests
```

**Structure Decision**: New `crates/persistent-entity/` crate added to workspace. Follows existing pattern (`crates/runtime`, `crates/persistence`, etc.). Internal module structure mirrors the spec's functional decomposition (actor, mailbox, lifecycle, recovery, passivation, snapshot).

## Complexity Tracking

> No constitution violations to justify. All gates pass conditionally — only the CAS prohibition in Gate 3 is a constraint that must be enforced in implementation.

## Research (Phase 0)

See [research.md](research.md) for consolidated findings on all technical unknowns.

## Data Model (Phase 1)

See [data-model.md](data-model.md) for entity definitions, field types, and relationships.

## Contracts (Phase 1)

See [contracts/](contracts/) for SPI trait definitions and public API contracts.

## Quickstart (Phase 1)

See [quickstart.md](quickstart.md) for validation scenarios and run guide.
