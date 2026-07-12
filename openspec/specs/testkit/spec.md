# TestKit Specification

## Purpose

Defines the observable behavior TestKit MUST provide so consuming projects can
test ego.rs services against the same public contracts production code uses
(Runtime, Service SDK, Configuration, Logger, Authorization, Security Context,
Identity) instead of hand-rolled, per-project mocks. Every requirement below
describes WHAT a test author can rely on. It does not define types, traits,
function signatures, or module layout — that is the concern of design documents.

A cross-cutting principle applies to every requirement in this document:
**TestKit MUST NOT introduce a parallel or divergent implementation of a
production contract.** Anything TestKit hands to a test (a context, a
provider, a security context, an identity, a configuration source, a logger)
MUST satisfy the exact same public contract the corresponding production
component satisfies, so a test exercises real dispatch and real validation
logic, not a look-alike stand-in that can silently drift from production.

---

## Requirements

### Requirement: Consistent Service Execution

TestKit MUST provide a consistent way to execute a service under test through
the same public entry points a production caller uses. TestKit MUST NOT
require a test author to hand-wire Runtime internals to invoke a service.

#### Scenario: Execution surfaces production-shaped output

- GIVEN a service registered for execution through TestKit
- WHEN a test invokes that service
- THEN the returned success and error types are identical to what a production caller receives

#### Scenario: Two tests execute independently

- GIVEN two tests each executing a service under test through TestKit
- WHEN both run in the same test binary
- THEN neither execution's state or outcome affects the other

---

### Requirement: Testing ServiceContext

TestKit MUST provide a `ServiceContext` usable in tests that satisfies the
same `ServiceContext` contract production code depends on, without requiring
a live production dependency graph.

#### Scenario: Service under test runs unmodified

- GIVEN a service under test that depends on `ServiceContext`
- WHEN it is supplied a TestKit `ServiceContext`
- THEN the service runs without any test-specific fork of its code

#### Scenario: Context isolation between tests

- GIVEN two independent tests each constructing their own TestKit `ServiceContext`
- WHEN both run
- THEN state set in one context MUST NOT leak into the other

---

### Requirement: Testing AuthorizationProvider

TestKit MUST provide `AuthorizationProvider` implementations for tests that
can be configured to allow or deny a given decision deterministically,
without a real policy engine or external authorization service.

#### Scenario: Configured to allow

- GIVEN a TestKit `AuthorizationProvider` configured to allow a specific action
- WHEN a service under test evaluates authorization for that action
- THEN the authorization check succeeds

#### Scenario: Configured to deny

- GIVEN a TestKit `AuthorizationProvider` configured to deny a specific action
- WHEN a service under test evaluates authorization for that action
- THEN the check fails with the same error type production authorization failures use

---

### Requirement: SecurityContext Helpers

TestKit MUST provide a way to construct a valid `SecurityContext` for tests —
principal, claims, scopes — without executing a full authentication flow.

#### Scenario: Constructing an authenticated SecurityContext

- GIVEN a test that needs an authenticated caller with a specified principal and claims
- WHEN it uses a TestKit `SecurityContext` helper
- THEN the result is indistinguishable, to consuming code, from a `SecurityContext` a real `AuthenticationProvider` would produce

#### Scenario: Representing the unauthenticated case

- GIVEN a test that needs to exercise unauthenticated behavior
- WHEN it requests the TestKit unauthenticated helper
- THEN the result represents "no authenticated principal" the same way production code represents it

---

### Requirement: Identity Builders

TestKit MUST provide builders that produce valid identity/`Principal` values
with sensible defaults, letting a test override only the fields relevant to it.

#### Scenario: Default identity satisfies production invariants

- GIVEN a test calls an identity builder with no overrides
- WHEN the resulting identity is used
- THEN it satisfies every invariant the production identity type enforces

#### Scenario: Overriding a single field leaves others at default

- GIVEN a test calls an identity builder overriding only one field (e.g. roles)
- WHEN the resulting identity is inspected
- THEN that field matches the override and every other field retains its default

---

### Requirement: Test Configuration

TestKit MUST provide a way to supply configuration values to a service under
test without a real configuration source (files, env vars, remote config),
while satisfying the same configuration contract production code depends on.

#### Scenario: Provided value is observed by the service

- GIVEN a test sets a configuration value through TestKit
- WHEN the service under test reads that key through the production configuration contract
- THEN it observes the value the test provided

#### Scenario: Unset key behaves like production

- GIVEN a test does not set a given configuration key
- WHEN the service under test reads that key
- THEN the outcome (default value or explicit "not found") matches the documented behavior of the production configuration contract, never a panic

---

### Requirement: Capturable Logger

TestKit MUST provide a logger that satisfies the production logging contract
and additionally lets a test inspect emitted records after execution.

#### Scenario: Captured record matches what was logged

- GIVEN a service under test logs a message at a given level with structured fields
- WHEN a test inspects the TestKit logger's captured records afterward
- THEN the captured record has the same level, message, and structured fields

#### Scenario: No leakage between logger instances

- GIVEN two tests each using their own TestKit logger instance
- WHEN both run
- THEN records captured by one instance MUST NOT appear in the other's captured records

---

### Requirement: Reusable Fixtures

TestKit MUST provide fixtures for commonly needed test setups (e.g. a
pre-wired service under test with a default `ServiceContext`,
`SecurityContext`, and configuration) so a typical test does not assemble
these building blocks individually.

