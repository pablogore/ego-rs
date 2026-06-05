# Implementation Plan: Execution Envelope

**Branch**: `004-execution-envelope` | **Date**: 2026-06-04 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/004-execution-envelope/spec.md`

## Summary

Define a canonical `ExecutionEnvelope<P>` struct in `ego-domain` that carries a mandatory payload, identity (aggregate_id, entity_id, tenant_id), correlation (correlation_id, causation_id, request_id), and metadata into the runtime. Payload-less execution models use `ExecutionEnvelope<()>` where `()` is Rust's zero-sized type. `DomainExecutionContext` is constructed from `ExecutionEnvelope<P>` via the `From` trait. The existing `crates/runtime/src/context.rs` `RuntimeExecutionContext` is refactored with a `from_envelope()` constructor.

## Technical Context

**Language/Version**: Rust (latest stable, edition 2021)

**Primary Dependencies**: `ego-domain` — reuses identity/correlation types from 002 (AggregateId, EntityId, TenantId, CorrelationId, CausationId, RequestId, Metadata). New dependency: `serde` (derive macros `Serialize + Deserialize`) — format-agnostic serialization framework; the transport layer selects the specific wire format.

**Storage**: N/A — Envelope is a transient carrier, not persisted.

**Testing**: `cargo test` — unit tests for envelope construction, context construction from envelope, serialization round-trip.

**Target Platform**: Linux server, macOS (development)

**Project Type**: Library (multi-crate Rust workspace)

**Performance Goals**: Envelope construction is zero-cost — fields are stored directly, no allocation beyond the payload and metadata map.

**Constraints**: 
- ExecutionEnvelope MUST reuse 002 identity/correlation types — no new type definitions
- ExecutionEnvelope MUST NOT reference transport, actor, Tokio, or runtime types
- ExecutionContext construction from envelope MUST produce a read-only context
- Payload is mandatory (`payload: P`); payload-less execution models use `ExecutionEnvelope<()>`
- Identity and correlation fields are optional — `Option<...>` throughout
- ExecutionEnvelope derives serde `Serialize + Deserialize` — format-agnostic, transport owns format
- TDD per `.speckit/constitution.md`

**Scale/Scope**: Multi-crate workspace with domain, runtime, infrastructure crate layers

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### Architecture Check: `docs/architecture.md`

| Rule | Status | Detail |
|------|--------|--------|
| Domain owns contracts | ✅ PASS | ExecutionEnvelope lives in `ego-domain` alongside ExecutionContext |
| Dependency direction | ✅ PASS | `ego-domain` depends on nothing internal; `ego-runtime` → `ego-domain` |
| Runtime neutrality | ✅ PASS | Envelope carries no async, Tokio, or runtime types |
| Patch over rewrite | ✅ PASS | Extends existing context.rs; no new crates |
| Concrete first | ✅ PASS | Struct, not trait — data carrier has no behavioral abstraction |
| **No infrastructure in domain** (§C) | ⚠️ JUSTIFIED | `serde` derives added to `ego-domain`. Justification: serde traits (`Serialize`, `Deserialize`) are format-agnostic trait definitions — not a wire format like JSON or protobuf. The architecture prohibits serialization *frameworks* (format-specific infrastructure), which serde is not. The transport layer retains ownership of the wire format. This was explicitly resolved via the `/clarify` process (Option B). |

### Engineering Quality: `.speckit/constitution.md`

| Principle | Status | Detail |
|-----------|--------|--------|
| Test First Development | ✅ COMPLIANT | tasks.md requires TDD workflow (Red/Green/Refactor); tests written before implementation |
| Minimum Coverage (≥85%) | ✅ TARGET | Envelope is a data struct with accessors — 100% coverage achievable |
| No Real Infrastructure in Unit Tests | ✅ COMPLIANT | Envelope tests require no DB, network, or external services |
| Mock-Based Isolation | ✅ N/A | Envelope has no external dependencies to mock |
| Deterministic Test Execution | ✅ COMPLIANT | Envelope tests are pure data transformations — no timing, I/O, or randomness |
| Testability by Design | ✅ COMPLIANT | Constructor injection via `From<ExecutionEnvelope<P>>`; no hidden state |

**Result**: PASS — All gates pass. serde dependency explicitly justified.

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

`ego-domain` already has the 002 identity/correlation types. New dependency: `serde` with `derive` feature for `Serialize + Deserialize` derives on `ExecutionEnvelope`. `ego-runtime` already depends on `ego-domain` (from 002).

## Backward Compatibility

The existing `crates/runtime/src/context.rs` `RuntimeExecutionContext` must continue to function during the transition. The new `from_envelope()` constructor is additive. The old constructor may be deprecated after all runtime paths are updated.

## Task Phase Mapping

| Phase | Tasks | Outcome |
|-------|-------|---------|
| 1. Setup | T001–T004 | Envelope struct compiles with serde |
| 2. Foundational (TDD) | T005–T008 | Construction, DomainExecutionContext conversion, tests pass |
| 3. User Story 1 | T009–T012 | RuntimeExecutionContext from envelope (MVP) |
| 4. User Story 2 | T013–T015 | Multiple payload types verified |
| 5. User Story 3 | T016–T018 | Transport independence verified (incl. serde round-trip) |
| 6. Polish | T019–T021 | No-regression, clippy, quickstart validation |

## Complexity Tracking

No constitution violations detected. N/A.
