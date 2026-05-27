## Why

Determinism is a constitutional invariant of ego-rs, but it is currently implicit across multiple specs (`project-constitution`, `runtime-abstraction`) and not formally governed as a first-class concern. Without a dedicated determinism constitution, replay is unreliable, observability loses consistency, lineage loses trustworthiness, testing becomes flaky, and runtime behavior becomes environment-dependent. A dedicated determinism constitution is needed to make deterministic behavior explicit, enforceable, and governance-driven.

## What Changes

- Create a constitutional **Determinism Constitution** spec (`specs/determinism-constitution/spec.md`) that defines deterministic-by-default behavior, forbidden nondeterminism, deterministic capability mediation, replay equivalence, deterministic state behavior, deterministic testing, governance enforcement, and deterministic observability
- Amend the **Project Constitution** to centralize the existing `Deterministic-first behavior` requirement into a cross-reference to the Determinism Constitution
- Amend the **Runtime Abstraction** spec to cross-reference the Determinism Axiom as governed by the Determinism Constitution
- Amend the **Testing Governance** spec to incorporate deterministic testing expectations from the Determinism Constitution

## Capabilities

### New Capabilities
- `determinism-constitution`: Constitutional governance for deterministic behavior across the ego-rs platform. Defines deterministic-by-default, forbidden nondeterminism, capability mediation, replay equivalence, deterministic state, deterministic testing, governance enforcement, and observability semantics.

### Modified Capabilities
- `project-constitution`: Replace/extend the existing `Deterministic-first behavior` requirement (Requirement 1) to cross-reference the Determinism Constitution as the governing spec. Preserve the original requirement as a constitutional invariant, supplemented by the dedicated spec.
- `runtime-abstraction`: Cross-reference the Determinism Constitution from the `Determinism Axiom` constitutional invariant. The axiom remains in the runtime spec but its governance is constitutionalized.
- `testing-governance`: Add deterministic testing expectations (no wall-clock timing, no hidden randomness, no unstable concurrency timing, flaky test rejection) as spec-level requirements.

## Impact

- `openspec/specs/`: New `determinism-constitution/` spec directory with `spec.md`
- `openspec/specs/project-constitution/spec.md`: Amendment to cross-reference determinism constitution
- `openspec/specs/runtime-abstraction/spec.md`: Amendment to cross-reference determinism constitution
- `openspec/specs/testing-governance/spec.md`: Amendment to add deterministic testing requirements
- No runtime code, no infrastructure changes, no library or framework changes — this is a constitutional and governance-only change
