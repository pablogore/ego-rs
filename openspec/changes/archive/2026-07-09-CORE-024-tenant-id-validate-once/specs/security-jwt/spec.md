# Delta for security-jwt

## ADDED Requirements

### Requirement: DefaultPrincipalMapper validates the tenant claim once, at mapping time

`DefaultPrincipalMapper::map()` MUST validate the resolved tenant claim (sourced from `tenant_id` | `tid` | `tenant`, first-present-wins, per the existing claim-priority order) by calling `TenantId::new()` before it is attached to the `Principal` being constructed. `map()` MUST NOT construct or return a `Principal` carrying an unvalidated or invalid tenant claim under any circumstance.

If the resolved tenant claim fails `TenantId::new()` validation (empty or whitespace-only after trim), `map()` MUST return `Err(AuthenticationError)` immediately, using an existing `AuthenticationError` variant capable of communicating a claim-level failure (e.g. `InvalidToken` or `MissingClaim`) — introducing a new error variant is a design-phase decision, not a requirement of this change, and MUST only happen if no existing variant can express the failure. The failure MUST surface at this mapping boundary (i.e. at login/authentication time), never later during request handling.

(Previously: the raw tenant claim string was attached to `Principal.tenant_id: Option<String>` with no validation performed by `DefaultPrincipalMapper`; an invalid claim silently reached the `Principal` and was only discovered on first use inside `TenantResolver::resolve()`, deep inside request handling.)

#### Scenario: Valid tenant claim maps to a validated TenantId

- GIVEN a claim set containing `"tid": "tenant-42"` and a valid `"sub"`
- WHEN `DefaultPrincipalMapper.map(&claim_set)` is called
- THEN `Ok((principal, claims))` is returned and `principal.tenant_id` is `Some(TenantId)` whose value is `"tenant-42"`

#### Scenario: Absent tenant claim leaves tenant_id unset

- GIVEN a claim set with a valid `"sub"` and none of `tenant_id` / `tid` / `tenant` present
- WHEN `DefaultPrincipalMapper.map(&claim_set)` is called
- THEN `Ok((principal, claims))` is returned and `principal.tenant_id` is `None`

#### Scenario: Invalid tenant claim fails at mapping time, not later

- GIVEN a claim set containing `"tid": "   "` (whitespace-only) and a valid `"sub"`
- WHEN `DefaultPrincipalMapper.map(&claim_set)` is called
- THEN `Err(AuthenticationError::_)` is returned — no `Principal` is constructed or returned, and the caller (the authentication boundary) observes the failure before any tenant-scoped request handling begins

**Tests**: `principal_mapper::tests::maps_tid_to_tenant_id` (existing, updated to assert `Some(TenantId)`), `principal_mapper::tests::maps_invalid_tenant_claim_fails` (new).
