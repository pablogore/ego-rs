## Context

ego-rs is a deterministic-first, fail-closed, replay-safe, placement-governed, persistence-governed, projection-governed, behavior-governed, interaction-governed, transport-governed, contract-governed, architecture-governed, lineage-aware, hexagonal system. Existing FOUNDATION specifications govern service meaning (012), transport exposure (013), participant interaction (014), behavior execution (015), read materialization (016), durable truth preservation (017), and execution ownership in space (018). However, lifecycle evolution semantics — how governed things activate, suspend, recover, restore, and transition through lifecycle — remain constitutionally ungoverned.

Without a dedicated Lifecycle Model constitution, the system lacks governance for lifecycle semantics that are critical to deterministic execution trustworthiness and fail-closed guarantees. The constitutional ownership chain defined across FOUNDATION-012 through FOUNDATION-018 is incomplete without lifecycle governance. This gap creates ambiguity that threatens the constitutional invariants of the platform.

## Goals / Non-Goals

**Goals:**

- Define constitutional governance for lifecycle semantics across ego-rs
- Define activation semantics — deterministic, replay-safe, implementation-neutral
- Define suspension semantics — deterministic, replay-safe, implementation-neutral
- Define recovery semantics — deterministic, replay-safe, restoration-trustworthy
- Define restoration semantics — deterministic, replay-safe, implementation-neutral
- Define lifecycle transition semantics — activation, suspension, recovery, restoration, termination
- Define lifecycle consistency expectations — explicit, deterministic, replay-safe
- Define deterministic lifecycle behavior — equivalent inputs produce equivalent lifecycle semantics
- Define lifecycle failure semantics — fail-closed, without retry or orchestration prescriptions
- Define replay-safe lifecycle semantics — equivalent replay preserves equivalent lifecycle
- Define lifecycle observability semantics — equivalent behavior produces equivalent observables
- Define explicit, non-overlapping authority boundaries with Behavior Model, Projection Model, Persistence Model, Placement Model, Runtime Abstraction, and Architecture Governance
- Complete the constitutional ownership chain with lifecycle governance

**Non-Goals:**

- Do NOT define actors, schedulers, orchestrators, or supervision systems
- Do NOT prescribe Akka, Lagom, Orleans, Temporal, Kubernetes, or any specific lifecycle framework
- Do NOT define workflow engines, lifecycle frameworks, cluster implementations, or runtime frameworks
- Do NOT define transport protocols, node topologies, or orchestration engines
- Do NOT modify how behavior executes (governed by Behavior Model)
- Do NOT modify how read knowledge materializes (governed by Projection Model)
- Do NOT modify how durable truth is preserved (governed by Persistence Model)
- Do NOT modify how execution ownership exists in space (governed by Placement Model)
- Do NOT modify how execution is implemented (governed by Runtime Abstraction)
- Do NOT modify what interaction means (governed by Service Contract Model)

## Decisions

**Decision 1: Lifecycle Model as a semantic constitution, not an implementation**

The Lifecycle Model governs the semantic meaning of *HOW governed things evolve through lifecycle* constitutionally — not how it is implemented. This mirrors the approach of the Behavior Model (governs behavior execution), Persistence Model (governs durable truth preservation), and Placement Model (governs ownership in space). All normative language uses SHALL/MUST/MUST NOT at the semantic level without scheduler, actor, orchestration, or runtime prescriptive detail.

Rationale: The foundational pattern established by FOUNDATION-012 through FOUNDATION-018 separates semantic governance from implementation. The Lifecycle Model completes this separation by governing the lifecycle evolution dimension. This ensures the entire WHAT → HOW exposed → HOW interact → HOW execute → HOW materialize → HOW durable truth preserve → HOW ownership exist in space → HOW lifecycle evolve chain is constitutionally governed without implementation coupling.

**Decision 2: Non-overlapping authority boundaries via explicit scope declarations**

Each constitution SHALL declare its authority scope, and cross-spec governance sections SHALL define explicit non-overlapping boundaries. The Lifecycle Model governs *how governed things evolve through lifecycle* — distinct from the Behavior Model's governance of *how behavior executes*, the Placement Model's governance of *how execution ownership exists in space*, and the Persistence Model's governance of *how durable truth is preserved*. This prevents authority ambiguity and ensures each spec maintains a single, clear concern.

Rationale: Without explicit authority boundaries, lifecycle overlaps with behavior execution (activation vs execution), ownership in space (placement lifecycle vs lifecycle evolution), and durable truth (restoration vs recovery) create ambiguity about which constitution takes precedence. Explicit scope declarations prevent this.

**Decision 3: Constitutional severity classification for governance enforcement**

Violations are classified into four severities: constitutional violation, validation failure, non-conformant behavior, and incomplete change. This provides a graduated enforcement model consistent with FOUNDATION-012 through FOUNDATION-018.

Rationale: Consistent severity classification across constitutional specs enables uniform governance enforcement without introducing per-spec enforcement logic.

**Decision 4: Delta specs for modified capabilities (ADDED Requirements only)**

For behavior-model, projection-model, persistence-model, placement-model, runtime-abstraction, and architecture-governance, only ADDED requirements are introduced — clarifying authority boundaries without modifying existing requirements. This avoids the risk of losing detail at archive time while establishing the necessary cross-spec governance.

Rationale: The requirements specify authority boundary clarifications, not behavioral changes to existing requirements. Using ADDED (not MODIFIED) preserves existing spec content intact.

**Decision 5: Every requirement must have at least one WHEN/THEN scenario**

All requirements include explicit testable scenarios using the WHEN/THEN format.

Rationale: Requirements without scenarios cannot be tested or verified. This constraint ensures governance is concrete and enforceable.

**Decision 6: Constitutional ownership chain extended with lifecycle governance**

The Lifecycle Model completes the constitutional ownership chain established across FOUNDATION-012 through FOUNDATION-018. The chain SHALL include Lifecycle Model as the terminal link: WHAT interaction means → HOW interaction becomes exposed → HOW participants interact → HOW behavior executes → HOW behavior becomes materialized as read knowledge → HOW durable truth is preserved and restored → HOW execution ownership exists in space → HOW governed things evolve through lifecycle.

Rationale: The constitutional ownership chain SHALL remain explicit and non-overlapping across all foundation specs. The Lifecycle Model is the final link completing this invariant.

## Risks / Trade-offs

**[Risk] Authority boundary ambiguity with Behavior Model** → Mitigation: Lifecycle Model governs lifecycle evolution semantics; Behavior Model governs execution semantics. Activation is a lifecycle concern (when and how something becomes active), not a behavior execution concern (what happens during execution). Explicit authority declarations prevent overlap.

**[Risk] Lifecycle semantics confused with placement lifecycle** → Mitigation: Placement lifecycle (ownership establishment, mobility, termination) is a dimension of placement governance. Lifecycle Model governs evolution semantics (activation, suspension, recovery, restoration) which are distinct from ownership movement semantics. Explicit scope separation clarifies this.

**[Risk] Recovery semantics overlap with persistence restoration** → Mitigation: Recovery semantics govern how lifecycle returns to a known state after failure; Persistence Model governs how durable truth is preserved and restored. Recovery is about lifecycle state evolution, not data restoration. Explicit authority boundaries prevent confusion.

**[Risk] Activation semantics perceived as runtime scheduling** → Trade-off: Activation is defined as a semantic concept — how governed meaning transitions from inactive to active — not as a scheduling or orchestration concept. Explicit non-goals prevent runtime leakage.
