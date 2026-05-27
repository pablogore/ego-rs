## Context

ego-rs is a deterministic-first, fail-closed, replay-safe, placement-governed, persistence-governed, projection-governed, behavior-governed, interaction-governed, transport-governed, contract-governed, architecture-governed, lineage-aware, hexagonal system. Existing FOUNDATION specifications govern service meaning (012), transport exposure (013), participant interaction (014), behavior execution (015), read materialization (016), and durable truth preservation (017). However, execution ownership semantics — where ownership exists, how it moves, and how it remains governable in space — remain constitutionally ungoverned.

Without a dedicated Placement Model constitution, the system lacks governance for ownership semantics that are critical to deterministic execution trustworthiness and fail-closed guarantees. The constitutional ownership chain defined across FOUNDATION-012 through FOUNDATION-017 is incomplete without placement governance. This gap creates ambiguity that threatens the constitutional invariants of the platform.

## Goals / Non-Goals

**Goals:**

- Define constitutional governance for placement semantics across ego-rs
- Define ownership semantics — deterministic, replay-safe, fail-closed
- Define locality semantics — deterministic, implementation-neutral
- Define execution location abstraction — abstract, no infrastructure assumptions
- Define mobility semantics — deterministic ownership movement
- Define placement lifecycle semantics — establishment, mobility, recovery, termination
- Define placement consistency expectations — explicit, deterministic, replay-safe
- Define deterministic placement behavior — equivalent inputs produce equivalent ownership semantics
- Define placement failure semantics — fail-closed, without retry or orchestration prescriptions
- Define replay-safe placement semantics — equivalent replay preserves equivalent ownership
- Define placement observability semantics — equivalent behavior produces equivalent observables
- Define explicit, non-overlapping authority boundaries with Behavior Model, Projection Model, Persistence Model, Runtime Abstraction, and Architecture Governance
- Complete the constitutional ownership chain with placement governance

**Non-Goals:**

- Do NOT define actors, schedulers, clusters, sharding implementations, or leader election
- Do NOT prescribe Akka, Lagom, Orleans, Temporal, Kubernetes, or any specific placement framework
- Do NOT define orchestration engines, placement frameworks, transport protocols, or network topologies
- Do NOT define node discovery systems, replication systems, or runtime frameworks
- Do NOT modify how behavior executes (governed by Behavior Model)
- Do NOT modify how read knowledge materializes (governed by Projection Model)
- Do NOT modify how durable truth is preserved (governed by Persistence Model)
- Do NOT modify how execution is implemented (governed by Runtime Abstraction)
- Do NOT modify what interaction means (governed by Service Contract Model)

## Decisions

**Decision 1: Placement Model as a semantic constitution, not an implementation**

The Placement Model governs the semantic meaning of *HOW execution ownership exists in space* constitutionally — not how it is implemented. This mirrors the approach of the Persistence Model (governs durable truth preservation) and the Behavior Model (governs behavior execution). All normative language uses SHALL/MUST/MUST NOT at the semantic level without cluster, scheduler, or runtime prescriptive detail.

Rationale: The foundational pattern established by FOUNDATION-012 through FOUNDATION-017 separates semantic governance from implementation. The Placement Model completes this separation by governing the final ungoverned dimension — execution ownership in space. This ensures the entire WHAT → HOW exposed → HOW interact → HOW execute → HOW materialize → HOW durable truth preserve → HOW ownership exist in space chain is constitutionally governed without implementation coupling.

**Decision 2: Non-overlapping authority boundaries via explicit scope declarations**

Each constitution SHALL declare its authority scope, and cross-spec governance sections SHALL define explicit non-overlapping boundaries. The Placement Model governs *how execution ownership exists in space* — distinct from the Behavior Model's governance of *how behavior executes* and the Persistence Model's governance of *how durable truth is preserved*. This prevents authority ambiguity and ensures each spec maintains a single, clear concern.

Rationale: Without explicit authority boundaries, ownership overlaps create ambiguity about which constitution takes precedence for placement semantics. Explicit scope declarations prevent this.

**Decision 3: Constitutional severity classification for governance enforcement**

Violations are classified into four severities: constitutional violation, validation failure, non-conformant behavior, and incomplete change. This provides a graduated enforcement model consistent with FOUNDATION-012 through FOUNDATION-017.

Rationale: Consistent severity classification across constitutional specs enables uniform governance enforcement without introducing per-spec enforcement logic.

**Decision 4: Delta specs for modified capabilities (ADDED Requirements only)**

For behavior-model, projection-model, persistence-model, runtime-abstraction, and architecture-governance, only ADDED requirements are introduced — clarifying authority boundaries without modifying existing requirements. This avoids the risk of losing detail at archive time while establishing the necessary cross-spec governance.

Rationale: The requirements specify authority boundary clarifications, not behavioral changes to existing requirements. Using ADDED (not MODIFIED) preserves existing spec content intact.

**Decision 5: Every requirement must have at least one WHEN/THEN scenario**

All requirements include explicit testable scenarios using the WHEN/THEN format.

Rationale: Requirements without scenarios cannot be tested or verified. This constraint ensures governance is concrete and enforceable.

**Decision 6: Constitutional ownership chain as a first-class invariant**

The Placement Model completes the constitutional ownership chain established across FOUNDATION-012 through FOUNDATION-017. The chain SHALL include Placement Model as the terminal link: WHAT interaction means → HOW interaction becomes exposed → HOW participants interact → HOW behavior executes → HOW behavior becomes materialized as read knowledge → HOW durable truth is preserved and restored → HOW execution ownership exists in space.

Rationale: The constitutional ownership chain SHALL remain explicit and non-overlapping across all foundation specs. The Placement Model is the final link completing this invariant.

## Risks / Trade-offs

**[Risk] Authority boundary ambiguity with Behavior Model** → Mitigation: Placement Model governs ownership in space; Behavior Model governs execution semantics. Explicit authority declarations prevent overlap.

**[Risk] Locality semantics may be misinterpreted as topology implementation** → Trade-off: Locality is defined as a semantic concept — how ownership relates to space — not as a physical or network topology concept.

**[Risk] Mobility semantics may be confused with runtime orchestration** → Mitigation: Mobility semantics govern ownership movement meaning, not orchestration implementation. Explicit non-goals prevent runtime leakage.

**[Risk] Placement consistency perceived as cluster consistency** → Trade-off: Placement consistency is constitutional consistency (deterministic, replay-safe), not distributed systems consistency.
