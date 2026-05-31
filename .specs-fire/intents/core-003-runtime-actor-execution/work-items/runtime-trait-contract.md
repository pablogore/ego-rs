---
id: runtime-trait-contract
title: Runtime trait and contract modules
complexity: medium
mode: confirm
depends_on: [workspace-setup, execution-id, execution-state, failure-types, runtime-handle]
---

## Tasks

### core-003-3-1-rewrite-runtime-trait

Rewrite `crates/runtime/src/runtime/runtime.rs` with `Runtime` trait:

```rust
pub trait Runtime: Send + Sync + 'static {
    fn spawn<F, Fut>(&self, f: F, name: Option<&str>) -> Result<ExecutionId, SpawnError>
    where F: FnOnce(RuntimeHandle) -> Fut + Send + 'static,
          Fut: Future<Output = ()> + Send + 'static;
    fn send<M>(&self, id: &ExecutionId, msg: M) -> Result<(), SendError>
    where M: Send + 'static;
    fn shutdown(&self, id: &ExecutionId);
    fn state(&self, id: &ExecutionId) -> Option<ExecutionState>;
}
```

Delete old flat `src/runtime.rs` (contained GAT-based trait with actor types).

### core-003-3-2-rewrite-isolation-module

Rewrite `crates/runtime/src/runtime/isolation.rs` as doc-only contract. Delete old flat `src/isolation.rs` (contained `Isolation` enum).

### core-003-3-3-rewrite-scheduler-module

Create `crates/runtime/src/runtime/scheduler.rs` as doc-only contract. Delete old flat `src/scheduling.rs` (contained `SchedulingPolicy` enum). Update `mod.rs` if module name changed.

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/design.md` (lines 83-141)
- `openspec/changes/core-003-runtime-actor-execution/spec.md` (lines 186-208)
- `crates/runtime/src/runtime.rs` (old file to DELETE)
- `crates/runtime/src/isolation.rs` (old file to DELETE)
- `crates/runtime/src/scheduling.rs` (old file to DELETE)

## Files changed

- `crates/runtime/src/runtime/runtime.rs` — REWRITE
- `crates/runtime/src/runtime/isolation.rs` — REWRITE (doc only)
- `crates/runtime/src/runtime/scheduler.rs` — CREATE (or REWRITE)
- `crates/runtime/src/runtime.rs` — DELETE
- `crates/runtime/src/isolation.rs` — DELETE
- `crates/runtime/src/scheduling.rs` — DELETE
- `crates/runtime/src/runtime/mod.rs` — verify module name (scheduler vs scheduling)

## Completion

- `cargo check -p ego-runtime` passes
- Trait has exactly 4 methods, no GATs
- No `ActorId`, `ActorLifecycleState`, `SupervisionStrategy` imports
- Isolation and scheduler modules are doc-only (no runtime types)
- Old flat files removed

## Dependencies

Requires all vocabulary types (ExecutionId, ExecutionState, SendError, SpawnError, RuntimeHandle).
