# Implementation Plan: Execution Context

**Branch**: `005-command-context` | **Date**: 2026-06-04 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/002-command-context/spec.md`

## Summary

Define a canonical `ExecutionContext` trait in `ego-domain` that represents pure execution context: identity (aggregate, entity, tenant), correlation, and metadata. No side effects, no persistence, no scheduling, no observability. Side-effect capabilities (persist, reply, schedule) are deferred to future specs (Effect API, Scheduling API). The existing runtime struct in `crates/runtime/src/context.rs` is reduced to a struct implementing the read-only domain trait.

## Technical Context

**Language/Version**: Rust (latest stable, edition 2021)

**Primary Dependencies**: `ego-domain` — no new external dependencies. Trait-only, pure contracts.

**Storage**: N/A — persistence is not a concern of this spec.

**Testing**: `cargo test` — trait contract tests in `ego-domain` (compile-time + unit tests), integration tests in `ego-runtime`.

**Target Platform**: Linux server, macOS (development)

**Project Type**: Library (multi-crate Rust workspace)

**Performance Goals**: Not specified at API level — context access is a zero-cost abstraction.

**Constraints**: 
- Domain contracts MUST be runtime-neutral (no `async`, no Tokio in trait signatures)
- ExecutionContext is read-only (`&self`) — no side effects
- Existing `crates/runtime/src/context.rs` must be refactored: `CorrelationId` moves to domain, existing struct implements domain trait
- ExecutionContext owns identity, correlation, metadata only — no persistence, replies, scheduling, observability, transport, or runtime execution

**Scale/Scope**: Multi-crate workspace with domain, runtime, infrastructure crate layers

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

**Result**: PASS — No violations.

## Project Structure

### Documentation (this feature)

```text
specs/002-command-context/
├── plan.md              # This file
├── spec.md              # Feature specification (revised)
├── research.md          # Design decisions (revised)
├── data-model.md        # Entity/field definitions (revised)
├── quickstart.md        # Validation guide (revised)
├── contracts/
│   └── execution_context.md  # Trait contract (revised)
├── checklists/
│   └── requirements.md
└── tasks.md             # Implementation tasks (revised)
```

### Source Code (repository root)

```text
crates/domain/src/
├── context.rs            # NEW — ExecutionContext trait, identity/correlation types
└── lib.rs                # MODIFY — add pub mod context, re-export

crates/runtime/src/
├── context.rs            # MODIFY — existing struct implements domain trait (read-only)
└── lib.rs                # MODIFY — update exports if needed
```

**Structure Decision**: Single trait in `ego-domain`, struct implementation in `ego-runtime`. No `async-trait`, no Tokio types in the domain trait.

### Crate Dependency Changes

```text
ego-runtime:
  before: depends on tokio, uuid
  after: adds dependency on ego-domain

ego-domain:
  before: no runtime dependencies
  after: still no runtime dependencies — trait-only, pure contracts
```

## Complexity Tracking

No constitution violations detected. N/A.
