# Delta for external-data-providers

This delta closes the runtime-side tenant-authority gap on the provider fetch
path. The existing canonical requirement "Tenant Isolation For Tenant-Scoped
Fetches" constrains only the **provider** (it "MUST NOT substitute or mint a
different tenant value") — a rule satisfied trivially, because the provider only
ever reads the tenant it is handed. This delta adds the missing **runtime**
obligation: the authoritative tenant derives from the established context, and
the runtime reconciles the caller-supplied request tenant against it per a fixed
five-outcome matrix — inject when absent, pass when it matches, and fail closed
otherwise (a `TenantMismatch` when a different tenant is established, a distinct
`TenantContextMissing` when a tenant is asserted with no established authority to
validate it) — so a forged request tenant can neither widen nor cross-tenant-read
and a caller can never choose the authorizing tenant. Each fail-closed decision
remains observable. It does not restate, re-specify, or re-work the timeout,
retry, or observability contract delivered by issue #234 ("Timeout/Retry
Observability", "Fetch Observability Signals"); those are PRESERVED unchanged.

## ADDED Requirements

### Requirement: Authoritative Tenant Derives From Established Context

For a tenant-scoped fetch, the tenant the fetch is authorized against MUST be the
tenant already established for the current command or entity context. The runtime
MUST NOT treat an unvalidated caller-supplied request tenant as the authority,
and a caller MUST NOT be able to choose the authorizing tenant at the fetch call
site. When the established context carries a tenant and the caller supplied none,
the runtime MUST inject the established tenant into the fetch. When the
established context carries a tenant and the caller supplied one, the runtime MUST
validate the caller-supplied value against the established tenant before any
provider invocation.

#### Scenario: Absent caller tenant is injected from the established context

- GIVEN an established context tenant `A`
- AND a fetch request whose tenant is absent
- WHEN the runtime performs the tenant-scoped fetch
- THEN the provider is invoked with tenant `A`, injected from the established
  context, not left absent and not chosen by the caller

#### Scenario: Matching caller tenant is authorized against the context

- GIVEN an established context tenant `A`
- AND a fetch request whose caller-supplied tenant equals `A`
- WHEN the runtime performs the fetch
- THEN the request is authorized and the provider is invoked with tenant `A`

### Requirement: Fail Closed On Tenant Mismatch

When the established context carries a tenant and the caller-supplied request
tenant is a different tenant, the runtime MUST fail the fetch closed with an
explicit `TenantMismatch` error and MUST NOT invoke the provider. The runtime
MUST NOT silently override the caller-supplied tenant with the established tenant
and proceed, because a silent override would hide a spoofing attempt or a
cross-tenant defect. The failure MUST be non-retryable and MUST carry no raw
tenant identifier in its message.

#### Scenario: Disagreeing caller tenant fails closed before any provider call

- GIVEN an established context tenant `A`
- AND a fetch request whose caller-supplied tenant is a different tenant `B`
- WHEN the runtime performs the fetch
- THEN the fetch fails with an explicit `TenantMismatch` error
- AND the provider is never invoked
- AND the error is non-retryable and exposes no raw tenant identifier

### Requirement: Fail Closed On Tenant Asserted Without Established Authority

When no tenant is established for the current context and the caller supplied a
request tenant, the runtime MUST fail the fetch closed with an explicit
`TenantContextMissing` error, distinct from `TenantMismatch`, and MUST NOT invoke
the provider. The assertion cannot be validated against any authority, so
accepting it would let the caller choose the authorizing tenant — which the
runtime MUST NOT permit. The failure MUST be non-retryable and MUST carry no raw
tenant identifier in its message. This case MUST be distinguishable from a
`TenantMismatch`, because there is no established tenant to mismatch against.

#### Scenario: Caller asserts a tenant with no established authority

- GIVEN no established tenant for the context
- AND a fetch request whose caller-supplied tenant is `C`
- WHEN the runtime performs the fetch
- THEN the fetch fails with an explicit `TenantContextMissing` error, distinct
  from `TenantMismatch`
- AND the provider is never invoked
- AND the error is non-retryable and exposes no raw tenant identifier

#### Scenario: An unauthorized assertion cannot cross-tenant-read

- GIVEN no established tenant for the context
- AND a caller asserts tenant `tenant-b` on the request
- WHEN the runtime performs the fetch
- THEN the fetch fails closed and the provider never receives `tenant-b`
- AND no data scoped to `tenant-b` is returned

### Requirement: Fail-Closed Tenant Decisions Are Observable

A fail-closed tenant decision (`TenantMismatch` or `TenantContextMissing`) MUST
emit exactly one terminal fetch signal through the runtime's existing
observability pipeline, classified as a distinct non-retryable outcome, with zero
retries scheduled and zero provider invocations. A fail-closed decision MUST NOT
be silently dropped without an emitted signal, so that cross-tenant attempts are
observable. The emitted signal MUST NOT carry a raw tenant identifier.