#### Scenario: Default fixture is immediately usable

- GIVEN a test requests a TestKit fixture with no customization
- WHEN the fixture is constructed
- THEN it yields a fully wired test setup with no further assembly required

#### Scenario: Overriding one building block leaves the rest at default

- GIVEN a test requests a fixture overriding a single building block (e.g. the `AuthorizationProvider`)
- WHEN the fixture is constructed
- THEN only the overridden building block differs; every other part retains its default

---

### Requirement: Assertion Helpers

TestKit MUST provide helpers for asserting common outcomes against ego.rs
contracts (e.g. that a call was authorized, that a specific production error
variant was returned) so tests express intent without reimplementing
comparison logic per project.

#### Scenario: Asserting an authorization outcome

- GIVEN a test invoked a service under test expecting authorization to succeed
- WHEN it uses a TestKit assertion helper to check that outcome
- THEN the helper passes when authorization succeeded and fails with a clear message otherwise

#### Scenario: Asserting a specific error variant

- GIVEN a test expects a specific production error variant
- WHEN it uses a TestKit assertion helper to check the result against that variant
- THEN the helper passes only when the actual result matches that variant, regardless of error message text

---

### Requirement: PrincipalBuilder keeps ergonomic string tenant input, validates at build time

`PrincipalBuilder::tenant()` MUST continue to accept `impl Into<String>` — test fixtures remain one-liner friendly and are not required to construct a `TenantId` themselves. `PrincipalBuilder::build()` MUST validate the supplied tenant string via `TenantId::new()` before attaching it to the produced `Principal`, mirroring the existing `SubjectId` validation pattern already present in `build()`.

If the tenant string fails validation (empty or whitespace-only after trim), `build()` MUST panic with a descriptive message, the same fail-fast-at-test-setup behavior `build()` already provides for an invalid subject id. A valid tenant string MUST produce a `Principal` whose `tenant_id` is `Some(TenantId)`.

(Previously: `tenant()` stored the raw string and `build()` passed it unvalidated to `Principal::with_tenant_id(impl Into<String>)`; an invalid test fixture tenant would compile and only fail later, deep inside `TenantResolver::resolve()`, if a test happened to exercise that path.)

#### Scenario: Valid tenant fixture builds successfully

- GIVEN `PrincipalBuilder::new().tenant("acme")`
- WHEN `.build()` is called
- THEN the returned `Principal.tenant_id` is `Some(TenantId)` whose value is `"acme"`

#### Scenario: Invalid tenant fixture panics at build time, not later

- GIVEN `PrincipalBuilder::new().tenant("")` (or a whitespace-only string)
- WHEN `.build()` is called
- THEN the call panics with a descriptive message identifying the invalid tenant — the failure happens at test setup, not inside code under test

**Tests**: `identity::tests::kind_tenant_and_attribute_overrides_are_applied` (existing, updated to assert `Some(TenantId)`), `identity::tests::empty_tenant_override_panics` (new), `identity::tests::whitespace_only_tenant_override_panics` (new).

---

### Requirement: TestKit Trait-Proxy Registration and Resolution Use the Canonical Production Path

`FixtureBuilder` MUST provide `with_service::<Tag>(self, svc: Arc<Tag::Service>) -> Result<Self, RegistryError>` and `ServiceTestFixture` MUST provide `resolve::<Tag>(&self) -> Result<Tag::Proxy, RuntimeError>`. Both MUST be pass-throughs to the identical `RuntimeBuilder::with_service`/`Runtime::resolve` this change adds to `service-sdk` — TestKit MUST NOT assemble a parallel or bespoke trait-proxy construction (no separate `InterceptorChain`/`Weak` wiring inside TestKit). This mirrors the existing, verified proof pattern for the `Injectable` DI path, where `FixtureBuilder::build()` calls the same `RuntimeInner::new_with_logger` constructor `RuntimeBuilder::build()` calls.

#### Scenario: FixtureBuilder registration reaches the same registry the production builder would populate
- GIVEN `FixtureBuilder::new().with_service::<HelloServiceTag>(Arc::new(HelloServiceImpl))?`
- WHEN the fixture builds its internal `Runtime`
- THEN the registered service is stored under the same `(Tag, version)` keying `RuntimeBuilder::with_service` would use — no separate TestKit-only registry exists

#### Scenario: ServiceTestFixture resolution yields the same generated proxy as production resolve()
- GIVEN a `ServiceTestFixture` built with `.with_service::<HelloServiceTag>(..)` registered
- WHEN `fixture.resolve::<HelloServiceTag>()` is called
- THEN `Ok(HelloServiceRef)` is returned, invoking the operation runs the identical guard order (authorize → tenant → interceptors → body) a production `rt.resolve::<HelloServiceTag>()` call would produce

#### Scenario: Resolving an unregistered tag from a test fixture fails the same way production does
- GIVEN a `ServiceTestFixture` with no registration for `OtherServiceTag`
- WHEN `fixture.resolve::<OtherServiceTag>()` is called
- THEN `Err(RuntimeError::ServiceNotFound)` is returned — the same error variant and condition `Runtime::resolve` returns

---

## Out of Scope

The following capabilities are explicitly deferred to future work:

- HTTP testing
- gRPC testing
- Database testing
- Testcontainers
- Snapshot testing
- Property testing
- Benchmarks
- Performance testing
- Chaos testing
