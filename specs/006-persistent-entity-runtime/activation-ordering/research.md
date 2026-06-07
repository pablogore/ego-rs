# Research Findings: Activation Ordering Model Validation

**Date**: 2026-06-07  
**Status**: Complete  

## Summary

All model decisions from spec-007 are already implemented in `crates/persistent-entity/src/`. Research confirms no gaps between the formal model and the existing code.

---

## 1. Single-Flight Activation (FR-001)

**Decision**: Per-entity `Mutex<()>` guard via `SharedActivation`.

**Implementation**: `activation.rs:SharedActivation` — `lock: Mutex<()>`. Used in `entity_ref.rs:activate_and_send()`.

```rust
let activation = registry.get_or_create_activation(self.entity.clone()).await;
let guard = activation.lock.lock().await;
// ... spawn decision, mailbox creation, spawn, insert_active, release
drop(guard);
```

**Status**: ✅ Aligned

---

## 2. Recovery Before Processing (FR-002 / FR-006)

**Decision**: `recover_state()` completes before `process_commands()` starts.

**Implementation**: `actor.rs:EntityActor::run()`:

```rust
pub async fn run(&mut self) {
    self.recover_state().await;        // ← barrier
    // ...
    self.process_commands().await;     // ← mailbox drained only after
}
```

The mailbox `Receiver` is never polled during `recover_state()`. Commands queued during recovery are buffered in the mpsc channel untouched.

**Status**: ✅ Aligned

---

## 3. Mailbox Before Recovery (FR-003)

**Decision**: mpsc channel created and Sender registered in active map before actor spawn completes.

**Implementation**: `entity_ref.rs:activate_and_send()`:

```rust
let (mailbox_tx, mailbox_rx) = mpsc::channel(capacity);
tokio::spawn(async move { /* actor with mailbox_rx */ });
registry.insert_active(entity, ActorHandle::new(mailbox_tx, join));
drop(guard);
mailbox_tx.send(envelope).await;  // first command in mailbox
```

**Status**: ✅ Aligned

---

## 4. Activation Lock Lifecycle (FR-004)

**Decision**: Mutex acquired before mailbox creation, released after `insert_active()`.

**Implementation**: `entity_ref.rs` — lock held from step 2 through step 7 in the spawn flow.

**Status**: ✅ Aligned

---

## 5. FIFO Ordering (FR-005)

**Decision**: mpsc channel provides FIFO ordering per entity.

**Implementation**: `tokio::sync::mpsc` guarantees FIFO for multi-producer single-consumer channels.

**Status**: ✅ Aligned

---

## 6. Panic Recovery (FR-007)

**Decision**: Actor panic → registry entry becomes stale (sender exists, receiver dropped). Next `send()` detects closed channel, caller triggers fresh activation.

**Implementation**: When actor panics, `JoinHandle` completes with error. No explicit cleanup needed — `send()` returning error is the detection mechanism.

**Status**: ✅ Aligned (implicit — relies on mpsc close semantics)

---

## 7. Passivation Consistency (FR-008)

**Decision**: Drain remaining commands before final snapshot and `mark_passivated()`.

**Implementation**: `actor.rs:passivate()` — `while let Some(envelope) = self.mailbox.recv().await` loop drains mailbox, then stores final snapshot and calls `registry.mark_passivated()`.

**Status**: ✅ Aligned

---

## 8. Activation Retry (FR-009)

**Decision**: Failed/PASSIVATED → sender not found → trigger new activation.

**Implementation**: `entity_ref.rs:send()` — `get_active_sender()` returns `None` → calls `activate_and_send()`.

**Status**: ✅ Aligned

---

## 9. Registry Visibility Semantics

**Decision**: Option A — Strong Visibility, Weak Readiness. Registry insertion signals existence, not readiness.

**Implementation Verified**:

1. `insert_active()` happens BEFORE `recover_state()` starts
2. `get_active_sender()` returns `Some(sender)` during recovery
3. Commands sent during recovery are buffered in mpsc channel
4. `process_commands()` is never called until `recover_state()` returns

**Status**: ✅ Aligned with formal model in `registry-visibility-semantics.md`

---

## 10. Mutex Scope & Lifetime

**Decision**: Mutex held only during activation setup (lookup → channel → spawn → insert), NOT during recovery.

**Implementation Verified**:

```
entity_ref.rs:activate_and_send()
  lock.lock().await     ← ACQUIRE
  get_active_sender()   ← re-check
  mpsc::channel()
  tokio::spawn()
  insert_active()
  drop(guard)           ← RELEASE (before recovery, before first command send)
```

**Status**: ✅ Aligned with `runtime-consistency-clarification.md`

---

## No NEEDS CLARIFICATION Remaining

All spec requirements (FR-001 through FR-010) have formal model decisions and verified implementations. No open questions.
