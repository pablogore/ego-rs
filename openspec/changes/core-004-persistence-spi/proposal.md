## Why

Foundation-005 defines the canonical Persistence SPI of ego-rs as a constitutional, runtime-neutral, backend-platform persistence abstraction. Persistence is a first-class platform capability. Without a constitutional persistence contract, the platform cannot guarantee deterministic replay, actor state restoration, durable event persistence, or fail-closed recovery — capabilities required for CQRS, Event Sourcing, workflows, and saga orchestration. This foundation must be established before any storage adapter or persistence implementation is created.

## What Changes

- Introduce constitutional Persistence SPI as a semantic persistence contract — not a database abstraction, ORM, repository pattern, or storage engine
- Define core Persistence SPI traits (EventStore, SnapshotStore) with append and read semantics, optimistic concurrency via versioning
- Define replay semantics: deterministic state reconstruction from event streams, snapshot + event catch-up
- Define fail-closed behavior: version conflicts, ambiguous storage outcomes, partial writes all produce explicit errors
- Define testing contract: in-memory adapters only, no real infrastructure, 95%+ coverage
- Define ownership boundary: persistence does not own domain lifecycle semantics or execution

## Capabilities

### New Capabilities
- `persistence-spi`: Canonical Persistence SPI encompassing core persistence port traits (EventStore, SnapshotStore), replay semantics, fail-closed behavior, ownership boundary, and testing contract. This is a single coherent capability — not decomposed into sub-capabilities — because the Persistence SPI is a unified constitutional surface whose sections are mutually dependent and must be validated as a whole.

### Modified Capabilities
<!-- No existing capabilities are modified. This is a new foundation. -->

## Impact

- New constitutional spec under `openspec/specs/persistence-spi/spec.md`
- Establishes semantic contract for all future persistence adapter implementations
- Consumes FOUNDATION-003 Runtime Abstraction and FOUNDATION-004 Actor Model without modifying them
- FOUNDATION-004 remains frozen; Foundation-005 builds on it
- Enables future platform capabilities: CQRS, Event Sourcing, durable actors, snapshots, replay, workflows, saga/process orchestration, service composition
- All conforming persistence realizations SHALL satisfy the Persistence SPI
