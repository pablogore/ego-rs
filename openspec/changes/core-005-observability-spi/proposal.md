## Why

ego-rs is a backend-platform / actor-platform framework that treats observability as a first-class platform concern. Currently, the platform lacks a canonical, runtime-neutral, transport-neutral observability contract. Without this, observable semantics are ad-hoc, vendor coupling leaks into the core, replay safety is undefined, and cluster-aware observability is impossible. FOUNDATION-007 defines the constitutional Observability SPI — a deterministic, replay-safe, hexagonal contract that observes platform behavior without owning execution.

## What Changes

- Introduces the canonical Observability SPI for ego-rs
- Defines observable semantics: execution visibility, actor lifecycle visibility, message visibility, failure visibility, replay visibility, placement visibility, ownership visibility, persistence visibility, locality visibility, restoration visibility
- Defines a canonical event model with semantic event categories expressed through canonical observability channels (trace, metric, log) — not vendor telemetry representations
- Defines deterministic correlation semantics independent of trace vendor IDs, runtime handles, transport identifiers, or network topology
- Defines replay-safe observability: replay MUST NOT create semantic ambiguity; identical inputs produce identical observable semantics
- Defines cluster-aware observability that observes placement, ownership, partition transitions, membership, and locality but does NOT own cluster behavior
- Establishes the Deterministic Observability Axiom: given identical inputs, logical time, ownership state, placement state, replay state, and execution semantics, observable semantics MUST be identical
- Defines a capability model with mandatory, optional, and forbidden capabilities
- Establishes governance: forbidden patterns, dependency analysis, determinism audit, capability inflation protection, replay-safety verification, mock-only capture of observable semantics
- Defines hexagonal boundaries: Observability SPI depends ONLY on Canonical Contracts, Runtime Contract, Actor Contract, Persistence SPI, Cluster Contract — never vice versa
- Defines a testing contract: mock-only capture of observable semantics, deterministic tests, replay reproducibility, no telemetry infrastructure, no external services, 95%+ semantic surface coverage
- Consumes observable semantics from Runtime, Actor, Persistence, and Cluster layers without creating reverse dependencies

## Capabilities

### New Capabilities

- `observability-spi`: Canonical Observability SPI contract — defines observable semantics, event model, correlation model, replay-safe observability, cluster-aware observability, deterministic guarantees, capability model, governance, hexagonal boundaries, and testing contract for the ego-rs backend platform

### Modified Capabilities

<!-- No existing capabilities have requirement changes — FOUNDATION-007 is a new constitutional foundation that consumes but does not modify prior constitutional surfaces -->

## Impact

- Canonical Contracts (FOUNDATION-002): Observable semantics implicitly extend what canonical contracts define as observable; no contract surface change
- Runtime Contract (FOUNDATION-003): Runtime exposes execution visibility semantics; Observability SPI consumes these without modifying the Runtime contract
- Actor Contract (FOUNDATION-004): Actor Model exposes lifecycle semantics; Observability SPI consumes these without modifying the Actor contract
- Persistence SPI (FOUNDATION-005): Persistence exposes replay and restoration visibility; Observability SPI consumes these without modifying the Persistence contract
- Cluster Contract (FOUNDATION-006): Cluster Model exposes placement, ownership, and partition semantics; Observability SPI consumes these without modifying the Cluster contract
- No reverse dependencies created — Observability SPI MUST NOT own runtime execution, runtime scheduling, persistence lifecycle, cluster coordination, transport, telemetry backend, exporter lifecycle, or vendor SDK integration
