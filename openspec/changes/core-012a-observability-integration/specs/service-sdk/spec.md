# Delta for service-sdk

Scope: CORE-012A. Wires the three reachable macro-guard denials (`MissingContext`, `TenantMismatch`, `AuthorizationDenied`) through the existing `Observability` port. Additive only — guard order, denial semantics, and tenant/authorization invariants established elsewhere are unchanged.

## ADDED Requirements

### Requirement: Reachable Macro-Guard Denials Are Recorded

Each denied invocation of a `#[authorize]` and/or `#[tenant_scoped]` guarded operation MUST produce exactly one recorded `Observability` event for the denial that occurred. Because guard evaluation short-circuits on the first denial, at most one of `MissingContext`, `TenantMismatch`, or `AuthorizationDenied` MUST ever be recorded for a single denied call, regardless of how many guard attributes are present.

#### Scenario: A single-guard denial records one event
- GIVEN an operation guarded only by `#[authorize]`
- WHEN the invocation is denied with `AuthorizationDenied`
- THEN exactly one event is recorded, reporting `AuthorizationDenied`

#### Scenario: A denial with both attributes present still records exactly one event
- GIVEN an operation guarded by both `#[authorize]` and `#[tenant_scoped]`
- WHEN the invocation is denied because authorization fails
- THEN exactly one event is recorded (`AuthorizationDenied`), and no second tenant-related event is recorded for that call

#### Scenario: Allowed invocations record no denial event
- GIVEN an operation guarded by `#[authorize]` and `#[tenant_scoped]`
- WHEN the invocation passes both guards
- THEN no denial event is recorded, and existing denial semantics and guard order are unaffected

### Requirement: Minimum Recorded Event Contract

Every recorded denial event MUST contain at minimum: denial kind, service name, and operation name. Additional contextual fields (e.g. correlation id, actor id, tenant identifier, metadata) are optional; their absence MUST NOT fail this contract.

#### Scenario: A minimal event with only the three required fields satisfies the contract
- GIVEN a denied invocation is recorded with only denial kind, service name, and operation name populated
- WHEN the recorded event is checked against this contract
- THEN the event satisfies the contract

#### Scenario: A missing required field violates the contract
- GIVEN a recorded event for a denied invocation
- WHEN denial kind, service name, or operation name is absent
- THEN the event does not satisfy this contract

### Requirement: Recorded Denial Data Is Redacted

Recorded denial event data MUST NOT expose raw tenant identifiers or denial-reason strings in its recorded/`Display`-safe form, following the same `Display`/`Debug` split already established by the `SecurityError` convention (`security-sdk/src/error/mod.rs:47-75`). Full diagnostic detail MUST remain available only via `Debug`, never via the recorded or `Display`-safe form.

#### Scenario: Recorded event omits raw tenant id and denial reason
- GIVEN a `TenantMismatch` denial carrying a specific tenant id and mismatch reason
- WHEN the resulting event is observed in its recorded/`Display`-safe form
- THEN neither the raw tenant id nor the denial-reason string appears

#### Scenario: Full diagnostic detail remains available via the original error's Debug only
- GIVEN the same denied invocation, which independently produces and returns a `SecurityError::TenantMismatch` to the caller per the pre-existing AD-010 convention
- WHEN that returned error value is inspected via `Debug`
- THEN the raw tenant id and denial reason are present there, for internal diagnostics only — this change does not need to duplicate that detail into the recorded event's own representation to satisfy this requirement

### Requirement: Runtime Accepts an Observability Implementor, Default Behavior Unchanged

`RuntimeBuilder` MUST expose `with_observability(...)` allowing callers to supply an `Observability` implementor at build time. When it is not called, the runtime MUST keep no observability sink configured (`None`); denial recording MUST behave as a silent no-op, with return values, errors, guard ordering, and panic behavior identical to the runtime's behavior before this change.

#### Scenario: Supplying an Observability implementor is accepted at build time
- GIVEN a `RuntimeBuilder` configured with `.with_observability(some_implementor)`
- WHEN the runtime is built and a guarded operation is invoked
- THEN the build succeeds and denial recording uses the supplied implementor

#### Scenario: Omitting with_observability preserves today's behavior exactly
- GIVEN a `RuntimeBuilder` on which `.with_observability(...)` is never called
- WHEN the runtime is built and any guarded operation (allowed or denied) is invoked
- THEN behavior is identical to before this change — same return values, same errors, no new panics — with no sink configured, so denial recording is a silent no-op

### Requirement: CrossTenantDenied Remains Uninstrumented By Design

`CrossTenantDenied` MUST NOT be instrumented by this change. This is a deliberate deferral, not an oversight: no macro-reachable call path exists today that can produce a `CrossTenantDenied` outcome, so leaving it uninstrumented affects no caller-observable behavior and creates no regression when a future change adds such a path.

#### Scenario: No reachable path emits a CrossTenantDenied event today
- GIVEN the current set of macro-guarded operations reachable through `#[authorize]` and `#[tenant_scoped]`
- WHEN any of them is invoked, allowed or denied
- THEN no `CrossTenantDenied` event is ever produced, because no macro-reachable caller can trigger this outcome

#### Scenario: A future CrossTenantDenied caller does not conflict with this change
- GIVEN this change ships with `CrossTenantDenied` uninstrumented
- WHEN a future change introduces a macro-reachable path producing `CrossTenantDenied`
- THEN that future change may add instrumentation without contradicting or requiring rework of any requirement in this spec
