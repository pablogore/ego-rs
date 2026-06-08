# Research: Persistent Entity Runtime

**Feature**: `006-persistent-entity-runtime`
**Date**: 2026-06-07
**Status**: Complete

## Overview

Consolidated findings for all technical unknowns and design decisions identified during planning for the CORE-006 Persistent Entity Runtime implementation.

---

## 1. Performance Targets

**Decision**: No hard latency/throughput targets at this stage.

**Rationale**: The spec defines correctness semantics (deterministic replay, single-writer, at-most-once actor) but does not mandate specific performance SLAs. The entity runtime is infrastructure for application developers, not a user-facing service with SLOs. Performance optimization is deferred to post-implementation profiling.

**Acceptable baseline**:
- Inline command on cached (ACTIVE) entity: sub-millisecond excluding persistence I/O
- Recovery from snapshot: dominated by snapshot deserialization + event replay count
- Passivation drain: bounded by mailbox depth × command execution time

---

## 2. Concurrency Budget Range & Defaults

**Decision**: Configurable with a reasonable default of 10,000 concurrent entity tasks.

**Rationale**: The spec defines the budget as a scheduling throttle but not the default value. 10,000 aligns with Tokio's typical task capacity before scheduler overhead becomes measurable. Configurable via `EntityRuntimeBuilder` so deployments can tune per workload.

---

## 3. Mailbox Capacity Default

**Decision**: Default mailbox capacity of 1,000 commands per entity.

**Rationale**: Balances burst tolerance (a single entity could receive 1,000 rapid-fire commands) with bounded memory. Configurable via `EntityRuntimeBuilder`. The 1,000 default matches common actor system mailbox sizes (Akka default is also 1,000).

---

## 4. Passivation Timeout Default

**Decision**: Default inactivity timeout of 5 minutes.

**Rationale**: Common actor system default (Akka uses 2 minutes for cluster, 5 for local). 5 minutes keeps entities resident long enough to avoid thrashing on intermittent workloads while freeing memory for truly idle entities. Configurable.

---

## 5. Snapshot Frequency Default

**Decision**: Default snapshot every 100 events.

**Rationale**: Balances recovery performance (max 100 events replayed after snapshot) with snapshot storage overhead. Configurable via `SnapshotStrategy`.

---

## 6. CAS Prohibition — Guard Mechanism

**Decision**: Use per-entity `Mutex` for reactivation guard, NOT CAS.

**Rationale**: Constitution §5 explicitly forbids "CAS loops (AtomicUsize, compare_exchange) anywhere in the system." The risk document's CAS option is superseded by constitutional constraints. The per-entity `Mutex` approach:
- Simple: `HashMap<EntityTriple, Arc<Mutex<()>>>` in the passivation registry
- Tokio-compatible: `tokio::sync::Mutex` is async-safe, holds across `.await` points
- Single-flight semantics naturally emerge: first acquirer spawns task, subsequent acquirers observe updated state
- Memory: one `Mutex` per unique entity triple attempting activation (transient, released after task spawn)

**Alternatives considered**: `std::sync::Mutex` (blocks thread — inappropriate for async), CAS (forbidden by constitution), channel-based ownership (more complex, same safety).

---

## 7. Crate Location & Naming

**Decision**: New crate `crates/persistent-entity/` with package name `ego-persistent-entity`.

**Rationale**: Follows existing crate naming convention (`ego-domain`, `ego-runtime`, `ego-persistence`, etc.). Placed at the same layer as `ego-runtime` — consumes `ego-domain` SPIs, provides concrete implementation.

---

## 8. Event Publisher SPI Implementation

**Decision**: Initial implementation uses an in-memory channel-based publisher with pluggable backpressure.

**Rationale**: The spec defines the EventPublisher SPI as a trait. The first implementation is an in-memory channel that buffers published events for test observation. Production implementations (outbox, Kafka, NATS) are deferred.

---

## 9. Existing Crate Reuse

**Decision**: Reuse `ego-domain::persistence::event_store::EventStore`, `ego-domain::persistence::snapshot::Snapshot`, and `ego-domain::persistence::repository::Repository` traits directly. The new crate does NOT define new persistence SPIs — it implements against the existing domain contracts.

**Rationale**: Avoids duplicating persistence abstractions. The existing traits are the correct level of abstraction for the entity runtime.

---

## 10. Public API Shape

**Decision**: Three-tier public API:
1. **`PersistentEntity<C, E, S>`** trait — user implements for domain entities (command handler, event applier, initial state)
2. **`EntityRef<C, E, S>`** — command-sending handle, created by the runtime per command invocation
3. **`EntityRuntime`** — lifecycle manager, built via `EntityRuntimeBuilder`, registered with application

All three expose only domain types — no Tokio, no Postgres, no implementation types.
