# Runtime Health Model Specification

## Purpose

Defines a transport-agnostic liveness/readiness/startup model for `ego-rs`:
domain-level value types and a `HealthContributor` contract (`ego-domain`,
zero infra deps), plus a service-sdk/runtime registry that executes
contributors concurrently, applies per-contributor timeouts, and folds their
`(status, requirement)` pairs into one deterministic global report. Adapters
(HTTP, gRPC, GraphQL, messaging, CLI/TUI, Kubernetes probes) are out of
scope — this spec fixes the model they will consume, not the surfaces
themselves.

## Requirements

### Requirement: Liveness Never Consults External Contributors

Liveness MUST report on process/runtime-internal health only. Computing
liveness MUST NOT invoke, poll, or await any `HealthContributor`, whether
`Required` or `Optional`. A failing external dependency (DB, broker,
provider, remote auth) MUST NOT be able to change the liveness result.

#### Scenario: External contributor failure does not affect liveness
- GIVEN a `Required` contributor whose check would resolve to `Unhealthy`
- WHEN liveness is computed
- THEN liveness is unaffected — no contributor was consulted

#### Scenario: Liveness computation invokes zero contributors
- GIVEN any set of registered contributors, `Required` or `Optional`
- WHEN liveness is computed
- THEN no contributor's check is invoked, structurally verifiable

### Requirement: Readiness Aggregates Registered Contributors

Readiness MUST fold every registered `HealthContributor`'s reported
`(HealthStatus, DependencyRequirement)` into a single global readiness
report, reflecting the current state of all registered contributors at
evaluation time.

#### Scenario: Readiness reflects all registered contributors
- GIVEN two or more registered contributors reporting different statuses
- WHEN readiness is computed
- THEN the global report reflects every registered contributor, none
  silently excluded

### Requirement: Aggregation Is Deterministic From (Status, Requirement)

The global readiness result MUST be a deterministic function of each
contributor's `(HealthStatus, DependencyRequirement)` pair. A `Required`
contributor reporting `Unhealthy` MUST make global readiness `Unhealthy`. An
`Optional` contributor reporting `Unhealthy` MUST NOT force global readiness
to `Unhealthy`; the aggregate SHOULD surface as `Degraded` instead. Given
the same set of contributor reports, the aggregator MUST always produce the
same global result. A contributor's check MUST be probe-independent: it produces
the same result regardless of which probe (readiness or startup) is aggregating,
and MUST NOT receive or branch on the probe kind. `HealthCode::InitializationPending`
MUST NOT change the lattice: the contributor reports `Unhealthy`,
`DependencyRequirement` drives `Unhealthy` vs `Degraded`, and the `HealthCode` is
preserved in the `ContributorReport`. The fold MUST be identical regardless of
probe.

#### Scenario: Required contributor Unhealthy forces global Unhealthy
- GIVEN a `Required` contributor reporting `HealthStatus::Unhealthy`
- WHEN readiness is aggregated
- THEN the global readiness report is `Unhealthy`

#### Scenario: Optional contributor Unhealthy degrades but does not fail
- GIVEN an `Optional` contributor reporting `HealthStatus::Unhealthy` and no
  `Required` contributor reporting `Unhealthy`
- WHEN readiness is aggregated
- THEN the global readiness report is `Degraded`, not `Unhealthy`

#### Scenario: Same inputs always produce the same aggregate
- GIVEN a fixed set of contributor `(status, requirement)` reports
- WHEN aggregation runs repeatedly against that same fixed set
- THEN the global report is identical every time

### Requirement: Contributors Are Executed Concurrently

Readiness aggregation MUST fan out to all registered contributors
concurrently. A single contributor's check MUST NOT block, delay, or serialize
the execution of any other contributor's check. Sequential polling MUST NOT
be the contract.

#### Scenario: A slow contributor does not delay others
- GIVEN one contributor whose check takes substantially longer than the
  others
- WHEN readiness is aggregated
- THEN the faster contributors' results are available without waiting for
  the slow contributor to finish sequentially

### Requirement: Per-Contributor Timeout Yields a Structured State

Each contributor check MUST be bounded by a per-contributor timeout. A
contributor whose check does not complete within its timeout MUST resolve
to a structured timeout code — never leave the aggregator hung or blocked.
An optional global aggregation budget MAY additionally bound the entire
aggregation.

#### Scenario: A hanging contributor times out with a structured result
- GIVEN a contributor whose check never completes within its configured
  timeout
- WHEN readiness is aggregated
- THEN that contributor's contribution resolves to a structured timeout
  code, and aggregation completes without hanging

#### Scenario: One timing-out contributor does not block the others
- GIVEN one contributor that times out and others that complete normally
- WHEN readiness is aggregated
- THEN the other contributors' results are included in the global report
  unaffected by the timeout

### Requirement: Public Health Codes Are a Closed, Structured Set

