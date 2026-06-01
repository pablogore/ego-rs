---
id: tokio-runtime-core
title: TokioRuntime implementation (spawn, send, shutdown, isolation)
complexity: high
mode: validate
depends_on: [lib-reexports]
---

## Tasks

### core-003-5-1-impl-tokio-runtime-struct

Implement `TokioRuntime` struct wrapping `tokio::runtime::Runtime` with internal execution unit registry (e.g., `HashMap<ExecutionId, UnitState>`). Implement `spawn`: create `RuntimeHandle` with closures wired to unit's mpsc channel, register unit, spawn wrapped future on tokio, return `Ok(ExecutionId)`.

### core-003-5-2-impl-tokio-send-routing

Implement `send`: look up unit by `ExecutionId`, send message via unit's channel. Wire sequential message processing loop per unit (receiver polling, in-order dispatch).

### core-003-5-3-impl-tokio-shutdown-isolation

Implement `shutdown` (Active -> Draining -> Terminated), `state` (return tracked state), panic catching per unit (`catch_unwind` -> Failed for that unit only), fail-closed on internal error (all units Failed, reject new spawn/send).

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/design.md` (lines 119-167, 186-203)
- `openspec/changes/core-003-runtime-actor-execution/spec.md` (lines 213-268)
- `crates/runtime-tokio/src/lib.rs` (current stub)

## Files changed

- `crates/runtime-tokio/src/lib.rs` — MAJOR REWRITE

## Completion

- `cargo check -p ego-runtime-tokio` passes
- `spawn` returns `Ok(ExecutionId)` and creates unit with `RuntimeHandle`
- `send` routes messages to correct unit
- Sequential in-order delivery per unit
- Panic in one unit sets that unit to Failed, other units unaffected
- `shutdown` transitions unit through lifecycle states
- Fail-closed: on internal error, all spawns return `Err(SpawnError { cause: SpawnErrorKind::Closed })`
- No `goakt`, `protoactor`, `akka` imports

## Dependencies

Requires lib-reexports (so `ego-runtime` types are available via crate dependency).
