# Design: Runtime Actor Execution

## Context

CORE-002 defines the domain contract: `Actor` trait (`type Message`), `ActorId`, semantic lifecycle states, and supervision semantics. CORE-003 owns all runtime mechanics. These were previously split into separate specs (mailbox, dispatch, supervision) — they are now unified as a single runtime execution concern.

## Goals / Non-Goals

**Goals:**
- `ActorSystem` spawns actors by `ActorId`, routes messages, manages lifecycle
- `Mailbox<Msg>` — bounded capacity, FIFO ordering, non-blocking send
- Sequential processing — one message at a time per actor
- `RuntimeSupervisor` executes supervision strategies (restart, stop, escalate)
- `ActorRef<Msg>` — sendable handle, implements Clone
- Deterministic enforcement of CORE-002's communication guarantees

**Non-Goals:**
- Persistence or event sourcing (CORE-004)
- Transport or remote messaging (CORE-007, deferred)
- Observability (CORE-005)
- Cluster or distribution (CORE-007, deferred)
- Effect systems, fan-out, reply patterns

## Decisions

### Decision 1: Runtime mechanics unified in one spec

Mailbox, dispatch, sequential execution, and supervision are runtime mechanics of the same concern: actor execution. Separating them into independent specs encouraged premature decisions about each without the context of the others.

### Decision 2: Domain defines semantics, runtime enforces them

CORE-002 says "messages SHALL be FIFO" — CORE-003 implements the FIFO mailbox. CORE-002 says "one message at a time" — CORE-003 enforces sequential processing. The runtime implements what the domain contracts declare.

### Decision 3: Tokio-first, never Tokio-bound

The first `ActorSystem` uses Tokio. The SPI remains runtime-neutral: no Tokio types in the actor contract (CORE-002), no Tokio assumptions in the execution semantics.

### Decision 4: In-memory only, no persistence

Mailbox and state are in-memory. Persistence is a separate concern (CORE-004). This keeps the runtime minimal — CORE-003 doesn't need a database or event store.

## Architecture

```
┌──────────────────────────────────────────┐
│ Domain (CORE-002)                        │
│  Actor trait, ActorId, lifecycle states  │
└──────────────┬───────────────────────────┘
               │ implements
               ▼
┌──────────────────────────────────────────┐
│ Runtime (CORE-003)                       │
│                                          │
│  ActorSystem                             │
│  ├─ spawn(actor) → ActorRef              │
│  ├─ route(message, ActorId)              │
│  └─ manage lifecycle                     │
│                                          │
│  Mailbox<Msg>                            │
│  ├─ bounded capacity                     │
│  ├─ FIFO ordering                        │
│  └─ non-blocking send                    │
│                                          │
│  RuntimeSupervisor                       │
│  ├─ parent-child hierarchy               │
│  ├─ restart/stop/escalate execution      │
│  └─ failure detection + propagation      │
└──────────────────────────────────────────┘
```

## Risks / Trade-offs

- **[Tokio coupling]** First implementation uses Tokio. Mitigation: CORE-002 contract is runtime-neutral. Tokio types never leak into domain.
- **[Supervision complexity]** Runtime supervisor must detect failures and schedule restarts. Mitigation: start with simple restart-only, add stop/escalate incrementally.
- **[Mailbox backpressure]** Bounded mailbox rejects on full — sender must handle rejection. Mitigation: this is explicit, observable, fail-closed behavior.