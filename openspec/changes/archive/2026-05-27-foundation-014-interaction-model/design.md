## Context

ego-rs defines a Service Contract Model (FOUNDATION-012) governing WHAT interactions mean and a Transport Binding Model (FOUNDATION-013) governing HOW interactions become transport-exposed. However, no constitutional spec governs HOW participants interact — the interaction semantics that sit between service meaning and transport exposure. Interaction semantics are currently implicit, leading to ambiguous response expectations, inconsistent workflow coordination, degraded observability, and compromised replay trustworthiness.

Key constraints:
- Must remain interaction-neutral (no actor models, queues, brokers, messaging systems)
- Must not prescribe transport protocols (REST, gRPC, Kafka, HTTP, GraphQL, RPC)
- Must not prescribe runtime implementations, orchestration engines, or schedulers
- Must complement existing specs (Service Contract Model, Transport Binding Model, Canonical Contracts Constitution, Determinism Constitution, Architecture Governance, Runtime Abstraction, Dependency Governance Constitution)
- Must align with the severity classification model established by Canonical Contracts Constitution
- Must be constitutional, deterministic, fail-closed, and implementation-agnostic

## Goals / Non-Goals

**Goals:**
- Define interaction semantics — HOW participants interact across governed boundaries
- Define request/reply interaction semantics with explicit response expectations
- Define fire-and-forget interaction semantics without response expectations
- Define publish/subscribe interaction semantics with governed multi-participant observation
- Define stream interaction semantics with deterministic ordering
- Define approval interaction semantics with deterministic approval expectations
- Define workflow interaction semantics spanning deterministic execution boundaries
- Define deterministic interaction behavior that preserves replay equivalence
- Define fail-closed interaction behavior for ambiguous or incompatible interactions
- Define interaction observability semantics that preserve meaning
- Define governance enforcement with constitutional severity alignment
- Define cross-spec governance with explicit authority ownership
- Amend `architecture-governance` to cross-reference the Interaction Model
- Amend `runtime-abstraction` to cross-reference the Interaction Model for runtime-mediated interactions

**Non-Goals:**
- Prescribing actor models, queues, brokers, or messaging systems
- Prescribing transport protocols (REST, gRPC, HTTP, Kafka, GraphQL, AMQP, NATS, RPC)
- Defining networking, serialization, or schema technologies
- Implementing runtime workflow execution, retries, or orchestration
- Defining service semantics (governed by Service Contract Model)
- Defining transport exposure semantics (governed by Transport Binding Model)
- Duplicating existing Service Contract Model, Transport Binding Model, Canonical Contracts Constitution, or Determinism Constitution requirements
- Prescribing workflow engines, saga pattern implementations, or orchestration frameworks

## Decisions

**Decision 1: Dedicated interaction model spec vs. extending Service Contract Model**
- Approach: Create a standalone `interaction-model` spec
- Rationale: Interaction semantics are distinct from service contract semantics. Service Contract Model defines WHAT an interaction means (commands, queries, endpoints). Interaction Model defines HOW participants interact (request/reply, fire-and-forget, pub/sub, streams, approvals, workflows). Merging them would conflate behavioral semantics with interaction patterns, violating separation of concerns.
- Alternatives considered: Extending Service Contract Model (would conflate service meaning with interaction mechanics), embedding in Transport Binding Model (interaction is not exposure — participants interact before transport is in scope)

**Decision 2: Interaction types as semantic interaction models, not protocol patterns**
- Approach: Define interaction semantics through governed interaction types: request/reply, fire-and-forget, publish/subscribe, stream, approval, and workflow. These are semantic interaction models that describe HOW participants interact, not WHAT interactions mean or HOW they are exposed.
- Rationale: Semantic interaction categories enable governance of participant behavior without prescribing implementation. A request/reply interaction means a response is expected, regardless of whether it is implemented via actors, queues, direct calls, or any other mechanism.
- Alternatives considered: Defining interaction through protocol patterns (sync/async, pub/sub transport) — rejected because it violates implementation neutrality

