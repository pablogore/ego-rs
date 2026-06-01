---
id: execution-id
title: ExecutionId type
complexity: low
mode: autopilot
depends_on: [workspace-setup]
---

## Tasks

### core-003-2-1-rewrite-execution-id

Create `crates/runtime/src/runtime/execution.rs` with `ExecutionId` newtype wrapping `Uuid`. Constructor `new()` generates random v4 uuid. Implements `Clone + Copy + Debug + Eq + Hash + Send + Sync`. Delete old flat `src/execution.rs`.

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/spec.md` (lines 6-37)
- `crates/runtime/src/execution.rs` (old file to DELETE)

## Files changed

- `crates/runtime/src/runtime/execution.rs` — CREATE
- `crates/runtime/src/execution.rs` — DELETE

## Completion

- `cargo check -p ego-runtime` passes
- `ExecutionId::new()` returns distinct ids
- No actor types, no `ego-domain` imports
- Old flat file removed

## Dependencies

Requires workspace-setup (for Cargo.toml with uuid dep + mod.rs scaffold).
