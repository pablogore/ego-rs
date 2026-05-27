## Context

Foundation-005 defines the Persistence SPI of ego-rs as a constitutional contract. This is not a storage engine design, an ORM, a repository framework, or a database adapter specification. It is the semantic persistence contract that all future storage realizations must satisfy.

The architecture builds on:
- **FOUNDATION-001 Architecture Constitution**: hexagonal architecture, dependency inversion, governance
- **FOUNDATION-002 Canonical Contracts**: determinism, serialization neutrality, capability contracts
- **FOUNDATION-003 Runtime Abstraction & Execution Model**: runtime neutrality, capability-based execution
- **FOUNDATION-004 Actor Model**: actor lifecycle, deterministic replay, actor restoration

Constraint: FOUNDATION-004 is frozen. Foundation-005 must consume rather than modify the actor model constitutional surface.

The proposal establishes a single coherent capability `persistence-spi` encompassing all persistence semantics. This design preserves that unity — the Persistence SPI must be a single constitutional surface because durability semantics, state persistence, event persistence, snapshot semantics, replay semantics, tenant isolation, persistence evolution, the ownership boundary, the failure model, and the capability model are interdependent and must be validated as a coherent whole.

## Goals / Non-Goals

**Goals:**

- Define a constitutional Persistence SPI that is runtime-neutral, storage-neutral, deterministic-first, fail-closed, and hexagonal
- Define durability semantics with explicit acknowledgment, deterministic guarantees, unambiguous visibility boundaries, and support for multiple durability realizations (durable, in-memory, deterministic replay, ephemeral)
- Define state persistence semantics supporting actor/application state persistence and restoration without coupling to storage representation
- Define event persistence semantics supporting durable recording, ordering, replayability, and idempotency expectations — enabling future CQRS/ES without coupling to EventStore implementations or redefining Persistence SPI itself
- Define snapshot semantics with clear boundaries, consistency expectations, and lifecycle
- Define replay persistence semantics preserving deterministic observable outcomes
- Define a Persistence Capability Model distinguishing mandatory, optional, and forbidden capabilities
- Define a fail-closed Persistence Failure Model handling durability ambiguity, partial writes, and restoration ambiguity
- Define persistence lifecycle states and transitions with invariants
- Define the Deterministic Persistence Axiom
- Define hexagonal boundaries: Persistence Contract depends only on Runtime Contract and Canonical Contracts; persistence remains valid for actors, workflows, services, process orchestration, and non-actor execution
- Define unified persistence contract semantics: single coherent contract across heterogeneous storage realizations
- Define tenant isolation semantics: single-tenant and multi-tenant through constitutional tenant isolation boundaries
- Define persistence evolution semantics: self-contained, reproducible, deterministic evolution
- Define persistence versioning semantics: version identifiers, version-aware persistence and restoration, fail-closed version mismatch, compatibility semantics, deterministic versioning
- Define ownership boundary: persistence does not own domain lifecycle semantics
- Define governance protecting constitutional invariants, vendor neutrality, and determinism
- Define the testing contract
- Link to FOUNDATION-008 for canonical validation

**Non-Goals:**

- NOT a database abstraction, ORM, repository pattern, storage engine, SQL abstraction, or DB framework
- NOT a storage adapter specification or implementation
- NOT a serialization format specification
- NOT a migration system or schema definition
- NOT a versioning tool or schema definition language
- NOT a caching strategy
- NOT a transport specification
- NOT a CQRS or Event Store implementation
- NOT a workflow or saga orchestration engine
- NOT a repository interface or language-level trait definition in Rust
- NOT a runtime adapter
- NOT a migration engine or schema tooling
- NOT a tenant isolation implementation strategy (schema-per-tenant, database-per-tenant)

## Decisions

**Decision 1: Single coherent spec vs. decomposed sub-specs**

The Persistence SPI is authored as a single spec document rather than decomposed into sub-capabilities (durability.md, state.md, event.md, snapshot.md, etc.).

