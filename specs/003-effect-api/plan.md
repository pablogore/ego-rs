# Implementation Plan: Effect API

**Branch**: `006-effect-api` | **Date**: 2026-06-04 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/003-effect-api/spec.md`

## Summary

Define a canonical `Effect` enum hierarchy in `ego-domain` that represents execution outcomes: NoEffect, StateMutation, EventEmission, Reply, and Composed. Effects are value types — they describe what the handler wants to happen without performing any side effects. Runtime crates interpret Effects and execute the described outcomes.

**Handler contract**: Execution handlers return `Effect<E, R, S>` synchronously. The Effect describes what should happen; the runtime decides how to execute it.

**Execution model support**: The generic type parameters (E = event, R = reply, S = state) make Effect usable by event-sourced entities, stateful entities, CRUD entities, workflows, sagas, and projections without modification. No DomainEvent bound required.

**Composition semantics**: Effects compose via the `Composed` variant, which holds a `Vec<Effect<E, R, S>>`. Composition is recursive — any Effect variant may appear as a child. The runtime SHALL recursively flatten nested `Composed` structures before interpretation (canonical recursive flattening). Flattening preserves execution order — depth-first traversal of the unflattened tree produces identical leaf order to linear iteration of the flattened list. The `and_then` combinator already flattens during construction; direct `compose()` preserves caller-provided structure. Combinators such as `and_then` or `combine` provide ergonomic construction.

## Technical Context

**Language/Version**: Rust (latest stable, edition 2021)

**Primary Dependencies**: `ego-domain` — no new external dependencies. Effect types are pure value types (enums, generics, standard library only).

**Storage**: N/A — Effects are not persisted; they describe outcomes.

**Testing**: `cargo test` — unit tests for Effect construction, composition, and assertion. No runtime needed.

**Target Platform**: Linux server, macOS (development)

**Project Type**: Library (multi-crate Rust workspace)

**Performance Goals**: Effect construction is a zero-cost abstraction (enums, no allocation required for simple cases).

**Constraints**: 
- Effects MUST NOT expose Tokio, actors, channels, mailboxes, network primitives, or database APIs
- Effects are value types — must implement Debug, Clone, PartialEq; SHOULD implement Eq, Hash
- Effects MUST NOT assume DomainEvent exists — event types are generic (any E type)
- Effect API MUST NOT depend on ExecutionContext (defined in 002)
- All implementation MUST follow TDD (Red/Green/Refactor) per `.speckit/constitution.md`

**Scale/Scope**: Multi-crate workspace with domain, runtime, infrastructure crate layers

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

**Result**: PASS — No violations.

## Project Structure

### Documentation (this feature)

```text
specs/003-effect-api/
├── plan.md              # This file
├── spec.md              # Feature specification (draft)
├── research.md          # Design decisions
├── data-model.md        # Entity/field definitions
├── quickstart.md        # Validation guide
├── contracts/
│   └── effect.md        # Effect type contract
├── checklists/
│   └── requirements.md
└── tasks.md             # Implementation tasks
```

### Source Code (repository root)

```text
crates/domain/src/
├── effect.rs             # NEW — Effect enum, variants, composition logic
└── lib.rs                # MODIFY — add pub mod effect, re-export

crates/runtime/src/
├── interpreter.rs        # NEW — runtime interprets Effects (future)
└── lib.rs                # MODIFY — add runtime interpreter (future)
```

**Structure Decision**: Effect types in `ego-domain` (value types only), Effect interpretation in runtime crates.

### Crate Dependency Changes

All Effect types are standard-library-only enums. No crate dependency changes needed.

## Backward Compatibility

The Effect API is additive — existing runtime code does not need to adopt Effects to function. Handlers that return Effects are a new pattern; existing handlers without Effects continue to work. The Effect API lives in `ego-domain` alongside ExecutionContext, so no crate dependency shuffling is required beyond what 002 established.

## Task Phase Mapping

| Phase | Tasks | Outcome |
|-------|-------|---------|
| 1. Setup | T001 | Effect enum compiles |
| 1b. Handler Return Type | T002b–T002c | Handler return type contract + module |
| 2. Foundational | T003–T005 | Construction + composition + tests |
| 3. Reply (US1) | T006–T007 | Reply effect, handler test |
| 4. Emit (US2) | T008–T009 | Event emission effect |
| 5. Compose (US3) | T010–T011 | Composed effects |
| 6. Polish | T012–T014 | No-regression, validation |

## Complexity Tracking

No constitution violations detected. N/A.
