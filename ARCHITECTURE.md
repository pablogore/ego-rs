# ego-rs Architecture

## Overview

ego-rs is a **hexagonal, actor-oriented, deterministic** backend framework written in Rust. It provides the primitives to build distributed, event-sourced, and replayable backend systems.

## Layer Architecture

```mermaid
flowchart TD
    transport["transport<br/>HTTP, gRPC handlers<br/>Depends on: application, domain"]
    application["application<br/>Command handlers, query handlers, use cases<br/>Depends on: domain"]
    domain["domain<br/>Actor trait, Command, Event, Query, ActorId<br/>Depends on: nothing internal"]
    infrastructure["infrastructure<br/>In-memory adapters, persistence, observability<br/>Depends on: application, domain"]

    transport --> application
    transport --> domain
    application --> domain
    infrastructure --> application
    infrastructure --> domain
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

### Immutability By Default

All domain data structures are immutable values. Changes produce new commands, events, or state instances — never in-place mutation. Event stores are append-only. Read-side projections derive from immutable event streams.

**Authority:** `.speckit/constitution.md` §8 — "Immutability By Default" and "Functional Programming". These rules are defined and enforced by the Constitution, not duplicated here.

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

## Persistent Entity Runtime (CORE-006)

The Persistent Entity Runtime provides an event-sourced, actor-per-entity execution model inspired by Lagom Framework. Each entity is a dedicated Tokio task with exclusive mailbox ownership, deterministic recovery, and single-flight activation.

### Runtime Architecture

```mermaid
flowchart TB
    subgraph Application["Application Layer"]
        PE["PersistentEntity&lt;C,E,S&gt;<br/>handle_command + apply_event"]
    end

    subgraph Runtime["Persistent Entity Runtime (ego-persistent-entity)"]
        ER["EntityRuntime<br/>Top-level lifecycle manager"]
        ERB["EntityRuntimeBuilder<br/>Config: mailbox, concurrency,<br/>passivation, backends"]

        subgraph Registry["Registry &amp; Activation"]
            REG["EntityRegistry<br/>• active: aggregate_id → { mailbox handle,<br/>published lifecycle state, epoch }<br/>• passivated: aggregate_id → version (advisory)"]
        end

        subgraph Actor["Actor Execution"]
            EA["EntityActor<br/>run() loop:<br/>1. recover_state()<br/>2. process_commands()<br/>3. passivate()"]
            MB["Mailbox<br/>bounded mpsc channel<br/>FIFO, configurable capacity"]
            LS["LifecycleStateMachine<br/>Recovering → Active →<br/>Passivating → Passivated<br/>↳ Failed (any state)"]
        end

        subgraph Persistence["Persistence"]
            PF["PersistenceFacade<br/>load_for_recovery()<br/>persist_events()<br/>store_snapshot()"]
            ES["EventStore SPI<br/>(append-only)"]
            SS["SnapshotStore SPI<br/>(cached state)"]
            EP["EventPublisher SPI<br/>(async, best-effort)"]
        end

        subgraph Infra["Infrastructure"]
            SCH["Scheduler<br/>Semaphore-based<br/>concurrency budget"]
        end

        REF["EntityRef&lt;C,E,S&gt;<br/>Per-command sender handle"]
    end

    PE -->|implements| REF
    ER -->|entity_ref lookup-or-spawn| REG
    REG -->|spawns| EA
    EA -->|writes| MB
    MB -->|lifecycle state| LS
    EA -->|load / persist| PF
    PF --> ES
    PF --> SS
    PF --> EP
    EA -->|concurrency slot| SCH
    ERB -.->|builds| ER
    ER -->|entity_ref| REF

    style Application fill:#e1f5fe,stroke:#01579b
    style Runtime fill:#f3e5f5,stroke:#7b1fa2
    style Registry fill:#ede7f6,stroke:#4527a0
    style Actor fill:#fff3e0,stroke:#e65100
    style Persistence fill:#e8f5e9,stroke:#1b5e20
    style Infra fill:#fce4ec,stroke:#880e4f
```

### Activation Ordering Model (Formal)

The activation ordering model defines the precise timing of mutex scope, mailbox creation, registry visibility, and recovery barrier — resolving all ambiguity between existence and readiness.

```mermaid
sequenceDiagram
    participant C as Caller (EntityRef)
    participant R as EntityRegistry
    participant T as Actor Task
    participant M as Mailbox (BoundedMailbox)
    participant P as EventStore

    Note over C,P: ACTIVATION — Single-Flight Lock Held
    C->>R: lookup_or_insert(aggregate_id)
    R-->>C: no live entry — I'm the spawner
    C->>M: BoundedMailbox::new(capacity) created
    Note right of M: Mailbox exists<br/>before spawn
    C->>R: insert entry { mailbox, state=Recovering, epoch }
    Note right of R: Entry EXISTS but is NOT<br/>counted active — existence ≠ active count
    C->>R: lock released
    C->>T: tokio::spawn(actor.run()) — strictly after lock release
    Note right of T: Spawning after release avoids the<br/>self-deadlock a panic-during-spawn<br/>would otherwise cause
    C->>M: send(first_command)
    Note right of M: Commands queue here<br/>during recovery

    Note over C,P: RECOVERY — Actor Context
    T->>T: run() begins (state=Recovering)
    T->>P: load_for_recovery()
    P-->>T: (snapshot, events)
    T->>T: replay events in order
    Note right of T: RECOVERY BARRIER<br/>No commands processed<br/>until recovery completes
    T->>T: transition(Active) — actor publishes via watch::Sender
    Note right of R: Now counted by active_count() —<br/>the actor is the sole writer of this state

    Note over C,P: COMMAND PROCESSING
    T->>M: recv() → first command
    T->>T: execute_command()
    T->>P: persist_events()
    P-->>T: new_version
    T->>M: recv() → next command...

    Note over C,P: PASSIVATION / TEARDOWN
    T->>T: (idle timeout) passivate() begins
    T->>T: drain remaining commands, store final snapshot
    T->>T: task ends — on ANY exit (normal, panic, cancellation)
    Note right of T: TeardownGuard::drop() fires —<br/>the one and only teardown path
    T->>M: close_and_drain() — terminally answers anything still queued
    T->>R: deactivate_if_mine(epoch) — remove entry
    T->>R: publish terminal state (backstop only if not already published)
