# Delta for testkit

## ADDED Requirements

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
