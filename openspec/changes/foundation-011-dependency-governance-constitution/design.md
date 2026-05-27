## Context

Dependency relationships across hexagonal architecture layers, runtime boundaries, and workspace modules are currently governed implicitly by `architecture-governance` and `runtime-abstraction` specs. There is no centralized dependency governance. Hidden coupling, direction violations, and version drift risk accumulating without constitutional enforcement.

Key constraints:
- No package manager, build tooling, or dependency injection framework prescriptions
- Must remain language-neutral and implementation-agnostic
- Must complement existing specs without duplication
- Must align with the severity classification model used by Determinism Constitution and Canonical Contracts Constitution

## Goals / Non-Goals

**Goals:**
- Define allowed dependency directions that preserve architectural boundaries
- Define forbidden dependencies (cyclic, bypass, hidden coupling)
- Define dependency governance rules with explicit visibility expectations
- Define version governance for dependency evolution
- Define workspace dependency governance
- Define hidden coupling prevention with explicit detection
- Define governance enforcement with four severity levels
- Amend `architecture-governance` to cross-reference the new spec

**Non-Goals:**
- Prescribing package managers, build tooling, or dependency scanners
- Prescribing dependency injection frameworks
- Implementing workspace tooling or module resolution
- Defining build pipelines
- Duplicating existing `architecture-governance` or `runtime-abstraction` requirements

## Decisions

**Decision 1: Dedicated dependency governance spec vs. extending architecture governance**
- Approach: Create a standalone `dependency-governance-constitution` spec
- Rationale: Dependency governance is cross-cutting (architecture, runtime, workspace, versions). A single spec provides unified governance without bloating `architecture-governance`.
- Alternatives considered: Extending `architecture-governance` (would conflate architecture with dependency governance), embedding in `project-constitution` (too general)

**Decision 2: Severity classification alignment**
- Approach: Use the same four-level model (Constitutional violation, Validation failure, Non-conformant behavior, Incomplete change) established in Canonical Contracts Constitution
- Rationale: Consistent severity semantics across all constitutional specs simplifies enforcement and tooling

**Decision 3: Cross-spec governance model**
- Approach: Dependency Governance references existing specs without duplicating them. Architecture Governance cross-references Dependency Governance for dependency direction.
- Rationale: Clean separation of concerns. Each spec owns its domain. Cross-references at archive time ensure consistency.

## Risks / Trade-offs

- **[Scope creep into build tooling]** → Clear non-goals and constitutional review gate prevent tooling prescriptions
- **[Overlap with Architecture Governance]** → Clear boundary: Architecture Governance defines layers and allowed directions; Dependency Governance defines violation classification, hidden coupling, version governance, and enforcement. Cross-references prevent duplication.
- **[Cross-reference brittleness]** → Spec names are stable constitutional identifiers. Archive workflow resolves cross-references at archive time.
