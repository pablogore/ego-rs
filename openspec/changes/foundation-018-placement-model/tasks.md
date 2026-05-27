## 1. Spec Verification

- [ ] 1.1 Verify every requirement has at least one WHEN/THEN scenario
- [ ] 1.2 Verify all normative language uses SHALL/MUST/MUST NOT (no should/may for normative requirements)
- [ ] 1.3 Verify the spec contains no scheduler, actor, cluster, placement engine, orchestration engine, topology, Kubernetes, transport, or runtime prescriptions
- [ ] 1.4 Verify every requirement listed in `Requirement coverage completeness` exists as a first-class requirement in the spec

## 2. Delta Spec Verification

- [ ] 2.1 Verify `specs/behavior-model/spec.md` correctly establishes the Placement Model authority boundary without modifying existing Behavior Model requirements
- [ ] 2.2 Verify `specs/projection-model/spec.md` correctly establishes the Placement Model authority boundary without modifying existing Projection Model requirements
- [ ] 2.3 Verify `specs/persistence-model/spec.md` correctly establishes the Placement Model authority boundary without modifying existing Persistence Model requirements
- [ ] 2.4 Verify `specs/runtime-abstraction/spec.md` correctly establishes the Placement Model authority boundary without modifying existing Runtime Abstraction requirements
- [ ] 2.5 Verify `specs/architecture-governance/spec.md` correctly establishes placement behavior governance across architectural boundaries without modifying existing Architecture Governance requirements
- [ ] 2.6 Verify each delta spec has at least one WHEN/THEN scenario
- [ ] 2.7 Verify each delta spec preserves authority ownership without modifying canonical behavior requirements
- [ ] 2.8 Verify Placement Model authority boundary exists for behavior-model and remains non-overlapping
- [ ] 2.9 Verify Placement Model authority boundary exists for projection-model and remains non-overlapping
- [ ] 2.10 Verify Placement Model authority boundary exists for persistence-model and remains non-overlapping
- [ ] 2.11 Verify Placement Model authority boundary exists for runtime-abstraction and remains non-overlapping
- [ ] 2.12 Verify placement behavior across architectural boundaries exists for architecture-governance and preserves Architecture Governance ownership

## 3. Governance Review

- [ ] 3.1 Verify no existing spec conflicts with placement governance — review Service Contract Model, Transport Binding Model, Interaction Model, Behavior Model, Projection Model, Persistence Model, Runtime Abstraction, Determinism Constitution, Architecture Governance, Dependency Governance Constitution
- [ ] 3.2 Verify authority ownership is explicit and non-overlapping across all constitutional specs
- [ ] 3.3 Verify WHAT / HOW exposed / HOW interact / HOW execute / HOW materialize / HOW durable truth / HOW ownership-in-space separation remains explicit and unambiguous
- [ ] 3.4 Verify replay-safe ownership expectations — replay equivalence and divergence governance are explicit and testable
- [ ] 3.5 Verify locality trustworthiness governance — locality semantics preserve deterministic interpretation, replay-safe locality, and governed ownership locality
- [ ] 3.6 Verify implementation neutrality — the spec contains no framework, runtime, cluster, scheduler, topology, or technology-specific prescriptions
- [ ] 3.7 Verify freeze-readiness — the spec is complete, unambiguous, and requires no additional requirements before freezing
- [ ] 3.8 Verify Placement Model remains constitutional, deterministic, replay-safe, fail-closed, implementation-neutral, runtime-neutral, scheduler-neutral, cluster-neutral, topology-neutral, and authority-complete
