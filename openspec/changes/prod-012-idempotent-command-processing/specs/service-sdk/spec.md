# Delta for Service SDK

## ADDED Requirements

### Requirement: ServiceContext Exposes the Operation Key

`ServiceContext` MUST expose an accessor for the `OperationKey` established
at ingress for the current request, following the same explicit-ownership
model as its other accessors — no ambient lookup.

#### Scenario: Handler reads the operation key from context
- GIVEN a `ServiceContext` carrying an `OperationKey` established at ingress
- WHEN a handler calls the context's operation-key accessor
- THEN it receives the identical `OperationKey`, with no ambient or global
  lookup involved

### Requirement: RuntimeBuilder Registers the Reservation Store, Fail-Closed

`RuntimeBuilder` MUST support registering exactly one
`OperationReservationStore` implementation. When `IdempotencyEnforcementMode`
resolves to its enforcing (default) variant and no `OperationReservationStore`
is registered, `build()`/`try_build()` MUST fail rather than start a runtime
that cannot honor the mandatory-key guarantee.

#### Scenario: Startup fails closed when enforcement is on with no store registered
- GIVEN a `RuntimeBuilder` with the default (enforcing)
  `IdempotencyEnforcementMode` and no `OperationReservationStore` registered
- WHEN the runtime is built
- THEN build fails, naming the missing registration — no runtime starts

#### Scenario: Registered store enables a successful build under enforcement
- GIVEN a `RuntimeBuilder` with an `OperationReservationStore` registered and
  the default enforcing mode
- WHEN the runtime is built
- THEN build succeeds

### Requirement: RuntimeBuilder Registers a Single Injectable Clock

`RuntimeBuilder` MUST support registering a `Clock` (generalized out of the
existing auth `Clock`), defaulting to a system-clock implementation. The
registered `Clock` MUST be the sole time source injected into both the
`OperationReservationStore` and `EffectDedupStore` — neither MUST call
`Utc::now()` directly.

#### Scenario: A custom Clock is observed identically by both stores
- GIVEN a `RuntimeBuilder` registered with a deterministic test `Clock`
- WHEN the reservation store and `EffectDedupStore` each read the current
  time
- THEN both observe the identical injected `Clock`, with no direct
  `Utc::now()` call in either

### Requirement: RuntimeBuilder Registers Enforcement Mode and Retention Policy

`RuntimeBuilder` MUST support configuring `IdempotencyEnforcementMode` and a
retention policy (TTL for reservations/stored responses, purge batch size).
Omitting configuration MUST resolve to the fail-closed enforcement default.

#### Scenario: Default configuration is fail-closed
- GIVEN a `RuntimeBuilder` with no explicit `IdempotencyEnforcementMode` call
- WHEN the runtime is built
- THEN the effective mode is the fail-closed mandatory-key variant

### Requirement: Purge-Worker Lifecycle Follows Existing Ordering

The reservation-purge background worker MUST start and stop under the same
lifecycle ordering contract (CORE-017) that governs other runtime-owned
background work. Shutdown MUST NOT release or abandon an in-progress
reservation's lease merely because the worker is stopping — an
in-progress lease is only ever resolved through its own expiry/takeover
path, never through a shutdown-triggered release.

#### Scenario: Shutdown does not release in-progress leases
- GIVEN a reservation `InProgress` with an active lease at the moment of
  runtime shutdown
- WHEN the purge worker and runtime shut down
- THEN the lease is left untouched by shutdown; it is only resolved later by
  expiry and takeover, never released as a side effect of shutdown
