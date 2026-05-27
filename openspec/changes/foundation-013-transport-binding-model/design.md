## Context

The Service Contract Model (FOUNDATION-012) constitutionally defines WHAT service interactions mean — service contract semantics, endpoint contract boundaries, exposure descriptors, and service policy attachment. However, there is no constitutional governance for HOW those service contracts become transport-exposed. Transport exposure is currently implicit, creating risk that transport semantics leak into service definitions, retry behavior becomes inconsistent, observability becomes transport-dependent, and replay trustworthiness degrades.

Key constraints:
- Must remain transport-neutral (no REST, gRPC, Kafka, HTTP, GraphQL, AMQP, NATS, RPC)
- Must not prescribe serialization, schema technologies, or networking
- Must complement FOUNDATION-012 without duplicating service semantics
- Must align with the severity classification model established by Canonical Contracts Constitution
- Must be constitutional, deterministic, fail-closed, and implementation-agnostic

## Goals / Non-Goals

**Goals:**
- Define transport binding semantics as HOW service contracts become exposed, distinct from WHAT they mean
- Define endpoint exposure binding model that preserves deterministic intent
- Define exposure descriptor binding for transport-neutral exposure visibility
- Define transport policy attachment semantics that remain declarative and explicit
- Define deterministic transport behavior that preserves replay equivalence
- Define fail-closed transport behavior for ambiguous or incompatible exposure
- Define transport observability semantics that do not mutate service meaning
- Define governance enforcement with constitutional severity alignment
- Define cross-spec governance with explicit authority ownership, especially with Service Contract Model
- Amend `architecture-governance` to cross-reference the Transport Binding Model
- Amend `runtime-abstraction` to cross-reference the Transport Binding Model for runtime-mediated transport exposure

**Non-Goals:**
- Prescribing transport protocols (REST, gRPC, HTTP, Kafka, GraphQL, AMQP, NATS, RPC)
- Defining networking, serialization, or schema technologies
- Implementing runtime transport execution, retries, circuit breakers, or rate limiting
- Defining service semantics (governed by Service Contract Model)
- Duplicating existing Service Contract Model, Canonical Contracts Constitution, or Determinism Constitution requirements
- Prescribing OpenAPI, protobuf, JSON Schema, Avro, or equivalent schema technologies

## Decisions

**Decision 1: Dedicated transport binding spec vs. extending Service Contract Model**
- Approach: Create a standalone `transport-binding-model` spec
- Rationale: Transport binding is a distinct governance concern from service contract semantics. Service Contract Model defines WHAT an interaction means; Transport Binding Model defines HOW that meaning is exposed. Merging them would conflate semantic governance with exposure governance, violating separation of concerns.
- Alternatives considered: Extending Service Contract Model (would conflate service semantics with transport exposure), embedding in Architecture Governance (too broad)

**Decision 2: Exposure models as semantic categories, not protocol patterns**
- Approach: Transport binding MAY expose through request/reply, stream-oriented, publish/subscribe, operator-facing, runtime-mediated, or external integration interactions — these are semantic exposure models, not protocol prescriptions
- Rationale: Semantic exposure categories enable transport-neutral governance while acknowledging that different exposure patterns have different deterministic expectations. A request/reply exposure has different semantics from a publish/subscribe exposure independent of protocol choice.
- Alternatives considered: Classifying by protocol pattern (HTTP method, Kafka topic) — rejected because it violates transport neutrality

**Decision 3: Endpoint exposure binding as separate from service endpoint contracts**
- Approach: Service Contract Model defines endpoint contracts (WHAT the interaction does). Transport Binding Model defines endpoint exposure binding (HOW the interaction is made available). Separate concerns with explicit boundary.
- Rationale: A single service endpoint contract may be exposed through multiple transport bindings. Decoupling exposure from semantics preserves transport neutrality and enables multiple exposure strategies.
- Alternatives considered: Merging exposure binding into endpoint contracts (would tie service semantics to transport exposure, violating neutrality)

**Decision 4: Severity classification alignment**
- Approach: Use the same four-level model (Constitutional violation, Validation failure, Non-conformant behavior, Incomplete change) established in Canonical Contracts Constitution
- Rationale: Consistent severity semantics across all constitutional specs simplifies enforcement and tooling.

**Decision 5: Cross-spec governance with explicit service-to-transport boundary**
- Approach: Service Contract Model governs service semantics; Transport Binding Model governs exposure semantics. Neither spec can govern the other's domain. Cross-references at archive time ensure consistency.
- Rationale: Clear separation prevents transport semantics from mutating service meaning and vice versa. The WHAT vs. HOW distinction is the constitutional boundary.

## Governance Ownership

**Transport Binding Model and Service Contract Model**: These two specs form a WHAT/HOW governance pair. Service Contract Model governs semantic service boundaries — what interactions mean, what endpoints do, what policies govern behavior. Transport Binding Model governs how those boundaries become exposed — how endpoints are made available, how exposure descriptors define visibility, how transport-level policies attach without mutating service meaning. Neither spec can govern the other's domain. A constitutional violation occurs if transport binding changes service semantics or if service semantics prescribe transport behavior.

**Transport Binding Model and Canonical Contracts Constitution**: Compatibility expectations at the transport level (endpoint exposure compatibility, binding evolution) SHALL be governed by Canonical Contracts Constitution. Transport Binding Model governs exposure semantics but does not redefine compatibility, evolution, or replay-safe interpretation — those remain with Canonical Contracts.

**Transport Binding Model and Runtime Abstraction**: Runtime-mediated transport exposure requires joint governance. Runtime Abstraction governs runtime capability semantics and execution expectations. Transport Binding Model governs how service interactions exposed through runtime capabilities are bound, exposed, and policy-attached. Neither spec alone can govern runtime-mediated transport exposure completely — Runtime Abstraction cannot define transport exposure semantics, and Transport Binding Model cannot define runtime execution semantics. Joint governance with non-overlapping authority resolves this.

**Transport binding exclusion from service semantics**: Transport binding intentionally excludes service semantics, canonical contract semantics, and architectural layer rules. This exclusion preserves the Service Contract Model's authority over service interaction meaning and prevents transport implementation details from leaking into the constitutional governance framework.

## Risks / Trade-offs

- **[Overlap with Service Contract Model]** → Clear boundary: Service Contract Model governs WHAT; Transport Binding Model governs HOW. Every requirement explicitly distinguishes between semantic governance and exposure governance. Cross-references at archive time prevent duplication.
- **[Cross-reference brittleness]** → Spec names are stable constitutional identifiers. Archive workflow resolves cross-references at archive time.
- **[Scope creep into transport protocols]** → Clear non-goals and constitutional review gate prevent transport prescriptions. Every requirement uses semantic exposure categories, never protocol-specific terminology.
- **[Ambiguity between exposure and semantics]** → Explicit criteria: if a concern describes WHAT an interaction means, it belongs to Service Contract Model. If it describes HOW that meaning is exposed, it belongs to Transport Binding Model.