```

### Five-State Lifecycle Machine

```mermaid
stateDiagram-v2
    [*] --> Recovering: command arrives

    Recovering --> Active: recovery complete
    Active --> Passivating: idle timeout / passivation
    Passivating --> Passivated: final snapshot stored
    Passivated --> Recovering: command reactivates

    Recovering --> Failed: irrecoverable error
    Active --> Failed: irrecoverable error
    Passivating --> Failed: irrecoverable error
    Passivated --> Failed: irrecoverable error
    Failed --> Recovering: on-demand recovery or restart
```

| State | In Registry (map entry)? | Counted by `active_count()`? | Commands |
|-------|---------------------------|-------------------------------|----------|
| `Recovering` | Yes | No | Buffered in mailbox, not executed |
| `Active` | Yes | Yes | Executed FIFO |
| `Passivating` | Yes (draining) | Yes | Existing drained, new rejected |
| `Passivated` | No (removed by `TeardownGuard`) | No | Triggers activation → Recovering |
| `Failed` | No (removed by `TeardownGuard`) | No | Retry triggers new activation |

### Key Design Invariants

| Invariant | Enforced By |
|-----------|-------------|
| Exactly one actor per entity triple | Registry-map single-flight — `lookup_or_insert()`'s one lock acquisition, not a separate activation mutex (FR-001) |
| Single source of truth for "active" | The actor is the sole writer of its lifecycle state; the registry only observes it via `watch::Receiver` |
| No command processed before recovery | `recover_state().await` completes before `process_commands()` (FR-002) |
| Mailbox exists before spawn | `BoundedMailbox::new()` created before `tokio::spawn` (FR-003) |
| Lock held only for map mutation, NOT spawn/recovery | Lock released before `tokio::spawn`; the erased mailbox's downcast also happens after release (FR-004) |
| FIFO command ordering per entity | Bounded mailbox, ordered delivery (FR-005) |
| Observable state is always consistent | Recovery barrier prevents partial-state observation (FR-006) |
| Passivation is irreversible | PASSIVATING → ACTIVE forbidden (FR-008) |
| Events never rolled back | Append-only event store (FR-026) |
| Snapshots are pure optimization | Event stream always authoritative (FR-012) |
| CAS forbidden | `parking_lot::Mutex` for the registry map, not atomic CAS loops (§5 constitution) |

**Reference**: Full activation ordering specification at `openspec/changes/archive/2026-06-22-persistent-entity-runtime/activation-ordering/`.

### Crate Layout

```
crates/persistent-entity/
├── Cargo.toml
└── src/
    ├── lib.rs                # Crate root, re-exports
    ├── runtime.rs            # EntityRuntime<E>
    ├── builder.rs            # EntityRuntimeBuilder<E>
    ├── entity_ref.rs         # EntityRef<C,E,S>
    ├── actor.rs              # EntityActor (recover → process → passivate)
    ├── registry.rs           # EntityRegistry (single-flight routing map + advisory passivated map)
    ├── mailbox.rs            # BoundedMailbox<T>, CommandEnvelope<C>
    ├── persistent_entity.rs  # PersistentEntity trait
    ├── lifecycle.rs          # LifecycleStateMachine
    ├── recovery.rs           # StateRecovery trait
    ├── persistence.rs        # PersistenceFacade<E>
    ├── publisher.rs          # EventPublisher<E>
    ├── snapshot.rs           # SnapshotStrategy
    ├── command_context.rs    # CommandContext
    ├── scheduler.rs          # Scheduler (semaphore)
    ├── error.rs              # EntityError
    └── testing.rs            # In-memory backends
```

## Implementation Roadmap

| ID | Name | Status |
|----|------|--------|
| CORE-001 | Deterministic Runtime Slice | In progress |
| CORE-002 | Actor Primitive (domain) | Spec complete, not implemented |
| CORE-003 | Runtime Actor Execution | Pending |
| CORE-004 | Persistence SPI | Pending |
| CORE-005 | Observability SPI | Pending |
| CORE-006 | Persistent Entity Runtime | **Design complete** (spec + activation ordering) |
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