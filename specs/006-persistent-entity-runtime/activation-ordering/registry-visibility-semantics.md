# Formal Registry Visibility Semantics — CORE-006 + Spec-007

**Date**: 2026-06-07  
**Status**: Final  
**Scope**: Formal definition of the "visible-but-not-ready" entity state during recovery, eliminating ambiguity between existence, readiness, and executability.

---

## Model Decision: Option A — Strong Visibility, Weak Readiness

Selected and formalized below.

---

## A. Final Meaning of Registry Visibility

### Core Definition

**Registry insertion is an existence signal, not a readiness signal.**

The registry serves two distinct functions, and they must be separated:

| Concept | Definition | Implemented By |
|---------|------------|----------------|
| **Existence** | An actor task exists for this entity | `EntityRegistry.active` contains `ActorHandle` |
| **Readiness** | The actor is ready to execute commands | `EntityActor.lifecycle.state() == Active` |

When `insert_active()` completes, the entity has **existence** but may not yet have **readiness**. Readiness is achieved only when `recover_state()` completes and the lifecycle transitions to `Active`.

### Why This Distinction Exists

The registry must be updated before the mutex is released to prevent double spawns. If we delayed registry insertion until after recovery, the mutex would need to be held throughout recovery — blocking all concurrent commands and defeating the purpose of the mailbox buffer.

The tradeoff is: **existence without readiness**, where the registry says "an actor exists" but the lifecycle state says "not ready yet."

### Three-State Visibility Model

```
EntityTriple
  │
  ├── NOT IN REGISTRY (neither exists nor ready)
  │     └── State: PASSIVATED or brand new
  │
  ├── IN REGISTRY + NOT READY (exists but not executable)
  │     └── State: RECOVERING (lifecycle == Recovering)
  │
  └── IN REGISTRY + READY (exists and executable)
        └── State: ACTIVE (lifecycle == Active)
```

---

## B. Exact Lifecycle State Interpretation During Recovery

### State Lattice

```
                    ┌──────────────┐
                    │  NOT IN      │
                    │  REGISTRY    │
                    │  (unknown)   │
                    └──────┬───────┘
                           │
                    ┌──────▼───────┐
                    │  REGISTRY    │
                    │  INSERTED    │
                    │  (EXISTS)    │
                    └──────┬───────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
              ▼            ▼            ▼
     ┌────────────┐ ┌───────────┐ ┌──────────┐
     │ RECOVERING │ │  ACTIVE   │ │ PASSIVAT.│
     │ (exists)   │ │ (ready)   │ │ (exists) │
     │ NOT ready  │ │ executable│ │ executable│
     └────────────┘ └───────────┘ └──────────┘
                           │
                           ▼
                    ┌──────────┐
                    │PASSIVATED│
                    │ removed  │
                    │from reg. │
                    └──────────┘
```

### State Interpretation Table

| Lifecycle State | In Registry? | Ready? | Executable? | Commands |
|-----------------|--------------|--------|-------------|----------|
| `Recovering` | Yes | No | No | Buffered in mailbox |
| `Active` | Yes | Yes | Yes | Executed immediately |
| `Passivating` | Yes | Yes (draining) | Yes | Executed immediately |
| `Passivated` | No | N/A | N/A | Triggers activation |
| `Failed` | No (removed) | No | No | Triggers new activation |

---

## C. Command Buffering/Execution Rules

### Rule Set

```
┌────────────────────────────────────────────────────────────────────┐
│ COMMAND DISPATCH MATRIX                                           │
├────────────────────────────────────────────────────────────────────┤
│                                                                    │
│  sender = registry.get_active_sender::<C>(&entity)                │
│       │                                                           │
│       ├── None ──► entity NOT visible ──► trigger activation      │
│       │                                                           │
│       └── Some(tx) ──► entity IS visible                          │
│               │                                                   │
│               ├── actor.state == Recovering ──► SEND TO MAILBOX   │
│               │       (command buffered, NOT executed)            │
│               │       caller awaits response_rx                   │
│               │       response blocked until recovery completes   │
│               │       AND command reaches front of queue          │
│               │                                                   │
│               ├── actor.state == Active ──► SEND TO MAILBOX      │
│               │       (command delivered, executed immediately    │
│               │        or as soon as actor is idle)               │
│               │                                                   │
│               ├── actor.state == Passivating ──► SEND TO MAILBOX │
│               │       (command processed before passivation)      │
│               │                                                   │
│               └── tx.send() returns error ──► actor dead          │
│                       (channel closed)                            │
│                       caller treats as NOT visible                │
│                       triggers new activation                     │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
```

### Key Invariants

1. **Commands are NEVER executed during RECOVERING.** The actor's `run()` method calls `recover_state().await` to completion before entering `process_commands()`. The mailbox receiver is not polled during recovery.

2. **The caller does not know whether the entity is RECOVERING or ACTIVE.** The caller only knows the entity exists (sender found) or doesn't (sender not found). The recovery state is invisible to callers.

3. **Response latency includes recovery time for early arrivals.** The first caller to activate an entity will wait for recovery to complete plus command execution. Subsequent callers experience only command execution latency.

4. **Mailbox capacity must accommodate commands arriving during recovery.** This is a configuration concern (`RuntimeConfig.mailbox_capacity`).

---

## D. Consistency Guarantee Under Concurrency

### Formal Invariant

> **∀ entities E at time t:**
> 
> If `E` is visible in the registry at time `t` (∃ ActorHandle in active map), then EXACTLY ONE of the following holds:
> 
> 1. **E is in RECOVERING state**: No command from E's mailbox will be dispatched until `t + δ` where `δ` is the time to complete `recover_state()`.
> 
> 2. **E is in ACTIVE state**: Commands from E's mailbox are being processed sequentially, FIFO.
> 
> 3. **E is in PASSIVATING state**: Commands are being drained; no new commands will be accepted after the mailbox closes.
>
> In all cases, EXACTLY ONE actor task exists for E, and no caller observes partial state.

### Concurrency Safety Proof

| Concern | How Addressed |
|---------|---------------|
| **Double activation** | Mutex serializes spawn; second caller re-checks active map after acquiring lock |
| **Command to non-ready entity** | Accepted via mpsc channel; `recover_state().await` acts as barrier before `process_commands()` |
| **Caller sees partial recovery** | Impossible — caller only sees response through `response_rx`, which is populated after command reaches front of queue and executes |
| **Actor dies during recovery** | Commands in mailbox are dropped (Receiver dropped); callers waiting on `response_rx` get canceled; next `send()` detects closed sender and triggers new activation |
| **Registry has stale entry** | Entry in active map with dead sender → `send()` returns error → caller retriggers activation (which overwrites stale entry) |
| **Recovery completes after command arrives** | Correctness preserved — recovery barrier ensures order: all recovery events replayed before any command from mailbox is read |

### Correctness Argument

The system is correct because:

1. **Existence ≠ Readiness**: The registry signals existence only. The lifecycle state machine signals readiness.
2. **Recovery is a barrier**: `recover_state().await` in `EntityActor::run()` guarantees that no command is ever read from the mailbox until recovery is complete.
3. **Mailbox is a buffer, not a dispatcher**: The mpsc channel provides ordered storage; the actor's `process_commands()` loop is the dispatcher, and it only starts after the barrier.
4. **Caller sees opaque latency**: The caller sends a command and awaits a response. Whether the entity was RECOVERING or ACTIVE is irrelevant to the caller — the response contains the correct result in either case.
