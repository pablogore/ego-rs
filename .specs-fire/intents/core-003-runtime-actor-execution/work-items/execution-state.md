---
id: execution-state
title: ExecutionState lifecycle enum
complexity: low
mode: autopilot
depends_on: [workspace-setup]
---

## Tasks

### core-003-2-2-create-execution-state

Create `crates/runtime/src/runtime/lifecycle.rs` with `#[non_exhaustive]` enum `ExecutionState`: `Active`, `Draining`, `Terminated`, `Failed`. Implements `Clone + Debug + PartialEq + Send + Sync`.

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/spec.md` (lines 41-79)
- `openspec/changes/core-003-runtime-actor-execution/design.md` (lines 119-126)

## Files changed

- `crates/runtime/src/runtime/lifecycle.rs` — CREATE

## Completion

- `cargo check -p ego-runtime` passes
- Enum has exactly 4 spec-compliant variants
- No actor vocabulary

## Dependencies

Requires workspace-setup (mod.rs scaffold with `pub mod lifecycle`).
