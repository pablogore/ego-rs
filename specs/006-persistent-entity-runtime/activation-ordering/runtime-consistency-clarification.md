# Final Runtime Consistency Clarification — CORE-006 + Spec-007

**Date**: 2026-06-07  
**Status**: Final  
**Scope**: Removes all remaining implementation ambiguity in Mutex scope, mailbox ownership, registry visibility timing, recovery barrier ordering, and failure safety during initialization.

---

## A. Final Lifecycle Ordering

### Complete Deterministic Sequence

```
Command Arrival
  │
  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ 1. registry.get_active_sender::<C>(&entity)                                 │
│    → Active? Yes ──────────────► mailbox_tx.send(envelope)                  │
│    → Active? No  ──► Activate                                                │
└─────────────────────────────────────────────────────────────────────────────┘
                                                                               
┌─ ACTIVATION ─────────────────────────────────────────────────────────────────┐
│                                                                              │
│ 2. registry.get_or_create_activation(&entity)                                │
│    → Creates SharedActivation { lock, result_tx, result_rx }                │
│    → Stores Arc<SharedActivation> in pending_activations                     │
│                                                                              │
│ 3. activation.lock.lock().await  ←── MUTEX ACQUIRED                          │
│    ╔══════════════════════════════════════════════════════════════╗           │
│    ║  SINGLE-FLIGHT CRITICAL SECTION (exactly 1 caller passes)  ║           │
│    ╚══════════════════════════════════════════════════════════════╝           │
│                                                                              │
│ 4. Re-check: registry.get_active_sender::<C>(&entity)                       │
│    → Found? Yes ──► drop(guard), redirect to existing mailbox               │
│    → NOT found ──► Proceed with spawn                                        │
│                                                                              │
│ 5. (mailbox_tx, mailbox_rx) = mpsc::channel(capacity)   ← MAILBOX CREATED   │
│                                                                              │
│ 6. tokio::spawn(async move {                                                │
│       let actor = EntityActor {                                             │
│           mailbox: mailbox_rx,   ← mailbox RECEIVER given to actor          │
│           state: None,                                                      │
│           version: 0,                                                       │
│           lifecycle: LifecycleState::Recovering,                            │
│           ...                                                               │
│       };                                                                    │
│       actor.run().await;          ← actor starts but may NOT YET be running │
│    })                                                                        │
│                                                                              │
│ 7. registry.insert_active(entity, ActorHandle::new(mailbox_tx, join))       │
│    ╔══════════════════════════════════════════════════════════════╗          │
│    ║  REGISTRY VISIBILITY POINT — actor now findable by other    ║          │
│    ║  callers. Sender committed BEFORE mutex released.          ║          │
│    ╚══════════════════════════════════════════════════════════════╝          │
│                                                                              │
│ 8. registry.remove_passivated(&entity)   ← clear stale passivated entry     │
│                                                                              │
│ 9. drop(guard)                           ←── MUTEX RELEASED                  │
│                                                                              │
│ 10. registry.remove_activation(&entity)  ← clean up pending entry           │
│                                                                              │
│ 11. mailbox_tx.send(envelope).await      ← FIRST COMMAND IN MAILBOX         │
│     (actor may still be in recover_state() — cmd is buffered)               │
│                                                                              │
│ 12. response_rx.await → return result to caller                             │
└─────────────────────────────────────────────────────────────────────────────┘
                                                                               
┌─ ACTOR TASK ────────────────────────────────────────────────────────────────┐
│                                                                              │
│ 13. EntityActor::run() begins execution                                     │
│     lifecycle = Recovering                                                   │
│                                                                              │
│ 14. recover_state().await  ←── RECOVERY BARRIER                              │
│     ┌─ persistence.load_for_recovery() → (snap, events)                     │
│     ├─ deserialize snapshot → state S                                       │
│     ├─ for each event: apply_event(&state, event).await                     │
│     ├─ self.state = Some(state)                                             │
│     ├─ self.version = count                                                 │
│     └─ transition(LifecycleState::Active)   ←── RECOVERY COMPLETE           │
│                                                                              │
│     On failure: transition(Failed) → remove_active() → return               │
│                                                                              │
│ 15. process_commands().await  ←── COMMAND PROCESSING BEGINS                 │
│     ┌─ Commands queued during recovery drained FIFO                        │
│     ├─ Each command: handle_command → persist → reload → apply_event        │
│     └─ Timeout reached → should_passivate() → break                         │
│                                                                              │
│ 16. passivate().await                                                        │
│     ┌─ Drain remaining commands from mailbox                               │
│     ├─ Store final snapshot                                                 │
│     └─ mark_passivated(entity, version) → actor task ends                   │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## B. Exact Ownership Rules

### 1. Mutex Scope & Lifetime

| Property | Value |
|----------|-------|
| **What it guards** | The spawn-vs-redirect decision window |
| **When acquired** | After `get_or_create_activation()`, before re-checking active map |
| **Scope** | Registry lookup → mailbox creation → spawn → insert_active → **RELEASED** |
| **Duration** | Does NOT include recovery |
| **Release point** | Immediately after `insert_active()`, before first command send |
| **Who holds it** | The spawning caller (`EntityRef::activate_and_send()`) — exclusively |
| **Concurrent callers** | Block on `lock.lock().await`; after release, re-check active map and redirect |
| **Panic safety** | `drop(guard)` on scope exit (Rust drop guarantee) |

**Final rule**: Mutex is held ONLY during activation setup, NOT during recovery.

### 2. Mailbox Ownership

| Component | Owner | Lifetime |
|-----------|-------|----------|
| **`mpsc::Sender`** | Registry (`ActorHandle.sender`) | From `insert_active()` until entity is passivated or actor fails |
| **`mpsc::Receiver`** | Actor task (`EntityActor.mailbox`) | From spawn until actor task completes |
| **Channel itself** | Shared (tokio internal) | Until all Senders and Receiver are dropped |

**The Sender is NOT shared via `Arc`** — it is stored inside `ActorHandle` in the Registry's `active` map. Callers clone the Sender when retrieving it from the registry via `downcast_sender::<C>().cloned()`.

**Validity timeline**:
- **Before recovery**: Valid — the Sender is in the registry before mutation guard is released
- **During recovery**: Valid — the Sender remains in the registry throughout
- **After recovery**: Valid — the Sender remains in the registry until passivation or failure

### 3. Registry Visibility Point

The ActorHandle becomes visible in the registry **at step 7** — `insert_active()` — which is:

- ✅ BEFORE recovery starts
- ✅ BEFORE the mutex is released  
- ✅ BEFORE the first command is sent

This means other callers can find the entity and send commands while the mailbox is still empty and recovery hasn't started. Those commands queue safely in the channel buffer.

---

## C. Failure-Safe Invariant

### Mid-Spawn Failure Scenarios

| Failure Point | Actor State | Registry State | Mailbox State | Recovery Behavior |
|---------------|-------------|----------------|---------------|-------------------|
| **After `tokio::spawn()` but before `insert_active()`** | Actor task exists, will run (or panic) | No entry in active map | Sender unreachable (not yet stored) | Subsequent callers see NO active entity → trigger NEW activation → new spawn |
| **After `insert_active()` but before recovery completes** | Actor task runs, enters `recover_state()` | Entry in active map (findable) | Sender stored, commands can arrive | If recovery fails → transition(Failed) → `remove_active()` → mailbox dropped → subsequent callers trigger new activation |
| **After recovery but during command processing** | Actor is ACTIVE and processing | Entry in active map | Sender stored | If command fails → specific error returned to caller; entity may stay ACTIVE or transition to FAILED |
| **Actor panics during recovery** | Task terminates (JoinHandle error) | Entry in active map (stale) | Sender stored but Receiver dropped | `send()` on Sender returns error → caller detects closed mailbox → treats as inactive → triggers new activation |
| **Actor panics during command processing** | Task terminates (JoinHandle error) | Entry in active map (stale) | Sender stored but Receiver dropped | Same as above — sender detects close, caller triggers new activation |

### Guarantees

| Concern | Guarantee |
|---------|-----------|
| **No double actor spawn** | Mutex serializes activation; second caller re-checks and redirects |
| **No orphan mailbox** | Channel dropped when both Sender (from registry removal) and Receiver (from task end) are dropped |
| **No leaked activation guard** | `drop(guard)` fires on scope exit even on panic (Rust unwind safety) |
| **No leaked pending_activation entry** | `remove_activation()` releases the HashMap entry after mutex release |
| **Stale actor cleanup** | Stale entry in active map is harmless — next `send()` detects closed channel and callers trigger fresh activation, which overwrites via `insert_active()` |
| **Message loss** | Commands are in mpsc buffer or caller's oneshot channel; channel close propagates error, caller can retry |

---

## D. Final Concurrency Guarantee

### No Race Window Analysis

| Window | Steps | Race Exists? | Why |
|--------|-------|-------------|-----|
| **Activation decision** | 2–4 | **No** | Mutex held; single caller proceeds; second blocks then re-checks |
| **Mailbox creation** | 5 | **No** | Local variables; not yet shared |
| **Registry insertion** | 7 | **No** | Mutex held; atomic `HashMap::insert` under async Mutex |
| **Mutex release → first command** | 9–11 | **No** | Sender is already in registry; other callers may send concurrently but this is safe (mpsc is multi-producer) |
| **Recovery → command processing** | 14–15 | **No** | `recover_state()` is sequential; `process_commands()` starts only after it returns |
| **Passivation → re-activation** | 16 → 1 | **No** | `mark_passivated()` removes active entry; subsequent `send()` observes no active sender → fresh activation |
| **Concurrent sends to same entity** | 11 onwards | **No** | mpsc guarantees FIFO ordering; Mutex ensures single writer for activation; all senders are producers to same channel |
| **Actor panic + concurrent send** | N/A | **No** | `send()` returns error → caller sees entity as inactive → triggers activation with Mutex guard |

### Final Invariant Statement

> **"At no point in the entire lifecycle from command arrival to actor passivation can a caller observe or interact with a partially initialized actor, and at no point can two actors exist for the same entity."**

This is enforced by:
1. **Mutex** serializes the spawn decision (single-flight)
2. **Registry visibility** comes after spawn and before recovery (actor is findable but commands only queue)
3. **Recovery barrier** prevents command dispatch before state is complete
4. **Channel semantics** prevent observing partial state (Receiver never peeked during recovery)
5. **Drop semantics** clean up all resources on failure without explicit teardown code
