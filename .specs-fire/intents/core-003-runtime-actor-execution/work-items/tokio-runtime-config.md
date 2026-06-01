---
id: tokio-runtime-config
title: TokioRuntimeBuilder and DefaultRuntime alias
complexity: low
mode: autopilot
depends_on: [tokio-runtime-core]
---

## Tasks

### core-003-5-4-add-tokio-runtime-builder

Add `TokioRuntimeBuilder` with: `worker_threads(self, n: usize) -> Self`, `current_thread(self) -> Self`, `build(self) -> TokioRuntime`. Configures `tokio::runtime::Builder` internally.

### core-003-5-5-add-default-runtime-alias

Add `pub type DefaultRuntime = TokioRuntime;` and `impl Default for TokioRuntime` (multi-threaded with available parallelism).

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/spec.md` (lines 272-328)
- `openspec/changes/core-003-runtime-actor-execution/design.md` (lines 206-207)
- `crates/runtime-tokio/src/lib.rs`

## Files changed

- `crates/runtime-tokio/src/lib.rs` — add builder + alias

## Completion

- `cargo check -p ego-runtime-tokio` passes
- `TokioRuntime::default()` creates multi-threaded runtime
- `TokioRuntime::builder().current_thread().build()` creates current-thread runtime
- `TokioRuntime::builder().worker_threads(4).build()` creates runtime with 4 workers

## Dependencies

Requires TokioRuntime struct to exist (tokio-runtime-core).