**Decision 3: WHAT / HOW / INTERACTION three-way governance model**
- Approach: The governance framework establishes a three-way separation: Service Contract Model governs WHAT interactions mean, Transport Binding Model governs HOW interactions become transport-exposed, Interaction Model governs HOW participants interact. Each spec owns its domain with non-overlapping authority.
- Rationale: This three-way model prevents conflation of concerns. Service contracts define intent; interactions define participant behavior; transport binding defines exposure. Ambiguity arises when these domains overlap.
- Alternatives considered: Two-way model with interactions embedded in service contracts or transport binding — rejected because both approaches conflate distinct governance concerns

**Decision 4: Severity classification alignment**
- Approach: Use the same four-level model (Constitutional violation, Validation failure, Non-conformant behavior, Incomplete change) established in Canonical Contracts Constitution
- Rationale: Consistent severity semantics across all constitutional specs simplifies enforcement and tooling.

**Decision 5: Cross-spec governance with explicit ownership**
- Approach: Each spec owns its domain. Service Contract Model governs service semantics. Transport Binding Model governs exposure semantics. Interaction Model governs participant interaction semantics. No spec can govern another's domain. Cross-references at archive time ensure consistency without duplication.
- Rationale: Clear authority prevents governance ambiguity. The WHAT / HOW / INTERACTION distinction is the constitutional boundary.

## Governance Ownership

**Interaction Model and Service Contract Model**: These two specs form a WHAT/INTERACTION governance pair. Service Contract Model governs WHAT interaction means — service semantics, endpoint contracts, exposure descriptors. Interaction Model governs HOW participants interact — response expectations, ordering semantics, interaction patterns. A service contract may define a command, but the Interaction Model determines whether that command follows request/reply or fire-and-forget interaction semantics. Neither spec can govern the other's domain.

**Interaction Model and Transport Binding Model**: These two specs form an INTERACTION/HOW governance pair. Interaction Model governs participant interaction behavior. Transport Binding Model governs how interactions become transport-exposed. A publish/subscribe interaction may be exposed through multiple transport bindings; the Interaction Model governs the publish/subscribe semantics while Transport Binding Model governs the exposure binding. Cross-references at archive time ensure consistency.

**Interaction Model and Canonical Contracts Constitution**: Compatibility expectations for interaction semantics (interaction pattern compatibility, interaction evolution) SHALL be governed by Canonical Contracts Constitution. Interaction Model governs participant interaction semantics but does not redefine compatibility, evolution, or replay-safe interpretation — those remain with Canonical Contracts.

**Interaction Model and Runtime Abstraction**: Runtime-mediated participant interactions require joint governance. Runtime Abstraction governs runtime capability semantics and execution expectations. Interaction Model governs how participants interact through runtime-mediated channels. Neither spec alone can govern runtime-mediated interactions completely.

## Risks / Trade-offs

- **[Overlap with Service Contract Model]** → Clear boundary: Service Contract Model governs WHAT; Interaction Model governs HOW participants interact. Every requirement explicitly distinguishes between semantic governance and interaction governance. WHAT vs. HOW participants interact is the split.
- **[Overlap with Transport Binding Model]** → Clear boundary: Interaction Model governs participant interaction semantics; Transport Binding Model governs exposure binding. INTERACTION vs. EXPOSURE is the split.
- **[Cross-reference brittleness]** → Spec names are stable constitutional identifiers. Archive workflow resolves cross-references at archive time.
- **[Scope creep into actor models or orchestration engines]** → Clear non-goals and constitutional review gate prevent implementation prescriptions. Every requirement uses semantic interaction categories, never implementation-specific terminology.
- **[Ambiguity between interaction and service semantics]** → Explicit criteria: if a concern describes WHAT a service interaction means, it belongs to Service Contract Model. If it describes HOW participants interact through that service, it belongs to Interaction Model.