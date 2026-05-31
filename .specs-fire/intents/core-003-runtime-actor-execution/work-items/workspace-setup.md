---
id: workspace-setup
title: Workspace setup
complexity: low
mode: autopilot
depends_on: []
---

## Tasks

### core-003-1-1-fix-runtime-cargo-toml

Fix `crates/runtime/Cargo.toml`: remove `ego-domain`, `chrono`, `serde`, `serde_json`, `mockall`; add `uuid = { version = "1", features = ["v4"] }`.

### core-003-1-2-verify-runtime-tokio-cargo-toml

Verify `crates/runtime-tokio/Cargo.toml` has correct deps (`ego-runtime` path + `tokio` full features).

### core-003-1-3-create-runtime-mod-scaffold

Create `crates/runtime/src/runtime/mod.rs` with `pub mod` declarations for: `runtime`, `execution`, `lifecycle`, `failure`, `handle`, `scheduler`, `isolation`.

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/design.md` (lines 42-54, 220-228)
- `openspec/changes/core-003-runtime-actor-execution/spec.md` (lines 376-413)
- `crates/runtime/Cargo.toml`, `crates/runtime-tokio/Cargo.toml`

## Files changed

- `crates/runtime/Cargo.toml` — rewrite deps
- `crates/runtime-tokio/Cargo.toml` — verify/fix
- `crates/runtime/src/runtime/mod.rs` — CREATE

## Completion

- `cargo check -p ego-runtime` succeeds
- `cargo check -p ego-runtime-tokio` succeeds
- `grep -q 'uuid' crates/runtime/Cargo.toml`
- `! grep -q 'ego-domain' crates/runtime/Cargo.toml`
- Module scaffold exists with 7 declarations

## Dependencies

None (first work item).
