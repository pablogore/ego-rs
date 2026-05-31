---
id: null-runtime-tests
title: NullRuntime test double and contract tests
complexity: medium
mode: confirm
depends_on: [runtime-trait-contract, lib-reexports]
---

## Tasks

### core-003-7-1-add-null-runtime-tests

Add `NullRuntime` struct in `#[cfg(test)]` module of `crates/runtime/src/runtime/runtime.rs`:

- Returns distinct `ExecutionId` per `spawn` call
- Tracks units in internal state registry
- `send` stores messages for test assertion
- `shutdown` sets state to `Terminated`
- `state` returns tracked state

Tests:
- `test_spawn_returns_unique_id`: spawn twice, ids differ
- `test_spawn_after_shutdown_returns_error`: spawn after shutdown -> `Err(SpawnErrorKind::Closed)`
- `test_send_to_unknown_id_returns_error`: send to non-existent id -> `SendError`
- `test_shutdown_terminates_unit`: spawn, shutdown, state is Terminated
- `test_failure_isolation`: unit panics, other units unaffected

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/spec.md` (lines 332-360)
- `openspec/changes/core-003-runtime-actor-execution/design.md` (line 258)
- `crates/runtime/src/runtime/runtime.rs`

## Files changed

- `crates/runtime/src/runtime/runtime.rs` — add `#[cfg(test)]` module

## Completion

- `cargo test -p ego-runtime` passes all tests
- No Tokio dependency in test module
- All contract semantics verified without any backend

## Dependencies

Requires Runtime trait + all vocabulary types + lib.rs exports.
