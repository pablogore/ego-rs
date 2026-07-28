# Delta for runtime-health-model

The `runtime-health-model` capability (introduced by PROD-005, merged to develop)
defines the transport-agnostic liveness/readiness/startup model, the
`HealthContributor` contract, and the single runtime-owned aggregator with its
single registration authority. This delta ADDS the read-side as a participant in
that single model: a read-side `HealthContributor` that contributes to readiness
and startup only — never liveness — reusing the existing aggregation and
registration mechanism without introducing a parallel model.

This delta targets `runtime-health-model` rather than `service-sdk` because it
adds a new PARTICIPANT to the health model (with read-side-specific status
semantics), mirroring how PROD-005's own `external-data-providers` delta made the
provider subsystem a contributor. The registration plumbing itself
(single-authority, one aggregator) is already specified by PROD-005's
`service-sdk` delta and is reused UNCHANGED, so no new `service-sdk` requirement
is warranted.

## ADDED Requirements

### Requirement: The Read-Side Participates as a Health Contributor

The read-side MUST participate in the single runtime health model as a
`HealthContributor`, reporting a `(HealthStatus, DependencyRequirement)` derived
from its lag and time-since-last-progress. It MUST register through the same
single registration authority the runtime already uses for other contributors
(e.g. the provider contributor) — NOT through a parallel read-side readiness
model. Its `check()` MUST be probe-independent: it produces the same result
regardless of whether readiness or startup is aggregating, and MUST NOT branch on
the probe. A runtime with no read-side registered MUST aggregate exactly as
before (an empty read-side contributes nothing).

#### Scenario: A registered read-side projection is folded into the global report
- GIVEN a read-side projection registered as a `HealthContributor` through the
  runtime's single registration authority
- WHEN readiness is aggregated
- THEN its `(HealthStatus, DependencyRequirement)` is folded into the one global
  readiness report alongside every other contributor

#### Scenario: The read-side does not create a parallel readiness model
- GIVEN the runtime after this change
- WHEN read-side health is queried
- THEN it is observable only through its `HealthContributor` registration in the
  single global model, not through a read-side-specific parallel readiness surface

#### Scenario: No read-side registered leaves aggregation unchanged
- GIVEN a runtime with no read-side projection registered
- WHEN readiness is aggregated
- THEN the global report is exactly what it would be without this change — the
  absent read-side contributes nothing

### Requirement: Read-Side Health Never Affects Liveness

The read-side `HealthContributor` MUST affect readiness and startup only, and
MUST NOT be able to change the liveness result. A lagging or stalled projection
MUST remove the instance from readiness rotation but MUST NOT fail liveness —
conflating the two would turn a downstream-lag condition into a process restart.
This MUST hold structurally: the contributor exposes only the model's `check()`
method (which liveness never invokes), so the read-side cannot enter the liveness
path even in principle.

#### Scenario: A stalled read-side does not fail liveness
- GIVEN a Required read-side projection whose `check()` resolves to Unhealthy
  because it is behind and stalled
- WHEN liveness is computed
- THEN liveness is unaffected — no contributor, read-side included, was consulted

#### Scenario: The same stalled read-side does fail readiness
- GIVEN that same Required stalled read-side projection
- WHEN readiness is aggregated
- THEN the global readiness report is Unhealthy, removing the instance from
  rotation without restarting it

### Requirement: Read-Side Status Folds by Requirement Like Any Contributor

The read-side contributor's status MUST fold through the model's existing
deterministic rules from its `(HealthStatus, DependencyRequirement)` pair. A
Required read-side projection reporting Unhealthy MUST make global readiness
Unhealthy; an Optional read-side projection reporting Unhealthy MUST NOT force
global Unhealthy and MUST instead surface as Degraded. A read-side projection
that is lagging but still progressing MUST report Degraded, and a read-side
projection still performing initial replay MUST report Unhealthy with the
initialization-pending code, distinguishable from a real dependency failure at
the same status. The read-side MUST NOT introduce a new health status or a new
health code.

#### Scenario: A Required stalled read-side forces global Unhealthy
- GIVEN a Required read-side projection reporting Unhealthy (behind and stalled)
- WHEN readiness is aggregated
- THEN the global readiness report is Unhealthy

#### Scenario: An Optional stalled read-side degrades but does not fail
- GIVEN an Optional read-side projection reporting Unhealthy, with no Required
  contributor reporting Unhealthy
- WHEN readiness is aggregated
- THEN the global readiness report is Degraded, not Unhealthy

#### Scenario: A lagging-but-progressing read-side is Degraded
- GIVEN a read-side projection whose lag exceeds its threshold but which is still
  progressing within its stall deadline
- WHEN its contribution is folded
- THEN it contributes Degraded, not Unhealthy

#### Scenario: Initial replay is distinguishable from a dependency failure
- GIVEN a Required read-side projection still performing initial replay,
  reporting Unhealthy with the initialization-pending code
- WHEN startup is aggregated
- THEN the global report is Unhealthy and the read-side contributor's reason code
  is initialization-pending — distinct from a Required read-side reporting a
  dependency-failure code at the same Unhealthy status
