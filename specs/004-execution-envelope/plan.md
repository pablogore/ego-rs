# Implementation Plan: Execution Envelope

**Branch**: `007-execution-envelope` | **Date**: 2026-06-04 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/004-execution-envelope/spec.md`

## Summary

Define a canonical `ExecutionEnvelope<P>` struct in `ego-domain` that carries payload, identity (aggregate_id, entity_id, tenant_id), correlation (correlation_id, causation_id, request_id), and metadata into the runtime. ExecutionContext is constructed from ExecutionEnvelope. The existing `crates/runtime/src/context.rs` struct is refactored to accept ExecutionEnvelope and implement the domain ExecutionContext trait.

## Technical Context

**Language/Version**: Rust (latest stable, edition 2021)

**Primary Dependencies**: `ego-domain` — reuses identity/correlation types from 002 (AggregateId, EntityId, TenantId, CorrelationId, CausationId, RequestId, Metadata). No new external dependencies.

**Storage**: N/A — Envelope is a transient carrier, not persisted.

**Testing**: `cargo test` — unit tests for envelope construction, context construction from envelope, serialization round-trip.

**Target Platform**: Linux server, macOS (development)

**Project Type**: Library (multi-crate Rust workspace)

**Performance Goals**: Envelope construction is zero-cost — fields are stored directly, no allocation beyond the payload and metadata map.

**Constraints**: 
- ExecutionEnvelope MUST reuse 002 identity/correlation types — no new type definitions
- ExecutionEnvelope MUST NOT reference transport, actor, Tokio, or runtime types
- ExecutionContext construction from envelope MUST produce a read-only context
- All fields except payload are optional — `Option<...>` throughout
- TDD per `.speckit/constitution.md`

**Scale/Scope**: Multi-crate workspace with domain, runtime, infrastructure crate layers

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

**Result**: PASS — No violations.

## Project Structure

### Documentation (this feature)

```text
specs/004-execution-envelope/
├── plan.md              # This file
├── spec.md              # Feature specification (draft)
├── research.md          # Design decisions
├── data-model.md        # Entity/field definitions
├── quickstart.md        # Validation guide
├── contracts/
│   └── envelope.md      # Envelope type contract
├── checklists/
│   └── requirements.md
└── tasks.md             # Implementation tasks
```

### Source Code (repository root)

```text
crates/domain/src/
├── envelope.rs           # NEW — ExecutionEnvelope<P> struct
├── context.rs            # MODIFY — add ExecutionContext::from(envelope) if not already present
└── lib.rs                # MODIFY — add pub mod envelope, re-export

crates/runtime/src/
├── context.rs            # MODIFY — existing struct refactored to accept ExecutionEnvelope
└── lib.rs                # MODIFY — update exports if needed
```

**Structure Decision**: ExecutionEnvelope in `ego-domain` (reuses 002 types), refactored runtime struct in `ego-runtime`.

### Crate Dependency Changes

`ego-domain` already has the 002 identity/correlation types. No new dependencies needed. `ego-runtime` already depends on `ego-domain` (from 002).

## Backward Compatibility

The existing `crates/runtime/src/context.rs` CommandContext struct must continue to function during the transition. A new constructor accepting ExecutionEnvelope is additive. The old constructor may be deprecated after all runtime paths are updated.

## Task Phase Mapping

| Phase | Tasks | Outcome |
|-------|-------|---------|
| 1. Setup | T001 | Envelope struct compiles |
| 2. Foundational | T002–T004 | Construction, context conversion, tests |
| 3. Runtime Integration | T005–T007 | Runtime struct refactored, tests |
| 4. Polish | T008–T009 | No-regression, validation |

## Complexity Tracking

No constitution violations detected. N/A.
