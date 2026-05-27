## Why

Foundation-005 defines the canonical Persistence SPI of ego-rs as a constitutional, runtime-neutral, backend-platform persistence abstraction. Persistence is a first-class platform capability. Without a constitutional persistence contract, the platform cannot guarantee deterministic replay, actor state restoration, durable event persistence, or fail-closed recovery — capabilities required for CQRS, Event Sourcing, workflows, and saga orchestration. This foundation must be established before any storage adapter or persistence implementation is created.

## What Changes

- Introduce constitutional Persistence SPI as a semantic persistence contract — not a database abstraction, ORM, repository pattern, or storage engine
- Define durability semantics: explicit acknowledgment, visibility boundaries, deterministic guarantees
- Define state persistence semantics: actor/application state persistence, restoration, lifecycle relationship
- Define event persistence semantics: append-only guarantees, ordering, replayability, idempotency
- Define snapshot semantics: boundaries, restoration, consistency, lifecycle
- Define replay persistence semantics: deterministic replay, restoration, consistency, fail-closed behavior
- Define persistence capability model: mandatory, optional, and forbidden capabilities
- Define persistence failure model: fail-closed semantics for durability ambiguity, partial writes, restoration ambiguity
- Define persistence lifecycle: Requested → Persisting → Persisted → Restoring → Restored → Failed with invariants and transitions
- Define Deterministic Persistence Axiom: identical inputs produce identical observable persistence outcomes
- Define hexagonal boundaries: Persistence Contract depends only on Runtime Contract and Canonical Contracts; persistence remains transversal and actor-independent
- Define unified persistence contract semantics: single coherent contract independent of storage realization
- Define tenant isolation semantics: deterministic tenant boundaries for single and multi-tenant configurations
- Define persistence evolution semantics: self-contained, reproducible, deterministic evolution without migration tooling coupling
- Define persistence versioning semantics: version identifiers for state schemas, event schemas, and snapshot formats; version-aware persistence and restoration; fail-closed version mismatch handling; backward/forward compatibility semantics
- Define ownership boundary: persistence does not own domain lifecycle semantics
- Define governance: constitutional invariants, forbidden patterns, capability inflation protection, vendor neutrality, determinism enforcement
- Define testing contract: deterministic tests, mock-only, replay reproducibility, no infrastructure dependencies, 95%+ coverage
- Link to FOUNDATION-008 for canonical constitutional validation examples

## Capabilities

### New Capabilities
- `persistence-spi`: Canonical Persistence SPI encompassing durability, state persistence, event persistence, snapshot, replay, versioning semantics, capability model, failure model, lifecycle, deterministic axiom, hexagonal boundaries, governance, and testing contract. This is a single coherent capability — not decomposed into sub-capabilities — because the Persistence SPI is a unified constitutional surface whose sections are mutually dependent and must be validated as a whole.

### Modified Capabilities
<!-- No existing capabilities are modified. This is a new foundation. -->

## Impact

- New constitutional spec under `openspec/specs/persistence-spi/spec.md`
- Establishes semantic contract for all future persistence adapter implementations
- Consumes FOUNDATION-003 Runtime Abstraction and FOUNDATION-004 Actor Model without modifying them
- FOUNDATION-004 remains frozen; Foundation-005 builds on it
- Enables future platform capabilities: CQRS, Event Sourcing, durable actors, snapshots, replay, workflows, saga/process orchestration, service composition
- All conforming persistence realizations SHALL satisfy the Persistence SPI
