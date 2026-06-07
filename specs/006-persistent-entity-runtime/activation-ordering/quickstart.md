# Quickstart: Activation Ordering Model Validation

**Date**: 2026-06-07  
**Prerequisites**: Rust 1.75+, existing `cargo` workspace

## Setup

```bash
# From repository root
cargo build --package ego-persistent-entity
cargo test  --package ego-persistent-entity
```

Expected: clean build, 12 tests passing.

## Validation Scenarios

### Scenario 1: Single-Flight Activation

**Goal**: Verify that concurrent commands to a PASSIVATED entity produce exactly one actor.

1. Open `crates/persistent-entity/src/entity_ref.rs`
2. Verify the activation flow:
   - `get_or_create_activation()` creates a per-entity `SharedActivation`
   - `activation.lock.lock().await` serializes concurrent callers
   - Second caller, after acquiring lock, re-checks `get_active_sender()` and redirects

**Expected**: Only one `tokio::spawn()` call per entity per activation cycle.

### Scenario 2: Recovery Barrier

**Goal**: Verify that commands are not executed during recovery.

1. Open `crates/persistent-entity/src/actor.rs`
2. Verify `EntityActor::run()`:
   - `recover_state().await` is called before `process_commands().await`
   - The mailbox `Receiver` is stored in `self.mailbox`
   - `process_commands()` is the only place that calls `self.mailbox.recv()`

**Expected**: No `mailbox.recv()` call occurs in `recover_state()`.

### Scenario 3: Registry Visibility Timing

**Goal**: Verify that `insert_active()` happens before recovery.

1. Open `crates/persistent-entity/src/entity_ref.rs`
2. Trace the `activate_and_send()` method:
   - `registry.insert_active(entity, ActorHandle::new(mailbox_tx, join))` at line 135-138
   - `actor.run().await` inside the spawned task calls `recover_state()` first

**Expected**: Registry has the Sender before the actor enters recovery.

### Scenario 4: First Command During Recovery

**Goal**: Verify that the first command arrives in the mailbox before or during recovery.

1. In `entity_ref.rs:activate_and_send()`:
   - `mailbox_tx.send(envelope).await` at line 144
   - This executes after `drop(guard)` but the actor task may still be in `recover_state()`
2. The command sits in the mpsc channel buffer until `process_commands()` drains it

**Expected**: First command is queued, not executed, until recovery completes.

### Scenario 5: Panic Recovery

**Goal**: Verify that actor panic leads to retry on next command.

1. Actor panics → JoinHandle completes with error
2. Registry still has `ActorHandle` with dead Sender
3. Next `send()` → `mailbox_tx.send(envelope)` → returns error (channel closed)
4. Caller receives error → treats entity as inactive → triggers new activation

**Expected**: No stale actor blocks future commands.

## Running Tests

```bash
# Unit tests
cargo test --package ego-persistent-entity

# Full workspace (check no regressions)
cargo test
```

## Design Documents

| Document | Path |
|----------|------|
| Spec | `specs/007-activation-ordering-model/spec.md` |
| Implementation Skeleton | `specs/007-activation-ordering-model/implementation-skeleton.md` |
| Runtime Consistency | `specs/007-activation-ordering-model/runtime-consistency-clarification.md` |
| Registry Visibility Semantics | `specs/007-activation-ordering-model/registry-visibility-semantics.md` |
| Data Model | `specs/007-activation-ordering-model/data-model.md` |
| Contracts | `specs/007-activation-ordering-model/contracts/README.md` |
| Research Findings | `specs/007-activation-ordering-model/research.md` |
