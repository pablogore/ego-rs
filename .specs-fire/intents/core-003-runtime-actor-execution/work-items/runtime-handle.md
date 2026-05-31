---
id: runtime-handle
title: RuntimeHandle with closure-based internals
complexity: low
mode: autopilot
depends_on: [workspace-setup, execution-id, execution-state, failure-types]
---

## Tasks

### core-003-2-4-create-runtime-handle

Create `crates/runtime/src/runtime/handle.rs` with:

- `RuntimeHandle` struct using `Arc<dyn Fn(...)>` closures for all operations
- No `dyn Runtime` stored (Runtime is not object-safe)
- Public methods: `id() -> ExecutionId`, `send_self<M: Send + 'static>(msg) -> Result<(), SendError>`, `shutdown()`, `state() -> Option<ExecutionState>`
- Implements `Clone + Send + Sync`
- Internal boxing via `Box<dyn Any + Send>` (hidden from public API)

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/design.md` (lines 197-203, 219-227)
- `crates/runtime/src/runtime/execution.rs`, `lifecycle.rs`, `failure.rs`, `mod.rs`

## Files changed

- `crates/runtime/src/runtime/handle.rs` — CREATE

## Completion

- `cargo check -p ego-runtime` passes
- All 4 public methods exist
- No `dyn Runtime` in struct fields
- `send_self` is generic (not `Box<dyn Any>` in public signature)

## Dependencies

Requires all previous vocabulary types: ExecutionId, ExecutionState, SendError.
