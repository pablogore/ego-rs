## Context

ego-rs is a deterministic-first, fail-closed, replay-safe, runtime-neutral, persistence-neutral, transport-neutral, contract-governed, interaction-governed, behavior-governed, projection-governed, hexagonal system. Existing FOUNDATION specifications govern service meaning (012), transport exposure (013), participant interaction (014), and behavior execution (015). However, the materialization of behavior outcomes into read knowledge — how projections execute, synchronize, replay, and expose governed read state — remains constitutionally ungoverned.

Without a dedicated Projection Model constitution, the system lacks governance for read-side materialization semantics that are critical to determinism, replay trustworthiness, and fail-closed guarantees. The Behavior Model defines *how behavior executes* but explicitly excludes *how behavior becomes materialized as read knowledge*. This gap creates ambiguity that threatens the constitutional invariants of the platform.

## Goals / Non-Goals

**Goals:**

- Define constitutional governance for projection semantics across ego-rs
- Define read-side materialization semantics — deterministic, replay-trustworthy, observable
- Define projection lifecycle semantics — init, activation, synchronization, restoration, termination
- Define replay-safe projections — equivalent replay produces equivalent read interpretation
- Define projection consistency expectations — explicit, no hidden assumptions
- Define deterministic projection behavior — equivalent outcomes, lifecycle, and replay produce equivalent read behavior
- Define projection observability semantics — equivalent behavior produces equivalent observables
- Define projection failure semantics — fail-closed, without prescribing retry or orchestration
- Define governance enforcement through constitutional severity classification
- Define explicit, non-overlapping authority boundaries with Behavior Model, Runtime Abstraction, and Architecture Governance

**Non-Goals:**

- Do NOT define databases, event stores, read model engines, or projection schedulers
- Do NOT prescribe Kafka, Postgres, ElasticSearch, Cassandra, Redis, or any specific projection technology
- Do NOT define streaming systems, replication systems, synchronization frameworks, or delivery guarantees
- Do NOT define persistence engines, indexing implementations, queues, or transport mechanisms
- Do NOT define retry implementations, orchestration frameworks, or CQRS libraries
- Do NOT modify what interaction means (governed by Service Contract Model)
- Do NOT modify how participants interact (governed by Interaction Model)
- Do NOT modify how behavior executes (governed by Behavior Model)
- Do NOT modify how execution is implemented at runtime (governed by Runtime Abstraction)

## Decisions

**Decision 1: Projection Model as a semantic constitution, not an implementation**

The Projection Model governs the semantic meaning of *HOW behavior becomes materialized as read knowledge* constitutionally — not how it is implemented. This mirrors the approach of the Behavior Model (governs how behavior executes) and the Interaction Model (governs how participants interact). All normative language uses SHALL/MUST/MUST NOT at the semantic level without persistence or runtime prescriptive detail.

Rationale: The foundational pattern established by FOUNDATION-012/013/014/015 separates semantic governance from implementation. The Projection Model extends this separation by governing the next ungoverned dimension — read materialization. This ensures the entire WHAT → HOW exposed → HOW interact → HOW execute → HOW materialize chain is constitutionally governed without implementation coupling.

**Decision 2: Non-overlapping authority boundaries via explicit scope declarations**

Each constitution SHALL declare its authority scope, and cross-spec governance sections SHALL define explicit non-overlapping boundaries. The Projection Model governs *how behavior becomes materialized as read knowledge* — distinct from the Behavior Model's governance of *how behavior executes*. This prevents authority ambiguity and ensures each spec maintains a single, clear concern.

Rationale: Without explicit authority boundaries, governance overlaps create ambiguity about which constitution takes precedence for projection semantics. Explicit scope declarations prevent this.

**Decision 3: Constitutional severity classification for governance enforcement**

Violations are classified into four severities: constitutional violation, validation failure, non-conformant behavior, and incomplete change. This provides a graduated enforcement model consistent with the Behavior Model approach.

Rationale: Consistent severity classification across constitutional specs enables uniform governance enforcement without introducing per-spec enforcement logic.

**Decision 4: Delta specs for modified capabilities (ADDED Requirements only)**

For behavior-model, runtime-abstraction, and architecture-governance, only ADDED requirements are introduced — clarifying authority boundaries without modifying existing requirements. This avoids the risk of losing detail at archive time while establishing the necessary cross-spec governance.

Rationale: The requirements specify authority boundary clarifications, not behavioral changes to existing requirements. Using ADDED (not MODIFIED) preserves existing spec content intact.

**Decision 5: Every requirement must have at least one WHEN/THEN scenario**

All requirements include explicit testable scenarios using the WHEN/THEN format. This ensures every constitutional requirement is verifiable and reviewable.

Rationale: Requirements without scenarios cannot be tested or verified. This constraint ensures governance is concrete and enforceable.

**Decision 6: Replay safety as a constitutional invariant for projections**

Projection replay equivalence SHALL be a constitutional invariant. Equivalent replay SHALL preserve observable read semantics, materialized interpretation, synchronization meaning, and consistency expectations. Replay divergence SHALL be treated as a constitutional violation.

Rationale: Replay trustworthiness is a core constitutional invariant of ego-rs. Without explicit replay governance for projections, replay divergence would threaten the determinism and trust guarantees that the platform provides.

## Risks / Trade-offs

**[Risk] Authority boundary ambiguity with Behavior Model** → Mitigation: Explicit cross-spec governance section defines WHAT/HOW/INTERACTION/BEHAVIOR/PROJECTION separation with dedicated scenarios verifying authority ownership.

**[Risk] Future specs may inadvertently overlap with Projection Model authority** → Mitigation: Every constitutional spec SHALL declare authority ownership. The Projection Model's authority scope is explicitly bounded ("how behavior becomes materialized as read knowledge") to prevent overreach.

**[Risk] Projection consistency expectations may be interpreted as delivery guarantees** → Trade-off: The spec explicitly separates consistency expectations (constitutional) from delivery guarantees (implementation). Consistency remains semantic; delivery is implementation-defined.

**[Risk] Implementers may confuse projection semantics with behavior semantics** → Trade-off: The spec establishes clear non-overlapping authority: Behavior Model governs HOW behavior executes; Projection Model governs HOW behavior becomes materialized as read knowledge. Projection is a consumer of behavior outcomes, not an executor of behavior.
