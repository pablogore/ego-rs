# Implementation Plan: CORE-007 Reactive Scheduler & Deterministic Projection Engine

**Branch**: `007-reactive-scheduler-projection` | **Date**: 2026-06-08 | **Spec**: specs/007-reactive-scheduler-projection/spec.md

**Input**: Feature specification from `specs/007-reactive-scheduler-projection/spec.md`

## Summary

Implement a reactive scheduling layer (CORE-007) that observes CORE-006 execution events via a bounded event bus, maintains a deterministic SchedulerState, and produces advisory activation suggestions via a pure SchedulingPolicy function. The Scheduler is reactive-only (no polling), non-authoritative (advisory output), and non-self-healing (recovery is external).

**Core Invariants** (7 total — see spec §9):
1. **I1 Determinism**: SchedulerState = f(observed_stream) only, where observed_stream ≡ events surviving DropPolicy
2. **I2 Per-entity ordering**: sequence_id never compared across entities; single-stream model — SchedulerState tracks one entity at a time
3. **I3 No execution authority**: output is advisory; `suggest_activation` is NOT a command; CORE-006 never depends on it
4. **I4 ReplayBuffer non-semantic**: diagnostic-only; never reconstruction or recovery; buffer differences do not affect state equivalence
5. **I5 Deterministic DropPolicy**: drop decisions depend only on arrival order + capacity + policy
6. **I6 Single-consumer bus**: Scheduler owns receiver exclusively (no double consumption); sender is Clone (multi-producer); dropping Scheduler closes channel
7. **I7 Policy field access**: `suggest_activation` may only read `total_events_consumed` and `last_suggestion` from state; must not read diagnostic or per-actor-scoped fields

## Technical Context

**Language/Version**: Rust 2021 edition (stable)

**Primary Dependencies**: `ego-domain` (ActorId, EntityId, TenantId, DomainEvent), `tokio` (bounded sync channel, `Notify`), `tracing` (observability), `thiserror` (error types). `EntityTriple` defined within `ego-scheduler` crate.

**Storage**: In-memory only — SchedulerState is ephemeral and reconstructable from event stream. ReplayBuffer is non-semantic diagnostic only.

**Testing**: `cargo test`, property-based determinism testing (identical streams → identical state), concurrent-vs-sequential equivalence, DropPolicy determinism under varying load.

**Target Platform**: Linux/macOS server (same as ego-runtime)

**Project Type**: Library crate (`crates/ego-scheduler`)

**Performance Goals**: Sub-millisecond per event (pure state projection). Policy evaluation O(pending). Actor throughput unaffected by Scheduler.

**Concurrency Model**: Concurrency is an implementation detail only — correctness is defined over sequential application of the observed event stream. Any concurrent processing MUST be equivalent to a single-threaded deterministic execution. No dependency on async runtime ordering semantics (Tokio or equivalent). Tokio may be used as the runtime but correctness MUST NOT depend on its scheduling behavior.

**Scale/Scope**: Per-actor ordering only. Event bus 4096 events. Replay buffer 1024 events.

**Bus Semantics**: `try_send()` is fire-and-forget — SendError is final, no retry orchestration. Each send is atomic per-event (no batch). Ordering guarantees apply only to successfully enqueued events. DropPolicy evaluated strictly at enqueue time. Scheduler never orchestrates retries — it only drains.

**SchedulerState Role**: Pure reducer output — a data structure, not a runtime engine. `apply()` is pure: `(Event, S) → S`. Entity switch detection (`current_entity != event.source_actor`) and per-entity field resets performed by Scheduler BEFORE calling `apply()`. Orchestration (bus drain, event loop, policy evaluation, per-entity reset) lives in `Scheduler`, not `SchedulerState`.

**Scheduler Architecture**: Decomposed into a deterministic pipeline of 6 pure components. Scheduler (`scheduler.rs`) is a thin orchestrator — composition only, no business logic. Pipeline order: EventIngestor → EntityRouter → StateReducer → GapDetector → PolicyEvaluator → SuggestionEmitter. Each stage is independently testable.

**Policy Collection**: RoundRobin uses `BTreeSet<EntityTriple>` for deterministic iteration order. `HashSet` iteration is non-deterministic in Rust and forbidden for scheduling decisions. `pending` collection type is `BTreeSet` at the trait level.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Gate | Status | Rationale |
|------|--------|-----------|
| TDD mandatory | PASS | Pure projection functions are exhaustively testable |
| Coverage >= 85% | PASS | Deterministic functions, property-testable |
| Mock-based isolation | PASS | Event bus abstraction enables isolated tests |
| Deterministic tests | PASS | I1: identical streams → identical state |
| No circular dependency | PASS | CORE-007 depends on CORE-006; never reverse |
| Immutability by default | PASS | Pure projection — new state, no mutation |
| Patch over rewrite | PASS | New crate; no existing code modified |
| No infrastructure in domain | PASS | Foundation layer, not domain |
| Per-entity scoping | PASS | I2: single-stream model; no cross-entity comparisons |
| Advisory-only output | PASS | I3: no execution authority; suggest_activation is NOT a command |
| No concurrency non-determinism | PASS | I1: sequential equivalence required; concurrency is implementation detail only |
| Single-consumer bus | PASS | I6: Scheduler owns receiver exclusively; no double consumption |
| Policy field isolation | PASS | I7: policy reads only allowed fields; no diagnostic or per-actor-scoped field access |
| Pipeline drift guard | PASS | Scheduler is fixed orchestration shell — no domain logic, no branching, composition only. Each responsibility in exactly one module. Drift detection: if logic appears in Scheduler beyond function calls → STOP and refactor |

No violations.

## Project Structure

### Documentation

```text
specs/007-reactive-scheduler-projection/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
├── spec.md              # Feature specification
└── tasks.md             # Phase 2 output
```

### Source Code

```text
crates/ego-scheduler/
├── Cargo.toml
└── src/
    ├── lib.rs                # Public API
    ├── scheduler.rs          # Scheduler: thin pipeline orchestrator (composition only, no business logic)
    │                         #   Pipeline stages: ingest → route → reduce → detect → evaluate → emit
    ├── scheduler/
    │   ├── ingest.rs         # EventIngestor: drains event bus only, returns Vec<BusItem>
    │   ├── route.rs          # EntityRouter: detects entity switch (current != event.source_actor), resets per-entity fields
    │   ├── reduce.rs         # StateReducer: wraps SchedulerState::apply() — pure function, no branching
    │   ├── detect.rs         # GapDetector: structural only — sequence_id != last + 1 → increment counter
    │   ├── evaluate.rs       # PolicyEvaluator: calls SchedulingPolicy::suggest_activation — no side effects
    │   └── emit.rs           # SuggestionEmitter: writes last_suggestion only, no logic
    ├── state.rs              # SchedulerState: deterministic projection
    ├── policy.rs             # SchedulingPolicy trait + RoundRobin
    ├── event_bus.rs          # Bounded bus with deterministic DropPolicy
    ├── metric.rs             # Observability (counters, gauges)
    ├── error.rs              # SchedulerError types
    └── gap.rs                # GapInfo type (used by detect.rs)
```

**Layer**: `foundation` (same as `ego-runtime`). Register in workspace `Cargo.toml`, `layers.toml`, `scripts/verify-layers.sh`.

## Complexity Tracking

No constitution violations.
