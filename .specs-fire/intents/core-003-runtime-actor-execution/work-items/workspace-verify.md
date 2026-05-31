---
id: workspace-verify
title: Workspace-wide verification
complexity: low
mode: autopilot
depends_on: [null-runtime-tests, tokio-integration-tests, layers-toml]
---

## Tasks

### core-003-7-3-workspace-verification

Full workspace verification:

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

## Inputs

All previous work items complete.

## Files changed

None (verification only).

## Completion

- `cargo check --workspace` succeeds (no errors)
- `cargo test --workspace` passes all tests
- `cargo clippy --workspace -- -D warnings` succeeds (no warnings)
- No regressions in existing workspace members (domain, application, infrastructure, transport, runtime-slice)

## Dependencies

Requires all implementation and test work items complete.