Rationale: The persistence semantics are interdependent. The Deterministic Persistence Axiom constrains all subsections. The failure model applies uniformly. The capability model governs across all dimensions. Decomposition would create cross-document dependency cycles and redundancy. A single document enforces coherence.

Alternatives considered:
- Split into per-concept specs: Rejected because it introduces circular dependencies (snapshot semantics depend on state and event semantics; failure model governs all) and forces readers to cross-reference constantly.
- Split into contract vs. lifecycle vs. governance: Rejected because governance and lifecycle semantics are integral to each persistence concept.

**Decision 2: Semantic contract only — no language-level interfaces**

The SPI defines semantic contracts, not Rust traits, TypeScript interfaces, or any language binding.

Rationale: Constitutional specs define WHAT the system guarantees, not HOW it enforces those guarantees. Language-level interfaces are implementation concerns that belong in adapter crates.

Alternatives considered:
- Including Rust trait definitions: Rejected — would couple the constitution to a specific language and runtime, violating FOUNDATION-001's language neutrality principle.
- Including pseudo-code interfaces: Rejected — pseudo-code is neither normative nor implementable; it would create ambiguity about whether it is binding or illustrative.

**Decision 3: Persistence Contract depends only on Runtime Contract and Canonical Contracts; Actor Contract consumes Persistence Contract**

Persistence is a transversal platform capability, not subordinated to any single contract. Following FOUNDATION-001 hexagonal architecture, Persistence Contract and Actor Contract are peers that both depend on Runtime Contract and Canonical Contracts.

Persistence Contract depends on:
- FOUNDATION-002 Canonical Contracts (determinism, serialization neutrality, capability contracts)
- FOUNDATION-003 Runtime Contract (runtime abstraction, execution guarantees)

Persistence Contract does NOT depend on:
- FOUNDATION-004 Actor Contract (persistence is consumed by actors but not architecturally dependent on the actor model)
- Any storage adapter or platform implementation

Actor Contract MAY consume Persistence Contract — it does not define it.

Rationale: Persistence semantics must be valid independent of whether an actor runtime is present. Persistence supports actors, workflows, services, process orchestration, and non-actor execution. The dependency direction is: Actor Contract → Persistence Contract, not the inverse.

**Decision 4: Fail-closed as the default persistence failure mode**

All persistence operations SHALL fail closed on ambiguity. Silent data loss, silent durability failure, and silent state corruption are forbidden by constitutional invariant.

Rationale: A backend platform framework must never silently lose data. Fail-closed behavior at the constitutional level ensures every adapter implementation inherits this requirement.

Alternatives considered:
- Fail-open with configurable tolerance: Rejected — silent durability failure is never acceptable in a platform framework.
- Best-effort semantics: Rejected — violates the constitutional non-negotiable of data integrity.

**Decision 5: Mandatory capabilities baseline is minimal but includes transversal concerns**

Mandatory capabilities include deterministic durability visibility, tenant boundary awareness, and deterministic persistence evolution compatibility alongside core state and event persistence. Snapshots, replay optimization, multi-tenant isolation optimization, and tenant portability are optional. Forbidden capabilities include transport ownership, orchestration ownership, runtime scheduling ownership, vendor-specific migration semantics, manual out-of-band persistence assumptions, ORM semantics, and tenant implementation leakage.

Rationale: Tenant awareness and evolution compatibility are mandatory because persistence is a transversal platform capability — every persistence realization must support tenant-scoped boundaries and self-contained evolution regardless of storage backend. ORM and repository semantics are explicitly forbidden because persistence is a semantic durability contract, not a data access abstraction.

**Decision 6: Deterministic Persistence Axiom governs all persistence operations**

The axiom applies uniformly to state persistence, event persistence, snapshots, replays, and failure outcomes. There is no carve-out for non-deterministic persistence paths.