The public health contract MUST expose failure reasons only through a
structured, closed set of codes — never a free-text message. Internal
detail (log lines, traces, error causes) MAY exist but MUST NOT cross the
public report boundary. The exact enumerated code variants are a design
concern; this requirement only fixes that the set MUST be closed and
structured.

#### Scenario: A contributor failure never leaks free text to the public report
- GIVEN a contributor whose underlying failure carries an arbitrary
  internal error message
- WHEN its result is surfaced in the public health report
- THEN the report exposes a structured code from the closed set only, with
  no free-text message field

#### Scenario: The closed code set has no open/free-text variant
- GIVEN the public health-code type
- WHEN its variants are inspected
- THEN every variant is a fixed, structured value — none accepts or
  exposes an arbitrary string

### Requirement: The Model Is Transport-Neutral

A health capability defined here MUST NOT require or privilege any transport
or deployment mechanism (HTTP, gRPC, GraphQL, messaging, Kubernetes,
CLI/TUI, or any other). The public contract (contributor trait, report/value
types) MUST contain no transport-specific type or concept. Adapters consume
the same framework-level report and map it to their own protocol; none is
privileged over another.

#### Scenario: Domain and aggregation types carry no transport dependency
- GIVEN the public contributor, status, code, requirement, and report types
- WHEN their signatures are inspected
- THEN none references HTTP, gRPC, GraphQL, messaging, or Kubernetes types
  or concepts

#### Scenario: Two different adapters can consume the same report
- GIVEN one computed global health report
- WHEN two independent protocol adapters each map it to their own
  representation
- THEN both consume the identical report; neither requires an
  adapter-specific field from the model

### Requirement: Contributors Register Through Lifecycle

A `HealthContributor` MUST become known to the aggregator only through
explicit registration integrated with the runtime's lifecycle. Startup/
initialization state MUST be observable as a distinct condition from
steady-state readiness. This distinct condition MUST be carried by
`ProbeKind::Startup` together with `ContributorReport.code == InitializationPending`,
NOT by a different global `HealthStatus`. The contributor's check remains
probe-independent, and the fold is identical for readiness and startup:
`InitializationPending` does not alter the lattice — `DependencyRequirement`
alone drives `Unhealthy` vs `Degraded`, while the `HealthCode` preserved on the
`ContributorReport` distinguishes "still initializing" from a real dependency
failure.

#### Scenario: An unregistered contributor is never aggregated
- GIVEN a contributor implementation that exists but was never registered
- WHEN readiness is aggregated
- THEN it has no effect on the global report

#### Scenario: A required initializing contributor is Unhealthy with a pending code
- GIVEN a registered `Required` contributor still completing startup,
  reporting `HealthStatus::Unhealthy` with `HealthCode::InitializationPending`
- WHEN startup is aggregated
- THEN the global report status is `Unhealthy` and the contributor's
  `ContributorReport.code` is `InitializationPending`

#### Scenario: An optional initializing contributor is Degraded with a pending code
- GIVEN a registered `Optional` contributor still completing startup,
  reporting `HealthStatus::Unhealthy` with `HealthCode::InitializationPending`
- WHEN startup is aggregated
- THEN the global report status is `Degraded` and the contributor's
  `ContributorReport.code` is `InitializationPending`

#### Scenario: A pending code is distinguishable from a real dependency failure at the same status
- GIVEN a `Required` contributor reporting `Unhealthy` with
  `HealthCode::DependencyFailure` (a real failure, not initialization)
- WHEN startup is aggregated
- THEN the global report status is `Unhealthy` — the same status a required
  initializing contributor produces — but the `ContributorReport.code` is
  `DependencyFailure`, distinguishing it from `InitializationPending`

### Requirement: TestKit Provides Same-Contract Health Test Support

`testkit` MUST expose building blocks — implementing the same public
`HealthContributor` contract used in production — for constructing
deterministic contributors (fixed status/requirement/timeout behavior) in
tests, without requiring test code to fake the aggregator itself.

#### Scenario: A deterministic test contributor drives a known aggregation outcome
- GIVEN a `testkit`-provided contributor fixed to report `Optional`/`Unhealthy`
- WHEN it is registered and readiness is aggregated in a test
- THEN the global report deterministically reflects `Degraded`, matching
  production aggregation semantics exactly

### Requirement: Exactly One Framework-Level Health Model

There MUST be exactly one framework-level health/readiness model per
runtime instance. A subsystem (including the provider subsystem) MUST NOT
expose a parallel health/readiness model as an alternative source of truth
— every subsystem participates as one or more contributors to this single
model.

#### Scenario: A subsystem's health is only reachable through the single model
- GIVEN any subsystem that reports health (e.g. the provider subsystem)
- WHEN its health is queried
- THEN it is observable only via its `HealthContributor` registration in
  the single global model, not through a subsystem-specific parallel report
