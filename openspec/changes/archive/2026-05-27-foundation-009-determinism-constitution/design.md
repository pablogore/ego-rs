## Context

Determinism is currently implicit across multiple specs: `project-constitution` defines a single `Deterministic-first behavior` requirement, and `runtime-abstraction` defines a `Determinism Axiom` constitutional invariant. Neither spec provides comprehensive governance for deterministic behavior. This design creates a dedicated **Determinism Constitution** spec that centralizes all determinism governance.

Key constraints:
- No runtime code, no implementation, no tooling, no library or framework prescriptions
- Must remain implementation-agnostic and future-proof
- Must be enforceable through governance review and automated validation
- Must cross-reference existing specs without duplicating their requirements

## Goals / Non-Goals

**Goals:**
- Define deterministic-by-default as a constitutional invariant
- Define forbidden nondeterminism with explicit categories
- Define deterministic capability mediation boundaries (time, randomness, ordering, scheduling)
- Define replay equivalence guarantees
- Define deterministic state behavior
- Define deterministic testing expectations
- Define governance enforcement with violation classification
- Define deterministic observability semantics
- Amend `project-constitution`, `runtime-abstraction`, and `testing-governance` to cross-reference the new spec

**Non-Goals:**
- Implementing schedulers, clocks, randomness providers, or runtime primitives
- Choosing concurrency strategies, libraries, or frameworks
- Prescribing infrastructure behavior or implementation tooling
- Defining implementation-specific testing frameworks
- Duplicating existing governance from `project-constitution` or `runtime-abstraction`

## Decisions

**Decision 1: Dedicated spec vs. extending existing specs**
- Approach: Create a standalone `determinism-constitution` spec rather than embedding determinism governance into every affected spec
- Rationale: Determinism is cross-cutting and constitutional. A single spec provides a unified governance surface, avoids fragmentation, and makes enforcement reviewable in one place. Existing specs cross-reference it rather than duplicate it.
- Alternatives considered: Embedding into `project-constitution` (would bloat the constitution), distributing across all affected specs (fragmented and inconsistent)

**Decision 2: Spec file naming convention**
- Approach: Use `determinism-constitution` as the kebab-case identifier, consistent with existing spec names like `project-constitution`, `examples-constitution`
- Rationale: Naming convention consistency across all governance specs

**Decision 3: Constitutional vs. technical language**
- Approach: Use RFC-style normative language (SHALL, MUST, MUST NOT) focused on what constitutes deterministic behavior, not how to implement it
- Rationale: The user explicitly requires implementation-agnostic, constitutional language. Avoids coupling to any runtime, library, or framework.

**Decision 4: Governance enforcement through classification**
- Approach: Define constitutional severity levels (constitutional violation, validation failure) rather than prescribing tooling or CI mechanics
- Rationale: Severity classification enables enforcement without coupling to specific tooling. Tooling decisions belong in implementation specs.

**Decision 5: Delta specs for modified capabilities**
- Approach: Create short delta spec files for `project-constitution`, `runtime-abstraction`, and `testing-governance` showing only the amendments
- Rationale: Minimizes change surface. The archive workflow will merge these deltas into the canonical spec locations.

## Risks / Trade-offs

- **[Constitutional scope creep]** → Clear non-goals section and constitutional review gate prevent implementation details from leaking into the spec
- **[Existing spec conflicts]** → Cross-referencing instead of duplicating means existing requirements remain authoritative in their original specs. Audit at archive time ensures no conflicts.
- **[Cross-reference brittleness]** → Spec names are stable constitutional identifiers. Archive workflow resolves cross-references to canonical paths.
- **[Over-specification]** → Constitutional language focuses on what, not how. Implementation detail is explicitly excluded per non-goals.
