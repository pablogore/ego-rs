# FOUNDATION-004: Actor Model

## Why

ego-rs is a backend platform framework. Its constitutional core MUST remain runtime-neutral and implementation-independent while enabling rich platform capabilities — CQRS, Event Sourcing, workflow orchestration, distributed execution, service composition, and deterministic replay — without coupling to any concrete actor runtime. The actor model is a first-class platform capability for building distributed, message-driven, deterministic backend systems.

Currently, ego-rs has no canonical actor model contract. Without a formal actor abstraction, domain and application logic risk implicit coupling to specific actor framework semantics, execution engines, or messaging infrastructure. The actor model is a core architectural pattern for the platform, but it must remain a pluggable behavioral abstraction — not a framework binding. This spec defines the constitutional Actor Model of ego-rs so that actor behavior, communication, supervision, lifecycle, identity, and determinism are governed by contract, not by any concrete implementation.

## What Changes

- Define the canonical Actor Model as a constitutional and runtime-agnostic abstraction for ego-rs
- Define what an actor is: a behavioral abstraction with defined responsibilities and non-responsibilities
- Define actor identity and addressing: logical actor references with location transparency by contract
- Define the communication model: actor-to-actor message exchange semantics, ordering guarantees, delivery expectations, isolation semantics, visibility rules, and determinism guarantees
- Define the message model: immutable message expectations, canonical message boundaries, serialization neutrality, message ownership, invalid message handling
- Define the actor lifecycle: states (Created, Starting, Running, Restarting, Stopped, Failed) with deterministic transitions and fail-closed semantics
- Define the supervision model: parent-child boundaries, failure propagation, escalation semantics, restart boundaries, supervision invariants
- Define concurrency semantics: actor isolation, single logical execution boundary, ordering expectations, visibility guarantees, determinism expectations — all runtime-neutral
- Define the actor capability model: mandatory capabilities (receive work, process message, state transition, supervision participation, identity resolution), optional capabilities (delayed delivery, lifecycle observation, deterministic replay participation), and forbidden capabilities (persistence, transport ownership, networking, business orchestration, observability infrastructure, runtime primitive leakage)
- Define the failure model: fail-closed on all ambiguous states, invalid message behavior, actor failure propagation, supervision failure visibility, deterministic error behavior
- Define the Determinism Axiom as a constitutional invariant: identical actor state + identical message sequence + identical logical time + identical runtime capabilities + identical context SHALL produce identical observable outcome
- Define actor non-responsibilities: what an actor MUST NOT do
- Define the testing contract: deterministic tests, mock-only tests, no real runtime requirement, replayability, reproducibility, 95%+ coverage, no infrastructure dependencies
- Define hexagonal boundaries: Core depends only on Actor Contract, Actor Contract depends only on Runtime Contract (FOUNDATION-003)
- Define governance: constitutional invariants, forbidden patterns, violation criteria, capability inflation protection

**BREAKING**: Establishes the Actor Model as a constitutional layer. Any existing code that directly depends on a concrete actor framework or runtime implementation must be adapted to comply.

## Capabilities

### New Capabilities

- `actor-model`: Canonical Actor Model for ego-rs including actor definition, identity and addressing, communication semantics, message model, lifecycle, supervision, concurrency semantics, capability model, failure model, determinism axiom, testing contract, hexagonal boundaries, and governance.

### Modified Capabilities

- `runtime-abstraction` (FOUNDATION-003): FOUNDATION-004 consumes Runtime Contract capability ports. The Actor Contract depends on runtime capabilities (execution, time, context) through FOUNDATION-003 without modifying its constitutional surface.
- `project-constitution`: Updated to include actor-model constitutional invariants: determinism axiom, location transparency, fail-closed supervision, and runtime-independent actor execution.

## Impact

- Introduces the Actor Model as a new constitutional layer between core domain/application and the runtime abstraction
- Core domain and application code must depend only on actor contracts, never on concrete actor runtime implementations
- The Actor Contract must depend only on the Runtime Contract (FOUNDATION-003), never on concrete runtime adapters
- All actor execution mechanics belong to runtime adapters — the core defines only semantics and contracts
- Future runtime implementations become possible through runtime adapters without core changes
- CI must verify compliance: no concrete actor framework dependencies in core code, no runtime implementation leakage into actor contracts
- Location transparency ensures actors can be local, remote, embedded, simulated, or distributed without core awareness
- FOUNDATION-008 Examples Constitution SHALL validate FOUNDATION-004 invariants through canonical examples
