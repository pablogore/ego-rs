# Delta for external-data-providers

## ADDED Requirements

### Requirement: Provider Health Participates As a Contributor, Not a Parallel Model

The provider subsystem's health/readiness signal (`ProviderHealth`,
`ProviderSubsystemReadiness`, `RuntimeDataProviderAccess::readiness()`) MUST
be exposed to the runtime as one or more `HealthContributor` registrations
in the single framework-level health model. The provider subsystem MUST NOT
expose a second, parallel readiness surface as an alternative consumption
path to the global model. The concrete migration mechanism (adapt existing
types, deprecate, or keep subsystem-internal wrapped by a contributor) is a
design decision; this requirement fixes only the observable outcome.

#### Scenario: Provider readiness reaches the global report through a contributor
- GIVEN one or more registered external data providers
- WHEN global readiness is aggregated
- THEN each provider's health is reflected in the single global report via
  its `HealthContributor` registration, not through a separate
  provider-only readiness call

#### Scenario: No duplicated readiness surface exists
- GIVEN the provider subsystem after this change
- WHEN its public surface is inspected for a readiness/health entry point
- THEN provider health is observable only through the single global model's
  contributor registration — no independent parallel readiness path remains

### Requirement: Provider Contributors Preserve No-Free-Text Semantics

Every provider `HealthContributor` MUST report status using the same
structured, closed code set as every other contributor — never a free-text
failure reason. This continues the no-free-text guarantee `ProviderHealth`
already established under #234/#235.

#### Scenario: A provider failure surfaces a structured code, not raw text
- GIVEN an external data provider whose underlying fetch fails with an
  arbitrary error message
- WHEN its `HealthContributor` check runs
- THEN the reported result carries a structured code from the closed set,
  with no free-text message crossing into the global report

### Requirement: Provider Health Checks Are Subject to Concurrent Aggregation and Timeout

Provider contributors MUST participate in the same concurrent fan-out and
per-contributor timeout the global aggregator applies to every contributor.
The previous sequential polling behavior of
`RuntimeDataProviderAccess::readiness()` MUST NOT be the contract providers
are held to going forward.

#### Scenario: Multiple provider contributors are checked concurrently, not sequentially
- GIVEN two or more registered provider contributors
- WHEN global readiness is aggregated
- THEN their checks execute concurrently as part of the aggregator's
  fan-out, not one after another in sequence

#### Scenario: A slow provider check times out with a structured code
- GIVEN a provider contributor whose check exceeds its configured
  per-contributor timeout
- WHEN readiness is aggregated
- THEN that contributor resolves to a structured timeout code and does not
  block aggregation of the other contributors
