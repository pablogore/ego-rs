## 1. Spec Creation

- [ ] 1.1 Create `specs/determinism-constitution/spec.md` with all ADDED requirements (deterministic-by-default, forbidden nondeterminism, capability mediation, replay equivalence, deterministic state, deterministic testing, governance enforcement, deterministic observability)
- [ ] 1.2 Verify every requirement has at least one scenario in WHEN/THEN format
- [ ] 1.3 Verify all normative language uses SHALL/MUST/MUST NOT
- [ ] 1.4 Verify the spec contains no implementation code, tooling prescriptions, or framework references

## 2. Spec Amendments

- [ ] 2.1 Create delta spec for `project-constitution` — modify `Deterministic-first behavior` requirement to cross-reference the Determinism Constitution
- [ ] 2.2 Create delta spec for `runtime-abstraction` — modify `Determinism Axiom` requirement to cross-reference the Determinism Constitution
- [ ] 2.3 Create delta spec for `testing-governance` — add `Deterministic testing expectations` requirement referencing the Determinism Constitution

## 3. Governance Review

- [ ] 3.1 Verify no existing spec conflicts with the new determinism governance
- [ ] 3.2 Verify constitutional severity classifications are documented and enforceable
- [ ] 3.3 Verify cross-references use correct spec names and paths
- [ ] 3.4 Verify the spec remains implementation-agnostic and freeze-ready
