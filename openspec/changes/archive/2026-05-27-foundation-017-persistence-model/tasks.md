## 1. Spec Verification

- [ ] 1.1 Verify every requirement has at least one WHEN/THEN scenario
- [ ] 1.2 Verify all normative language uses SHALL/MUST/MUST NOT (no should/may for normative requirements)
- [ ] 1.3 Verify the spec contains no storage engine, database, repository, scheduler, transaction manager, replication engine, streaming engine, or runtime prescriptions
- [ ] 1.4 Verify every requirement listed in `Requirement coverage completeness` exists as a first-class requirement in the spec

## 2. Delta Spec Verification

- [ ] 2.1 Verify `specs/projection-model/spec.md` correctly establishes the Persistence Model authority boundary without modifying existing Projection Model requirements
- [ ] 2.2 Verify `specs/runtime-abstraction/spec.md` correctly establishes the Persistence Model authority boundary without modifying existing Runtime Abstraction requirements
- [ ] 2.3 Verify `specs/architecture-governance/spec.md` correctly establishes persistence behavior governance across architectural boundaries without modifying existing Architecture Governance requirements
- [ ] 2.4 Verify each delta spec has at least one WHEN/THEN scenario
- [ ] 2.5 Verify Projection Model authority boundary requirement exists and remains explicit and non-overlapping
- [ ] 2.6 Verify Runtime Abstraction authority boundary requirement exists and remains explicit and non-overlapping
- [ ] 2.7 Verify persistence behavior across architectural boundaries requirement exists and preserves Architecture Governance ownership
- [ ] 2.8 Verify each delta spec preserves authority ownership without modifying canonical behavior requirements

## 3. Governance Review

- [ ] 3.1 Verify no existing spec conflicts with persistence governance — review Service Contract Model, Transport Binding Model, Interaction Model, Behavior Model, Projection Model, Runtime Abstraction, Determinism Constitution, Architecture Governance, Dependency Governance Constitution
- [ ] 3.2 Verify authority ownership is explicit and non-overlapping across all constitutional specs
- [ ] 3.3 Verify WHAT / HOW exposed / HOW interact / HOW execute / HOW materialize / HOW durable truth separation remains explicit and unambiguous
- [ ] 3.4 Verify replay safety expectations — replay equivalence and divergence governance are explicit and testable
- [ ] 3.5 Verify lineage trustworthiness governance — lineage semantics preserve deterministic interpretation, replay trustworthiness, restoration trustworthiness, and governed causality
- [ ] 3.6 Verify implementation neutrality — the spec contains no framework, runtime, storage, database, or technology-specific prescriptions
- [ ] 3.7 Verify freeze-readiness — the spec is complete, unambiguous, and requires no additional requirements before freezing
