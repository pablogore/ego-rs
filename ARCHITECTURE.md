# ego-rs Architecture

## Overview

ego-rs is a **hexagonal, actor-oriented, deterministic** backend framework written in Rust. It provides the primitives to build distributed, event-sourced, and replayable backend systems.

## Layer Architecture

```
┌──────────────────────────────────────────────┐
│ transport                                    │
│ HTTP, gRPC handlers → delegates to app layer │
│ Depends on: application, domain              │
└──────────────┬───────────────────────────────┘
               │
┌──────────────▼───────────────────────────────┐
│ application                                  │
│ Command handlers, query handlers, use cases  │
│ Depends on: domain                           │
└──────────────┬───────────────────────────────┘
               │
┌──────────────▼───────────────────────────────┐
│ domain                                       │
│ Actor trait, Command, Event, Query, ActorId  │
│ Depends on: nothing internal                 │
└──────────────────────────────────────────────┘
               ▲
┌──────────────┴───────────────────────────────┐
│ infrastructure                               │
│ In-memory adapters, persistence, observability│
│ Depends on: application, domain              │
└──────────────────────────────────────────────┘
```

## Dependency Rules (enforced by `layers.toml` + `scripts/verify-layers.sh`)

| Layer | May depend on |
|-------|--------------|
| `ego-domain` | nothing internal |
| `ego-application` | `ego-domain` |
| `ego-infrastructure` | `ego-application`, `ego-domain` |
| `ego-transport` | `ego-application`, `ego-domain` |

Forbidden:
- `domain → application|infrastructure|transport`
- `application → infrastructure|transport`
- `infrastructure → transport`
- `transport → infrastructure`

## Core Concepts

### Actor Model (CORE-002)

The actor is the central behavioral abstraction. An actor:
- Declares its message type (`type Message`)
- Has a location-transparent `ActorId`
- Processes one message at a time (enforced by runtime)
- Participates in a supervision hierarchy

```rust
pub trait Actor {
    type Message;
}
```

### Runtime Execution (CORE-003)

The runtime layer owns:
- `ActorSystem` — spawn/stop actors, route messages
- `Mailbox<M>` — bounded, FIFO, non-blocking
- `ActorRef<M>` — sendable handle
- `RuntimeSupervisor` — restart/stop/escalate

### CQRS + Event Sourcing

- **Commands** (`Command` trait) — mutate state
- **Events** (`DomainEvent` trait) — record state transitions (append-only)
- **Queries** (`Query` trait) — read state without mutation

### Determinism Axiom

> Given identical inputs, runtime state, logical time, and context, the observable outcome MUST be identical.

All framework primitives are deterministic by default. Randomness, wall-clock time, and external I/O are injected through explicit ports — never implicit behavior.

### Fail-Closed

Ambiguous states produce rejection, never silent continuation. Unknown inputs, undefined transitions, and partial failures are explicit errors.

## Crate Layout

```
ego-rs/
├── crates/
│   ├── domain/        # ego-domain: core contracts
│   ├── application/   # ego-application: handlers
│   ├── infrastructure/ # ego-infrastructure: adapters
│   ├── transport/     # ego-transport: HTTP/gRPC
│   └── runtime/       # ego-runtime: actor execution (CORE-003, not yet created)
├── core/
│   └── runtime-slice/ # runtime-slice: deterministic execution types
├── contracts/         # Protobuf contracts (Buf)
├── openspec/          # Specs, changes, proposals
└── scripts/           # Verification scripts
```

## Implementation Roadmap

| ID | Name | Status |
|----|------|--------|
| CORE-001 | Deterministic Runtime Slice | In progress |
| CORE-002 | Actor Primitive (domain) | Spec complete, not implemented |
| CORE-003 | Runtime Actor Execution | Pending |
| CORE-004 | Persistence SPI | Pending |
| CORE-005 | Observability SPI | Pending |
| CORE-007 | Cluster Model | Archived (deferred, post-MVP) |
| CORE-010 | SDK + Developer API | Deferred |
| CORE-011 | Examples | Deferred |

## Key Principles

1. **Framework-first** — build the framework before modeling runtime governance
2. **Minimal primitives** — one concept, one trait, one responsibility
3. **Implementation-driven** — every spec ends in runnable code
4. **Archiveable specs** — implement → archive → next
5. **No bureaucracy** — no governance engines, no policy DSLs, no enterprise abstractions
6. **Domain owns contracts, runtime owns execution** — clean hexagonal boundary
7. **Tokio-first, never Tokio-bound** — contracts are runtime-neutral