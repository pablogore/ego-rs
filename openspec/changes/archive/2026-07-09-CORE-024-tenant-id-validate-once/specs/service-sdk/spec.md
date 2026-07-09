# Delta for service-sdk

## ADDED Requirements

### Requirement: TenantResolver does not re-validate a Principal's pre-validated tenant claim

`TenantResolver::resolve()` MUST NOT call `TenantId::new()` or any other validation function against a tenant value that originates from `Principal.tenant_id`. Because `Principal.tenant_id` is now `Option<TenantId>` (already validated at `Principal` construction — see the `security-sdk` and `security-jwt` deltas of this change), `resolve()` MUST treat that value as pre-validated and clone it directly into the returned `CanonicalTenant`. The private `validated()` helper, as it applied to the Principal-derived tenant, MUST be removed — there is no remaining reason to re-run `TenantId::new()` against a value that is already a `TenantId`.

This requirement does NOT extend to the caller-supplied system/internal tenant hint (the `supplied_tenant: Option<&str>` parameter, used only in the `AllowSystemInternal` unauthenticated branch). That hint is a raw string that never passes through `Principal` construction, so it MUST continue to be validated via `TenantId::new()` exactly as before — this change does not alter that path's behavior.

(Previously: every call to `resolve()` re-validated the Principal's tenant claim by calling `Self::validated(principal_tenant)` — a private helper wrapping `TenantId::new()` — against the raw `&str` obtained from `Principal.tenant_id.as_deref()`, on every single tenant-scoped request, even though the claim never changes between authentication and each subsequent call.)

#### Scenario: Authenticated principal's tenant is cloned without re-validation

- GIVEN a `SecurityContext` wrapping a `Principal` with `tenant_id = Some(TenantId::new("tenant-a").unwrap())`
- WHEN `resolver.resolve(Some(&security), None)` is called
- THEN `Ok(CanonicalTenant::scoped(tenant))` is returned where `tenant` is a clone of the Principal's `TenantId` — no call to `TenantId::new()` or `TenantIdError` construction occurs during this resolution

#### Scenario: Hint agreement/mismatch behavior unchanged

- GIVEN a `SecurityContext` wrapping a `Principal` with `tenant_id = Some(TenantId::new("tenant-a").unwrap())`
- WHEN `resolver.resolve(Some(&security), Some("tenant-b"))` is called (hint disagrees, compared as strings)
- THEN `Err(SecurityError::TenantMismatch { expected: "tenant-a", actual: "tenant-b" })` is returned — identical to pre-change behavior

#### Scenario: System/internal caller-supplied hint is still validated

- GIVEN `TenantResolver::new(TenantEnforcementMode::AllowSystemInternal)` and no `SecurityContext`
- WHEN `resolver.resolve(None, Some(""))` is called (blank caller-supplied hint)
- THEN `Err(SecurityError::MissingContext)` is returned — the system/internal hint path still runs `TenantId`-equivalent validation on the raw string, unchanged by this delta

#### Scenario: No validation call reachable on the Principal-derived path (structural)

- GIVEN the source of `TenantResolver::resolve()` after this change
- WHEN the Principal-derived branches (the `Some(security)` match arm) are inspected
- THEN no call to `TenantId::new(...)` (directly or via a `validated()`-style helper) appears on the code path that handles `security.principal().tenant_id` — the only operation performed on that value is a clone into `CanonicalTenant::scoped(...)`

**Tests**: `tenant::tests::resolve_authenticated_hint_absent_resolves_to_principal_tenant`, `tenant::tests::resolve_authenticated_hint_agrees_resolves_to_principal_tenant`, `tenant::tests::resolve_authenticated_hint_disagrees_is_tenant_mismatch` (existing, updated fixture construction to build a `Principal` with `Option<TenantId>` directly), `tenant::tests::resolve_unauthenticated_allow_system_internal_with_hint_resolves_to_hint` (existing — confirms hint-path validation still runs). The "no re-validation" property (previous scenario) is verified by code inspection at review time, not a runtime assertion — once `Principal.tenant_id` is `Option<TenantId>`, there is no invalid value a unit test could construct to distinguish "validated once" from "re-validated every call".

---

## Out of Scope for This Delta

- **No change to `ServiceContext.tenant_id` / `tenant_hint()`** (`crates/service-sdk/src/context/mod.rs`). That is a deliberately-raw ingress hint per AD-011, a different concept from the authenticated Principal's tenant claim, and is untouched by this change. `testkit::TestContextBuilder`, which builds this hint, is likewise untouched.
- **No change to `TenantEnforcementMode` variants or the hint-mismatch/agreement decision logic** — only the source of validation for the Principal-derived value changes (removed), not the resolution algorithm's branches.
