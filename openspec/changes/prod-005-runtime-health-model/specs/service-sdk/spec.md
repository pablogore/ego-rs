# Delta for service-sdk

## ADDED Requirements

### Requirement: Lifecycle Registration Supports Health Contributors

`LifecycleManaged` components (or an equivalent lifecycle-integrated
registration point) MUST be able to register a `HealthContributor` with the
runtime's health aggregator during initialization. Registration MUST
integrate with the existing lifecycle without requiring a component to
manage its own separate health-reporting channel.

The runtime is the single registration authority. Lifecycle components MAY
contribute contributors via this lifecycle-integrated registration point;
runtime-owned facilities (e.g. registered data providers) are adapted and
registered by the runtime/builder during the same construction phase. No
subsystem registers directly against a mutable global aggregator — registration
flows through the one runtime authority, not a second competing channel.

#### Scenario: A lifecycle-managed component registers a contributor during initialize
- GIVEN a component implementing lifecycle management that also registers a
  `HealthContributor`
- WHEN the runtime initializes that component
- THEN its contributor becomes part of the aggregator's registered set,
  reachable by subsequent readiness aggregation

#### Scenario: A component with no health contributor is unaffected
- GIVEN a lifecycle-managed component that registers no `HealthContributor`
- WHEN the runtime initializes and later aggregates readiness
- THEN readiness aggregation is unaffected by that component's absence of a
  contributor — registration remains optional per component

#### Scenario: Runtime-owned providers register through the same authority, not a parallel channel
- GIVEN registered data providers and a lifecycle-managed component that each
  contribute health contributors
- WHEN the runtime is constructed
- THEN both are registered by the runtime construction phase into the single
  aggregator — neither registers directly against a mutable global aggregator as
  a separate authority

### Requirement: The Runtime Owns the Single Global Aggregator

Exactly one health/readiness aggregator MUST exist per runtime instance.
`service-sdk`/runtime MUST own the concurrent execution (fan-out,
per-contributor timeout, optional global budget) and the deterministic
folding of contributor reports into one global report; a subsystem MUST NOT
run its own separate aggregation loop as an alternative source of readiness
truth.

#### Scenario: All registered contributors are aggregated by one runtime-owned aggregator
- GIVEN contributors registered from multiple subsystems (e.g. providers,
  and any other lifecycle-managed component)
- WHEN readiness is computed
- THEN all of them are folded by the same single runtime-owned aggregator
  into one global report

#### Scenario: No subsystem runs a competing aggregation loop
- GIVEN the runtime after this change
- WHEN its subsystems are inspected for readiness aggregation logic
- THEN only the runtime-owned aggregator performs contributor fan-out and
  folding — no subsystem duplicates this responsibility
