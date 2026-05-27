## 1. Spec Verification

- [ ] 1.1 Verify every requirement in `specs/projection-model/spec.md` has at least one WHEN/THEN scenario
- [ ] 1.2 Verify all normative language uses SHALL/MUST/MUST NOT (no should/may for normative requirements)
- [ ] 1.3 Verify the spec contains no persistence engine, scheduler, queue, event store, streaming engine, orchestration, or runtime prescriptions
- [ ] 1.4 Verify every requirement listed in `Requirement coverage completeness` exists as a first-class requirement in the spec

## 2. Delta Spec Verification

- [ ] 2.1 Verify `specs/behavior-model/spec.md` correctly establishes the Projection Model authority boundary without modifying existing Behavior Model requirements
- [ ] 2.2 Verify `specs/runtime-abstraction/spec.md` correctly establishes the Projection Model authority boundary without modifying existing Runtime Abstraction requirements
- [ ] 2.3 Verify `specs/architecture-governance/spec.md` correctly establishes projection behavior governance across architectural boundaries without modifying existing Architecture Governance requirements
- [ ] 2.4 Verify each delta spec has at least one WHEN/THEN scenario

## 3. Governance Review

- [ ] 3.1 Verify no existing spec conflicts with projection governance — review Service Contract Model, Transport Binding Model, Interaction Model, Behavior Model, Runtime Abstraction, Determinism Constitution, Architecture Governance, Dependency Governance Constitution
- [ ] 3.2 Verify authority ownership is explicit and non-overlapping across all constitutional specs
- [ ] 3.3 Verify WHAT / HOW exposed / HOW interact / HOW execute / HOW materialize separation remains explicit and unambiguous
- [ ] 3.4 Verify requirement coverage completeness — every requirement listed in the requirement coverage completeness section has a corresponding requirement in the spec
- [ ] 3.5 Verify implementation neutrality — the spec contains no framework, runtime, persistence, or technology-specific prescriptions
- [ ] 3.6 Verify replay safety expectations — replay equivalence and divergence governance are explicit and testable
- [ ] 3.7 Verify freeze-readiness — the spec is complete, unambiguous, and requires no additional requirements before freezing
