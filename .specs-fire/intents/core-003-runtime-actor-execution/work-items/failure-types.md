---
id: failure-types
title: SendError and SpawnError types
complexity: low
mode: autopilot
depends_on: [workspace-setup, execution-id]
---

## Tasks

### core-003-2-3-create-failure-types

Create `crates/runtime/src/runtime/failure.rs` with:

- `SendError` struct: `id: ExecutionId`, `cause: SendErrorKind`. Implements `Debug + Display + std::error::Error`.
- `SendErrorKind` enum: `NotFound`, `Closed`. `#[non_exhaustive]`.
- `SpawnError` struct: `pub cause: SpawnErrorKind`. Implements `Debug + Display + std::error::Error`.
- `SpawnErrorKind` enum: `Closed`, `Internal`. `#[non_exhaustive]`.

Delete old flat `src/error.rs` (contained `RuntimeError` enum).

## Inputs

- `openspec/changes/core-003-runtime-actor-execution/spec.md` (lines 83-183)
- `openspec/changes/core-003-runtime-actor-execution/design.md` (lines 143-157)
- `crates/runtime/src/error.rs` (old file to DELETE)
- `crates/runtime/src/runtime/execution.rs` (for `ExecutionId` import)

## Files changed

- `crates/runtime/src/runtime/failure.rs` — CREATE
- `crates/runtime/src/error.rs` — DELETE

## Completion

- `cargo check -p ego-runtime` passes
- No `RuntimeError` enum anywhere
- No `MailboxFull` variant
- All error types implement required traits

## Dependencies

Requires workspace-setup (mod.rs) + execution-id (for `ExecutionId` in `SendError`).
