---
id: lib-reexports
title: Public API re-exports
complexity: low
mode: autopilot
depends_on: [runtime-trait-contract]
---

## Tasks

### core-003-4-1-rewrite-lib-reexports

Rewrite `crates/runtime/src/lib.rs`: declare `pub mod runtime` and re-export `Runtime`, `ExecutionId`, `ExecutionState`, `SendError`, `SendErrorKind`, `SpawnError`, `SpawnErrorKind`, `RuntimeHandle`. Remove old exports (`Isolation`, `SchedulingPolicy`, `RuntimeError`).

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/spec.md` (lines 375-393)
- `openspec/changes/core-003-runtime-actor-execution/design.md` (lines 56-66)
- `crates/runtime/src/lib.rs` (old file to REWRITE)

## Files changed

- `crates/runtime/src/lib.rs` — REWRITE

## Completion

- `cargo check -p ego-runtime` passes
- Exports exactly the 8 specified public types
- Does NOT export `NullRuntime`, `Isolation`, `SchedulingPolicy`, `RuntimeError`

## Dependencies

Requires Runtime trait + all vocabulary types to exist.