Rationale: ego-rs is a deterministic platform. Any persistence path that produces non-deterministic outcomes violates the platform's fundamental contract. The axiom must be absolute to preserve the integrity of deterministic replay and actor restoration.

**Decision 7: Versioning is constitutional, migration tooling is not**

Versioning semantics (version identifiers, compatibility validation, version-aware persistence and restoration, fail-closed mismatch handling) are constitutional because they affect determinism, replay safety, and restoration guarantees. Migration tooling (schema definition languages, DDL, transformation pipelines, upgrade sequences) is an implementation concern.

Rationale: Without constitutional versioning semantics, version mismatch during restoration would produce undefined behavior — silent corruption, degraded state, or non-deterministic outcomes. Versioning must be governed by the constitution. Migration execution is an adapter concern.

Alternatives considered:
- Omit versioning entirely: Rejected — version mismatch on restoration is a determinism failure that must be constitutionally governed.
- Delegate versioning to adapters: Rejected — would create non-deterministic restoration paths outside constitutional governance.
- Include migration semantics: Rejected — migration tooling is implementation-specific and would violate vendor and implementation neutrality.

**Decision 8: PostgreSQL is first implementation target but MUST NOT appear in constitutional contracts**

All references to PostgreSQL, its features, or its semantics are non-normative examples. The constitutional spec must not reference any vendor, product, or implementation.

Rationale: The SPI must be implementable through heterogeneous storage realizations. Vendor neutrality is a constitutional invariant.

## Risks / Trade-offs

- [Risk] Over-specification constrains future storage implementations → [Mitigation] Capability model distinguishes mandatory vs. optional; forbidden list is narrow and essential.
- [Risk] Under-specification leads to ambiguous implementations → [Mitigation] Each requirement includes deterministic scenarios with WHEN/THEN format; testing contract enforces 95%+ coverage.
- [Risk] Circular dependency between persistence and actor model → [Mitigation] Explicit architectural boundary: Persistence Contract depends on Runtime Contract and Canonical Contracts only, not on Actor Contract.
- [Risk] Single coherent spec grows too large to maintain → [Mitigation] The persistence domain is bounded; the spec defines semantics only, not implementation mechanics, keeping it focused and contained.
- [Risk] Future CQRS/ES capabilities may require relaxation of the Deterministic Axiom → [Mitigation] The axiom is written to accommodate event-sourced persistence (identical events + identical state produce identical outcomes); it does not require transactional consistency from the storage layer.
- [Risk] Tenant isolation semantics may be interpreted as requiring specific isolation implementations → [Mitigation] Explicit forbidden list: schema-per-tenant, database-per-tenant, shared table models are non-normative; tenant implementation neutrality is a constitutional invariant.
- [Risk] Persistence evolution semantics may drift toward migration engine specification → [Mitigation] Explicit non-goal: evolution semantics are NOT database migrations, schema tooling, DDL, or SQL migration engines.
- [Risk] Unified contract may be misinterpreted as requiring a single storage adapter → [Mitigation] Explicit provision: storage realization MAY vary while contractual surface remains identical.
- [Risk] "Data" terminology may be misinterpreted as byte/blob representation → [Mitigation] Representation-neutral "persistence artifacts" terminology; explicit representation neutrality invariant in governance section.
- [Risk] Event persistence framing may over-bias toward event sourcing → [Mitigation] Explicit clarification: event persistence is a supported semantic, not a redefinition of Persistence SPI; persistence remains valid for actor, workflow, service, and state persistence.
- [Risk] Tenant portability may be treated as weakening deterministic guarantees → [Mitigation] Explicit constitutional boundary: portability MUST preserve determinism, isolation, and restoration guarantees.
- [Risk] Versioning semantics may drift toward schema migration specification → [Mitigation] Explicit non-goal: versioning does NOT define schema definition languages, DDL, migration scripts, transformation pipelines, or upgrade sequences.
