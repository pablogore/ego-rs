---
id: tokio-integration-tests
title: TokioRuntime integration tests
complexity: medium
mode: confirm
depends_on: [tokio-runtime-core, tokio-runtime-config]
---

## Tasks

### core-003-7-2-add-tokio-integration-tests

Create `crates/runtime-tokio/tests/tokio_runtime_tests.rs` with tests:

- `test_multi_threaded_default`: default runtime, spawn, verify
- `test_current_thread`: current-thread runtime, spawn, verify
- `test_send_message`: spawn, send message, verify delivery
- `test_sequential_delivery`: spawn, send multiple messages, verify order
- `test_failure_isolation`: spawn unit that panics, other units unaffected
- `test_shutdown`: spawn, shutdown, verify termination
- `test_configured_worker_threads`: build with 4 workers, spawn, verify
- `test_fail_closed`: internal error, spawn returns Err, send returns Err

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/spec.md` (lines 248-268)
- `crates/runtime-tokio/src/lib.rs`

## Files changed

- `crates/runtime-tokio/tests/tokio_runtime_tests.rs` — CREATE

## Completion

- `cargo test -p ego-runtime-tokio` passes all tests
- All 8+ test scenarios pass

## Dependencies

Requires TokioRuntime with all methods implemented.
