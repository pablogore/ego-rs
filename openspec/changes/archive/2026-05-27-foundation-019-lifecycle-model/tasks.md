## 1. Spec Verification

- [ ] 1.1 Verify every requirement has at least one WHEN/THEN scenario
- [ ] 1.2 Verify all normative language uses SHALL/MUST/MUST NOT (no should/may for normative requirements)
- [ ] 1.3 Verify the spec contains no scheduler, actor, cluster, orchestration engine, supervision system, workflow engine, runtime framework, topology, Kubernetes, transport, or runtime prescriptions
- [ ] 1.4 Verify every requirement listed in `Requirement coverage completeness` exists as a first-class requirement in the spec
- [ ] 1.5 Verify lifecycle semantics remain deterministic, replay-safe, fail-closed, implementation-neutral, and runtime-neutral

## 2. Delta Spec Verification

- [ ] 2.1 Verify `specs/behavior-model/spec.md` correctly establishes the Lifecycle Model authority boundary without modifying existing Behavior Model requirements
- [ ] 2.2 Verify `specs/projection-model/spec.md` correctly establishes the Lifecycle Model authority boundary without modifying existing Projection Model requirements
- [ ] 2.3 Verify `specs/persistence-model/spec.md` correctly establishes the Lifecycle Model authority boundary without modifying existing Persistence Model requirements
- [ ] 2.4 Verify `specs/placement-model/spec.md` correctly establishes the Lifecycle Model authority boundary without modifying existing Placement Model requirements
- [ ] 2.5 Verify `specs/runtime-abstraction/spec.md` correctly establishes the Lifecycle Model authority boundary without modifying existing Runtime Abstraction requirements
- [ ] 2.6 Verify `specs/architecture-governance/spec.md` correctly establishes lifecycle behavior governance across architectural boundaries without modifying existing Architecture Governance requirements
- [ ] 2.7 Verify each delta spec has at least one WHEN/THEN scenario
- [ ] 2.8 Verify each delta spec preserves authority ownership without modifying canonical behavior requirements
- [ ] 2.9 Verify Lifecycle Model authority boundary exists for behavior-model and remains non-overlapping
- [ ] 2.10 Verify Lifecycle Model authority boundary exists for projection-model and remains non-overlapping
- [ ] 2.11 Verify Lifecycle Model authority boundary exists for persistence-model and remains non-overlapping
- [ ] 2.12 Verify Lifecycle Model authority boundary exists for placement-model and remains non-overlapping
- [ ] 2.13 Verify Lifecycle Model authority boundary exists for runtime-abstraction and remains non-overlapping
- [ ] 2.14 Verify lifecycle behavior across architectural boundaries exists for architecture-governance and preserves Architecture Governance ownership
- [ ] 2.15 Verify Behavior Model no longer owns lifecycle semantics
- [ ] 2.16 Verify Lifecycle Model remains sole authority for lifecycle evolution semantics
- [ ] 2.17 Verify canonical FOUNDATION-015 Behavior Model wording is harmonized at archive time to remove lifecycle semantics from Behavior Model authority

## 3. Governance Review

- [ ] 3.1 Verify no existing spec conflicts with lifecycle governance — review Service Contract Model, Transport Binding Model, Interaction Model, Behavior Model, Projection Model, Persistence Model, Placement Model, Runtime Abstraction, Determinism Constitution, Architecture Governance, Dependency Governance Constitution
- [ ] 3.2 Verify authority ownership is explicit and non-overlapping across all constitutional specs
- [ ] 3.3 Verify WHAT / HOW exposed / HOW interact / HOW execute / HOW materialize / HOW durable truth / HOW ownership-in-space / HOW lifecycle evolution separation remains explicit and unambiguous
- [ ] 3.4 Verify replay-safe lifecycle expectations — replay equivalence and divergence governance are explicit and testable
- [ ] 3.5 Verify lifecycle trustworthiness governance — lifecycle semantics preserve deterministic interpretation, replay-safe lifecycle, and governed lifecycle evolution
- [ ] 3.6 Verify implementation neutrality — the spec contains no framework, runtime, cluster, scheduler, orchestration, supervision, or technology-specific prescriptions
- [ ] 3.7 Verify freeze-readiness — the spec is complete, unambiguous, and requires no additional requirements before freezing
- [ ] 3.8 Verify Lifecycle Model remains constitutional, deterministic, replay-safe, fail-closed, implementation-neutral, runtime-neutral, scheduler-neutral, orchestration-neutral, supervision-neutral, and authority-complete
- [ ] 3.9 Verify lifecycle evolution semantics do not overlap execution semantics
- [ ] 3.10 Verify lifecycle recovery semantics do not overlap persistence restoration semantics
- [ ] 3.11 Verify lifecycle governance remains implementation-neutral, runtime-neutral, scheduler-neutral, orchestration-neutral, supervision-neutral
- [ ] 3.12 Verify lifecycle authority ownership remains explicit and non-overlapping
- [ ] 3.13 Verify Lifecycle Model remains freeze-ready
- [ ] 3.14 Verify canonical Behavior Model no longer owns lifecycle semantics
- [ ] 3.15 Verify lifecycle semantics remain execution-neutral
- [ ] 3.16 Verify lifecycle recovery semantics remain distinct from persistence restoration semantics
- [ ] 3.17 Verify Lifecycle Model remains runtime-neutral
- [ ] 3.18 Verify Lifecycle Model remains placement-neutral
- [ ] 3.19 Verify constitutional ownership chain remains terminal and explicit
- [ ] 3.20 Verify Lifecycle Model remains freeze-ready
