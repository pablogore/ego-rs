# Implementation Plan: CORE-007 Reactive Scheduler & Deterministic Projection Engine

**Branch**: `007-reactive-scheduler-projection` | **Date**: 2026-06-08 | **Spec**: specs/007-reactive-scheduler-projection/spec.md

**Input**: Feature specification from `specs/007-reactive-scheduler-projection/spec.md`

## Summary

Design and implement a reactive scheduling layer (CORE-007) that observes CORE-006 execution events via a bounded event bus, maintains a deterministic SchedulerState, and produces advisory activation suggestions via a pure SchedulingPolicy function. The Scheduler is reactive-only (no polling), non-authoritative (suggestions are advisory), and non-self-healing (recovery is external). Three hard architectural invariants govern observed-stream determinism, per-actor ordering, and strictly external recovery.

## Technical Context

**Language/Version**: Rust 2021 edition (stable)

**Primary Dependencies**: `ego-domain` (ActorId, EntityId, TenantId, DomainEvent), `tokio` (bounded sync channel for event bus), `tracing` (observability), `thiserror` (error types). **`EntityTriple` does not exist in `ego-domain` yet** — will be defined within the `ego-scheduler` crate (see research.md for rationale).

**Storage**: In-memory only — SchedulerState is ephemeral and reconstructable from event stream (per spec §9). No persistence.

**Testing**: `cargo test` (unit + integration), property-based testing for deterministic projection (two instances fed same events → identical state)

**Target Platform**: Same as ego-runtime — Linux/macOS server

**Project Type**: Library crate within Rust workspace (`crates/ego-scheduler`)

**Performance Goals**: Scheduler processing is sub-millisecond per event (pure state projection). Policy evaluation MUST complete within bounded time (per spec §4.3). Actor execution throughput MUST be identical with/without Scheduler load (per spec §11).

**Constraints**:
- MUST NOT modify CORE-006 execution path (P1 — Actor is execution authority)
- MUST NOT poll (P2 — reactive only)
- Determinism depends only on observed event stream (P3 — hard invariant)
- MUST NOT block Actor execution or enforce scheduling decisions (P4)
- Event bus MUST have bounded capacity (default 4096)
- No self-healing (hard invariant per §14.3)

**Scale/Scope**: Per-Actor partial ordering only. No global ordering (hard invariant per §14.2). Event bus bounded to 4096 events. Replay buffer bounded to 1024 events.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Gate | Status | Rationale |
|------|--------|-----------|
| TDD mandatory (Red-Green-Refactor) | PASS | Tests can be written before implementation; projection is pure function |
| Coverage >= 85% | PASS | Pure projection functions are exhaustively testable; policy evaluation is deterministic |
| Mock-based isolation | PASS | Event bus abstraction allows isolated SchedulerState tests |
| Deterministic tests | PASS | Core invariant: identical observed streams → identical state — directly testable |
| No circular dependency | PASS | CORE-007 depends on CORE-006 events; CORE-006 depends on nothing from CORE-007 |
| Immutability by default | PASS | SchedulerState is updated via pure projection (new state, no mutation) |
| Patch over rewrite | PASS | New crate does not modify existing CORE-006 code |
| No infrastructure in domain | PASS | CORE-007 is operational layer, not domain |

No violations. All gates pass without justification.

## Project Structure

### Documentation (this feature)

```text
specs/007-reactive-scheduler-projection/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
├── spec.md              # Feature specification
└── tasks.md             # Phase 2 output (/speckit.tasks)
```

### Source Code (repository root)

```text
crates/ego-scheduler/
├── Cargo.toml
└── src/
    ├── lib.rs                # Public API: Scheduler, SchedulerState, SchedulingPolicy
    ├── scheduler.rs          # Core Scheduler: event consumption, state projection
    ├── state.rs              # SchedulerState: deterministic projection state
    ├── policy.rs             # SchedulingPolicy trait + built-in RoundRobin
    ├── event_bus.rs          # Bounded event bus (tokio sync channel wrapper)
    ├── metric.rs             # Observability metrics (counters, gauges)
    ├── error.rs              # Scheduler error types
    └── gap.rs                # Gap detection logic
```

**Structure Decision**: New `crates/ego-scheduler` library crate. Follows existing workspace conventions (see `crates/runtime/` for reference). Layer assignment = `foundation` (same as `ego-runtime`). Must update `Cargo.toml` workspace members, `layers.toml`, and `scripts/verify-layers.sh`.

## Complexity Tracking

No constitution violations. Table omitted.
