# Delta for external-data-providers

**Capability choice**: `external-data-providers` is a new capability
directory — no such entry exists under `openspec/specs/` today, and the
proposal (§9, §10) names it as the first canonical spec for read-side
external data access. This delta specifies the **observable contract** this
capability must satisfy, deliberately at the level of behavior rather than
mechanism. AD-001–AD-012 (proposal §11) are now resolved: `design.md` is the
authoritative record of the resolution model (a runtime-owned SPI +
registry, reached by handlers through a handler-facing facade port owned by
`persistent-entity`), the fetch contract shape, registry key, error
taxonomy, and lifecycle mechanism. This document does not restate,
hardcode, or relitigate any of those concrete choices — it only fixes the
externally observable behavior every requirement below MUST exhibit, which
the chosen design already satisfies.

## ADDED Requirements

### Requirement: Fail-Closed Provider Resolution

Resolving a provider key with no matching registration MUST fail loudly and
observably. The system MUST NOT substitute a silent default, a no-op
provider, or an empty result in place of a missing registration, regardless
of which resolution path a handler uses to reach the provider.

#### Scenario: Unregistered key fails loudly

- GIVEN no provider is registered for key `K`
- WHEN a handler resolves a provider for `K`
- THEN resolution fails with an explicit, observable error; no silent
  default or empty value is returned

#### Scenario: Registered key resolves successfully

- GIVEN a provider is registered for key `K`
- WHEN a handler resolves a provider for `K`
- THEN it receives that provider and can invoke its fetch capability

### Requirement: Duplicate Registration Fails At Registration Time

Registering a second provider under a key that already has a registered
owner MUST fail immediately at registration time — never at first
resolution, never last-wins or first-wins. Each key has exactly one owner.

#### Scenario: Duplicate registration rejected

- GIVEN a provider is already registered for key `K`
- WHEN a second provider is registered for the same key `K`
- THEN registration fails immediately and the first registration remains
  the sole owner

#### Scenario: Distinct keys register independently

- GIVEN two providers registered under two distinct keys
- WHEN both registrations occur
- THEN both succeed and each resolves independently

### Requirement: Explicit, Non-Reflective Registration

A provider MUST become known to the runtime only through an explicit
registration call. The system MUST NOT discover, auto-register, or
activate a provider through type scanning, reflection, or any implicit
mechanism.

#### Scenario: Compiled-but-unregistered provider never resolves

- GIVEN a provider type exists in the binary but was never explicitly
  registered
- WHEN a handler attempts to resolve its key
- THEN resolution fails exactly as an unregistered key would; the
  provider's mere presence in the binary has no effect

### Requirement: Zero Runtime Overhead When Unused

An application that registers no providers MUST incur no measurable
runtime overhead attributable to this capability — no background work, no
per-request allocation, no startup cost beyond what registering zero
providers trivially requires.

#### Scenario: No providers registered, no observable overhead

- GIVEN a runtime with no provider ever registered
- WHEN the runtime starts up and serves requests under load
- THEN no measurable latency, allocation, or background work attributable
  to this capability is observable

### Requirement: Fetch Observability Signals

Every fetch attempt MUST emit, through the runtime's existing
observability pipeline (never a separate or provider-owned pipeline): its
latency, whether it timed out, how many retries occurred, whether it was
served from cache or fetched fresh, and the provider's name/identity as a
correlation field.

#### Scenario: Successful fetch emits latency and provider identity

- GIVEN a registered provider completes a fetch successfully
- WHEN the fetch completes
- THEN a signal is emitted carrying latency and the provider's identity,
  through the runtime's existing observability pipeline

#### Scenario: Cache hit is distinguishable from cache miss

- GIVEN a provider fetch that is served from cache and another that is not
- WHEN each is observed through emitted signals
- THEN the cache hit and the cache miss are distinguishable from each other

#### Scenario: Retried or timed-out fetch signals retries and timeout

- GIVEN a fetch that times out and is retried before eventually completing
- WHEN the fetch sequence finishes
- THEN emitted signals reflect the timeout occurrence and the number of
  retry attempts

### Requirement: Providers Replaceable By Deterministic Test Doubles

Any registered provider MUST be substitutable with a deterministic test
double without modifying the handler or caller code that resolves it.
`testkit` MUST supply at least one such double, and a handler MUST obtain
external data only through a registered provider — never by constructing
an external client inline.

#### Scenario: Test double swaps in without touching handler code

- GIVEN a handler that resolves and fetches from a registered provider
- WHEN the registered provider is replaced with a `testkit` test double at
  the registration boundary
- THEN the handler's code is unchanged and its behavior is fully
  deterministic under the double

#### Scenario: Reference-app handler never constructs a client inline

- GIVEN the reference-app's dogfooded provider usage
- WHEN its handler code is inspected
- THEN it obtains external data exclusively through a registered provider;
  no external client is constructed inline in the handler

### Requirement: SPI Isolated From Runtime Internals

A provider implementation MUST be satisfiable using only the SPI's
publicly exposed types. Implementing or registering a provider MUST NOT
require depending on, or having knowledge of, the runtime's internal
modules, state, or lifecycle machinery.

#### Scenario: A provider compiles and registers without runtime-internal dependencies

- GIVEN a new provider implementation outside the runtime crate
- WHEN it is implemented and registered
- THEN it depends only on the public SPI surface, never on runtime-internal
  types or modules

### Requirement: Backward Compatibility For Existing Handlers

Existing `PersistentEntity` implementations that never use an external
data provider MUST continue to compile and behave unchanged once this
capability ships.

#### Scenario: Unmodified handler compiles and passes unchanged

- GIVEN an existing handler that never resolves an external data provider
- WHEN the workspace is rebuilt after this capability ships
- THEN it compiles and its existing tests pass without modification

### Requirement: Explicit, Single-Owner Lifecycle

Provider startup and shutdown MUST be owned by exactly one component in
the runtime's lifecycle; no provider manages its own startup/shutdown
independently of that owner.

#### Scenario: Shutdown reaches every registered provider exactly once

- GIVEN two or more providers registered at startup
- WHEN the runtime shuts down
- THEN each registered provider's shutdown is invoked exactly once, through
  the one owning lifecycle path, never skipped or double-invoked

### Requirement: Tenant Isolation For Tenant-Scoped Fetches

When a fetch is tenant-scoped, the tenant value a provider receives MUST be
the tenant already established for the current request or entity; a
provider MUST NOT be able to substitute or mint a different tenant value.

#### Scenario: Tenant-scoped fetch receives only the established tenant

- GIVEN a tenant-scoped fetch for an established tenant `T`
- WHEN the provider executes that fetch
- THEN it receives `T` as the tenant, with no path to substitute or
  override it with a different value