#### Scenario: A mismatch emits one terminal signal with no retries or provider calls

- GIVEN an established context tenant `A` and a caller-supplied tenant `B != A`
- WHEN the runtime reconciles the request and fails closed
- THEN exactly one terminal fetch signal is emitted with a non-retryable
  tenant-mismatch outcome
- AND no retry is scheduled and the provider is never invoked
- AND the signal carries no raw tenant identifier

#### Scenario: A context-missing decision emits one terminal signal with no retries or provider calls

- GIVEN no established tenant for the context and a caller-supplied tenant `C`
- WHEN the runtime reconciles the request and fails closed
- THEN exactly one terminal fetch signal is emitted with a non-retryable
  tenant-context-missing outcome
- AND no retry is scheduled and the provider is never invoked
- AND the signal carries no raw tenant identifier

### Requirement: Cross-Tenant Access Via A Forged Request Tenant Is Impossible

A tenant-scoped fetch MUST NOT be able to read another tenant's data by forging
the request tenant. After reconciliation the provider MUST receive only the
context-authoritative tenant; there MUST be no path by which a caller-supplied
tenant that differs from the established context reaches the provider or causes a
fetch scoped to that different tenant. Concurrent dispatches for different tenants
MUST NOT be able to observe one another's tenant.

#### Scenario: Forged cross-tenant read never reaches the provider

- GIVEN an established context tenant `tenant-a`
- AND a caller forges the request tenant as `tenant-b`
- WHEN the runtime performs the fetch
- THEN the fetch fails closed
- AND the provider never receives `tenant-b`
- AND no data scoped to `tenant-b` is returned

#### Scenario: The provider receives only the authorized tenant

- GIVEN an established context tenant `A` and any caller-supplied request tenant
- WHEN the fetch is authorized (caller absent or caller equal to `A`)
- THEN the provider receives exactly `A`, with no path to widen or substitute it

#### Scenario: Concurrent dispatches do not cross-contaminate tenants

- GIVEN two concurrent dispatches, one established for `tenant-a` and one for
  `tenant-b`, sharing the runtime's provider access
- WHEN both issue tenant-scoped fetches with an absent request tenant
- THEN the `tenant-a` dispatch's provider receives `tenant-a` and the `tenant-b`
  dispatch's provider receives `tenant-b`, with neither observing the other's
  tenant

### Requirement: Tenant-Agnostic Fetch Compatibility Is Preserved

When no tenant is established for the context and the caller supplied no request
tenant, the runtime MUST perform the fetch without imposing a tenant, preserving
the single-tenant and not-tenant-scoped usage. The existing request constructors
that produce a tenant-agnostic request (an absent tenant) MUST continue to
compile and behave unchanged, and enforcement MUST be a no-op in this
both-absent case. Preserving tenant-agnostic mode MUST NOT be achieved by
accepting an arbitrary caller-supplied tenant when no authority is established —
that case fails closed (see "Fail Closed On Tenant Asserted Without Established
Authority").

#### Scenario: Both-absent request is unchanged tenant-agnostic pass-through

- GIVEN no established tenant for the context
- AND a tenant-agnostic fetch request (no tenant)
- WHEN the runtime performs the fetch
- THEN the provider is invoked with no tenant, exactly as before this change
- AND existing tenant-agnostic callers compile and pass without modification

## MODIFIED Requirements

### Requirement: Tenant Isolation For Tenant-Scoped Fetches

When a fetch is tenant-scoped, the tenant value a provider receives MUST be the
tenant already established for the current request or entity, as reconciled and
authorized by the runtime provider-access chokepoint before the provider is
invoked. A provider MUST NOT be able to substitute or mint a different tenant
value, and a caller MUST NOT be able to make the provider receive a tenant other
than the established one by populating the request tenant field: the runtime is
the enforcing authority, and any caller-supplied tenant that disagrees with the
established context — or that is asserted when no context tenant is established —
fails closed rather than reaching the provider.

(Previously: the requirement constrained only the provider — "a provider MUST NOT
be able to substitute or mint a different tenant value" — and named the tenant a
provider receives as "the tenant already established for the current request or
entity" without stating who enforces that. It is strengthened here to name the
runtime provider-access chokepoint as the enforcing authority and to close the
caller-side forgery path, which the provider-only phrasing left to convention.)

#### Scenario: Tenant-scoped fetch receives only the established tenant

- GIVEN a tenant-scoped fetch for an established tenant `T`
- WHEN the provider executes that fetch
- THEN it receives `T` as the tenant, with no path to substitute or override it
  with a different value

#### Scenario: A caller cannot widen the tenant by populating the request field

- GIVEN a tenant-scoped fetch whose established tenant is `T`
- AND a caller that populates the request tenant with a different value `U`
- WHEN the runtime reconciles the request
- THEN the fetch fails closed and the provider never receives `U`
