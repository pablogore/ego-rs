# Delta for security-sdk

## MODIFIED Requirements

### FR-001: Principal — tenant_id is a pre-validated TenantId, not a raw String

`Principal.tenant_id` MUST be typed `Option<TenantId>` (domain type from `ego_domain::context`), not `Option<String>`. `Principal::with_tenant_id()` MUST accept a `TenantId` value directly — its signature changes from `impl Into<String>` to `TenantId`. `Principal` construction MUST NOT perform any tenant validation itself; by the time a caller has a `TenantId` to pass in, `TenantId::new()` has already validated it (non-empty after trim — unchanged semantics, see `ego_domain::context::TenantId`). No raw, unvalidated tenant string MUST survive past `Principal` construction into any downstream field or method.

(Previously: `tenant_id: Option<String>`, stored unvalidated at parse time; `with_tenant_id(impl Into<String>)` accepted any string including invalid ones, deferring validation to first use in `TenantResolver::resolve()`.)

#### Scenario: Default construction leaves tenant_id unset

- GIVEN `Principal::new(kind, subject_id)`
- WHEN the principal is constructed with no tenant supplied
- THEN `principal.tenant_id` is `None`

#### Scenario: with_tenant_id sets a pre-validated TenantId

- GIVEN a `TenantId` constructed via `TenantId::new("acme").unwrap()`
- WHEN `principal.with_tenant_id(tenant_id.clone())` is called
- THEN `principal.tenant_id` is `Some(tenant_id)` and no additional validation occurs during this call

#### Scenario: with_tenant_id overwrites the previous value

- GIVEN a principal already carrying `Some(TenantId::new("acme").unwrap())`
- WHEN `with_tenant_id(TenantId::new("contoso").unwrap())` is called again
- THEN `principal.tenant_id` is `Some(TenantId::new("contoso").unwrap())`

#### Scenario: Builder signature rejects raw strings at compile time

- GIVEN the `with_tenant_id` method signature `fn with_tenant_id(self, tenant_id: TenantId) -> Self`
- WHEN a caller attempts to pass a raw `&str` or `String` directly
- THEN the code fails to compile — the type system, not a runtime check, forces validation to happen before this call

**Tests**: `principal::tests::constructs_with_required_fields`, `principal::tests::with_tenant_id_sets_field`, `principal::tests::with_tenant_id_overwrites`, `principal::tests::subject_id_and_attributes`.

---

## Out of Scope for This Delta (Non-Goals)

- **No `Arc<str>` migration.** `TenantId` continues to wrap an owned `String`; cloning it remains an allocation. Making the id types allocation-cheap is a separate, larger decision covering all six `id_type!` types (`TenantId`, `EntityId`, `CorrelationId`, `CausationId`, `RequestId`, `AggregateId`), tracked as a future issue.
- **No change to `TenantId::new()`'s validation rule.** Non-empty-after-trim is preserved exactly; this delta relocates *when* validation runs, not *what* it checks.
- **No `Principal` field-visibility tightening.** `tenant_id` stays a public field; the type change (`Option<TenantId>` instead of `Option<String>`) is itself the safety guarantee — a `Principal` cannot be constructed with an invalid tenant claim because `TenantId` cannot be constructed with one.
