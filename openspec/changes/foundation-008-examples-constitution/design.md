## Context

ego-rs has no constitutional governance for examples. There is no mandatory policy requiring examples for new capabilities, no architectural compliance validation for example code, no testing or documentation standards, and no canonical repository structure. This means onboarding depends on ad hoc demos, architectural drift can pass undetected through example code, and the platform lacks executable, maintained demonstrations of its own capabilities.

The existing specs define constitutional requirements for project governance, testing, architecture, and runtime abstraction — but none of these apply to examples. This design defines how to extend that constitutional framework to cover examples without creating a separate, disconnected policy layer.

## Goals / Non-Goals

**Goals:**

- Define a constitutional **Examples Constitution** spec with mandatory policies, categories, architecture compliance, testing, documentation, repository structure, and CI governance
- Extend existing specs (Project Constitution, Testing Governance, Architecture Governance, Runtime Abstraction) to cover examples
- Define canonical `examples/` directory structure with mandatory categories
- Ensure examples evolve with the specs they demonstrate — breaking a spec without updating its example SHALL be treated as an incomplete change

**Non-Goals:**

- Designing or implementing any concrete example
- Defining example content or behavior — only governance and structure
- Creating example directories or files — only the constitutional rules that require them
- Changing the production code architecture, testing, or runtime specs — only extending their scope to examples

## Decisions

1. **Constitutional spec over standalone document** — A constitutional spec under `specs/examples-constitution/` ensures governance is versioned, reviewable, and enforceable through the existing OpenSpec process. This aligns with the `specs/` convention and the OpenSpec-driven development requirement in the Project Constitution.

2. **Amend existing specs instead of duplicating** — Example-specific requirements that extend existing governance (testing, architecture) are added as delta requirements to the existing specs rather than duplicating them in the Examples Constitution. The Examples Constitution defines mandatory policy, categories, structure, and CI governance; the amended specs extend their scope.

3. **Canonical directory structure under `examples/`** — Top-level `examples/` with mandatory category subdirectories. This avoids scattered example locations and makes validation deterministic.

4. **CI validation as governance gates** — Deterministic unit validation blocks pull requests. Integration validation runs separately and blocks releases. This mirrors the production CI model.

5. **Versioning tied to spec changes** — Examples are versioned by the spec they demonstrate. When a spec requirement changes, the corresponding example MUST be updated in the same change. There is no separate example versioning.

6. **Determinism requirement applies to examples** — Examples SHALL be deterministic, same as production code. Examples MUST NOT depend on wall-clock time, random values, or external services unless those are explicitly injected through port parameters.

## Risks / Trade-offs

- **[Risk] Example maintenance burden** — Requiring examples for every capability creates ongoing maintenance cost. **Mitigation**: Examples ARE the onboarding and demonstration surface — they replace ad hoc demos and documentation that would otherwise rot. The cost is not new; it is redirected.
- **[Risk] CI execution time** — Running examples in CI increases pipeline duration. **Mitigation**: Deterministic examples run in the primary pipeline alongside unit tests. Integration examples run in a separate validation stage.
- **[Risk] Over-specification** — Too many mandatory categories could discourage contributors. **Mitigation**: Categories are minimal and aligned with platform architecture layers. New categories require a constitutional amendment.
- **[Risk] Example drift from reality** — Examples might become toy demos disconnected from production architecture. **Mitigation**: Architecture validation applies identically to all code. The Executable Documentation requirement reinforces the teaching purpose.
