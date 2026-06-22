# Implementation Plan: Activation Ordering Model for Persistent Entity Runtime

**Branch**: `007-activation-ordering-model` | **Date**: 2026-06-07 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/007-activation-ordering-model/spec.md`

## Summary

Formalize and validate the activation ordering model for CORE-006 Persistent Entity Runtime: Mutex-based single-flight activation, mailbox-before-recovery semantics, recovery-first execution barrier, and registry visibility timing. The existing implementation in `crates/persistent-entity/` must be verified against the formal model defined in spec-007 and its design documents.

## Technical Context

**Language/Version**: Rust 1.75+

**Primary Dependencies**: tokio (sync, time), serde + serde_json, async-trait, thiserror, uuid, log, ego-domain

**Storage**: ego-domain `EventStore<E>` + `Snapshot` SPIs (in-memory for tests, production backends external)

**Testing**: `cargo test` (existing 12 unit tests), integration tests with tokio::test, concurrency stress tests

**Target Platform**: Linux/macOS servers (single-process Tokio runtime)

**Project Type**: Library (Rust crate: `ego-persistent-entity`)

**Performance Goals**: 10,000 concurrent entities per process, <100ms recovery for 10K events

**Constraints**: Single-process only; no external infra dependencies; deterministic recovery; no double spawn

**Scale/Scope**: Feature already implemented — this plan validates the formal model against the implementation

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

No constitution rules defined (template placeholders only). Gates pass trivially.

## Project Structure

### Documentation (this feature)

```text
specs/007-activation-ordering-model/
├── plan.md                        # This file
├── spec.md                        # Feature specification
├── research.md                    # Phase 0 — model verification findings
├── implementation-skeleton.md     # Rust runtime skeleton (v2)
├── runtime-consistency-clarification.md  # Mutex/mailbox/recovery/registry timing
├── registry-visibility-semantics.md     # Visibility vs readiness formal model
├── data-model.md                  # Phase 1 — entity/state model
├── quickstart.md                  # Phase 1 — validation scenarios
├── contracts/                     # Phase 1 — trait contracts
└── tasks.md                       # Phase 2 — task breakdown
```

### Source Code (repository root)

```text
crates/persistent-entity/
├── Cargo.toml
└── src/
    ├── lib.rs                     # Re-exports
    ├── runtime.rs                 # EntityRuntime<E>
    ├── builder.rs                 # EntityRuntimeBuilder<E>
    ├── entity_ref.rs              # EntityRef<C,E,S> — activation trigger
    ├── actor.rs                   # EntityActor<C,E,S> — recovery + command loop
    ├── registry.rs                # EntityRegistry — active/passivated/pending_activations
    ├── activation.rs              # SharedActivation — Mutex guard + watch channel
    ├── mailbox.rs                 # Mailbox<C>, CommandEnvelope
    ├── persistent_entity.rs       # PersistentEntity trait
    ├── publisher.rs               # EventPublisher<E>
    ├── persistence.rs             # PersistenceFacade<E>
    ├── recovery.rs                # StateRecovery trait
    ├── lifecycle.rs               # LifecycleStateMachine
    ├── snapshot.rs                # SnapshotStrategy
    ├── command_context.rs         # CommandContext
    ├── error.rs                   # EntityError
    ├── scheduler.rs               # Scheduler (semaphore)
    ├── supervisor.rs              # Supervisor (failure hooks)
    └── testing.rs                 # InMemoryEventStore + InMemorySnapshotStore + NoopPublisher
```

**Structure Decision**: Single Rust library crate following the existing layout in `crates/persistent-entity/`. No structural changes needed.

## Complexity Tracking

No constitution violations. Existing structure matches the crate's responsibility boundaries.
