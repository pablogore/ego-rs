## Context

Service interaction boundaries across ego-rs are currently implicit. Runtime abstractions define capability ports, Architecture Governance defines architectural boundaries, and the Canonical Contracts Constitution governs semantic contracts — but none constitutionally define service-level contract semantics, endpoint contract boundaries, exposure descriptors, or service-level policy attachment. Transport semantics risk leaking into service definitions, runtime boundaries become ambiguous, and replay trustworthiness degrades without constitutional service contract governance.

Key constraints:
- Must remain transport-neutral (no REST, gRPC, Kafka, HTTP, GraphQL, RPC)
- Must not prescribe serialization, schema technologies, or networking
- Must complement existing specs (Canonical Contracts Constitution, Determinism Constitution, Architecture Governance, Runtime Abstraction, Dependency Governance Constitution)
- Must align with the severity classification model established by Canonical Contracts Constitution
- Must be constitutional, deterministic, and implementation-agnostic

## Goals / Non-Goals

**Goals:**
- Define service contract semantics as deterministic behavioral boundaries between producers and consumers
- Define endpoint contract model with explicit, unambiguous semantics
- Define exposure descriptor model for transport-neutral service visibility
- Define service policy attachment semantics that remain declarative and explicit
- Define deterministic interaction boundaries that preserve replay equivalence
- Define fail-closed service behavior for ambiguous or incompatible interactions
- Define service observability semantics that preserve meaning
- Define governance enforcement with constitutional severity alignment
- Define cross-spec governance with explicit authority ownership
- Amend `architecture-governance` to cross-reference the Service Contract Model

**Non-Goals:**
- Prescribing transport protocols (REST, gRPC, HTTP, Kafka, GraphQL, RPC)
- Defining networking, serialization, or schema technologies
- Implementing runtime messaging or transport binding
- Defining retries, timeouts, circuit breakers, or runtime execution behavior
- Duplicating existing Canonical Contracts Constitution or Determinism Constitution requirements
- Prescribing OpenAPI, protobuf, JSON Schema, Avro, or equivalent schema technologies

## Decisions

**Decision 1: Dedicated service contract spec vs. extending Canonical Contracts Constitution**
- Approach: Create a standalone `service-contract-model` spec
- Rationale: Service contract governance is distinct from general canonical contract semantics. Canonical Contracts defines contracts at all boundaries; Service Contract Model specifically governs service-level interaction semantics, endpoint contracts, exposure descriptors, and policy attachment. Separation prevents bloating the canonical contracts spec with service-specific concerns.
- Alternatives considered: Extending Canonical Contracts Constitution (would conflate general contract semantics with service-level concerns), embedding in Architecture Governance (too broad)

**Decision 2: Service classification as transport-agnostic behavioral categories**
- Approach: Services MAY represent commands, queries, approvals, validation requests, runtime interaction boundaries, operator workflows, or integration boundaries — but these are semantic categories, not transport operations
- Rationale: Semantic categories preserve transport neutrality while enabling deterministic behavioral expectations. A command is semantically distinct from a query regardless of how it is transported.
- Alternatives considered: Classifying by transport pattern (HTTP method, Kafka topic) — rejected because it violates transport neutrality

**Decision 3: Exposure descriptor as separate concern from endpoint contract**
- Approach: Separate exposure descriptors (who can see/access) from endpoint contracts (what the interaction does)
- Rationale: Clear separation between visibility/policy and behavioral semantics. A single service may expose the same endpoint contract differently to different consumers.
- Alternatives considered: Merging exposure and endpoint concerns into a single descriptor (conflates visibility with behavior, makes policy attachment implicit)

**Decision 4: Severity classification alignment**
- Approach: Use the same four-level model (Constitutional violation, Validation failure, Non-conformant behavior, Incomplete change) established in Canonical Contracts Constitution
- Rationale: Consistent severity semantics across all constitutional specs simplifies enforcement and tooling. Service contract violations should map to the same classification as canonical contract violations.

**Decision 5: Cross-spec governance with explicit ownership**
- Approach: Each spec owns its domain. Canonical Contracts Constitution remains authoritative for contract semantics. Determinism Constitution for deterministic expectations. Architecture Governance for architectural boundaries. Service Contract Model for service-specific concerns.
- Rationale: Clear authority prevents governance ambiguity. Cross-references at archive time ensure consistency without duplication.

## Governance Ownership

**Service Contract Model and Canonical Contracts Constitution**: Canonical Contracts Constitution governs semantic contracts at all boundaries — defining contract semantics, compatibility, evolution, and replay-safe interpretation universally. Service Contract Model governs service-level interaction boundaries specifically, including endpoint contracts, exposure descriptors, and policy attachment. The separation exists because service-level governance involves producer-consumer behavioral boundaries with distinct concerns (exposure, policy, endpoint semantics) that would bloat the general canonical contracts spec. Service Contract Model preserves canonical contract semantics without redefining them.

**Architecture Governance and Service Contract Model**: Cross-layer service interactions require joint governance. Architecture Governance determines whether an interaction follows layer rules and dependency direction. Service Contract Model determines whether the interaction complies with service contract semantics, endpoint governance, and policy attachment. Neither spec alone can govern cross-layer service boundaries completely — Architecture Governance cannot define semantic service behavior, and Service Contract Model cannot define architectural correctness. Joint governance with non-overlapping authority resolves this.

**Transport binding exclusion**: Transport binding (REST, gRPC, Kafka, HTTP, GraphQL, RPC, serialization, networking) remains intentionally excluded from all constitutional specs. Service contracts define semantic interaction expectations. Transport binding defines how those semantics are delivered at runtime. Coupling semantics to transport would violate determinism, replay safety, and transport neutrality. Transport binding SHALL be governed separately outside this constitution.

## Risks / Trade-offs

- **[Overlap with Canonical Contracts Constitution]** → Clear boundary: Canonical Contracts governs contracts at all boundaries (semantic contracts, compatibility, evolution). Service Contract Model governs service-level concerns (endpoints, exposure, policy). Cross-references at archive time prevent duplication.
- **[Cross-reference brittleness]** → Spec names are stable constitutional identifiers. Archive workflow resolves cross-references at archive time.
- **[Scope creep into transport binding]** → Clear non-goals and constitutional review gate prevent transport prescriptions.
- **[Ambiguity between service and non-service contracts]** → Explicit criteria: a service contract exists whenever a behavioral interaction boundary exists between producer and consumer. All other contracts (e.g., runtime SPI contracts) remain governed by Canonical Contracts Constitution alone.
