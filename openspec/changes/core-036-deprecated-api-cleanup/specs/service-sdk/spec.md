# Delta for service-sdk

## ADDED Requirements

### Requirement: Cross-Tenant Access Checks Are Destination-Scoped Only

`ServiceContext` MUST expose exactly one cross-tenant access predicate,
`is_cross_tenant_allowed_for(&TenantId)`, which returns `true` only when a cross-tenant grant is
present AND was scoped to that exact destination (CORE-008A AD-008). The permit-presence-only
predicate `is_cross_tenant_allowed()` — which reports "is any permit attached" rather than "is
access allowed to the destination actually being accessed" — MUST NOT exist. After removal, a
workspace-wide search for `is_cross_tenant_allowed` (excluding the `_for` variant) MUST return zero
matches across crate sources, tests, and `COOKBOOK.md`.

(Why the predicate goes: `is_cross_tenant_allowed()` was `#[deprecated]`, and its own note documents
a security foot-gun — it reports whether *any* permit is attached, so gating a real access decision
on it would let a permit issued for one destination authorize a different one. The safe,
destination-scoped `is_cross_tenant_allowed_for(&TenantId)` (CORE-008A AD-008) already exists, so
keeping the presence-only predicate violated `PRD.md:140`.)

(Migration: every in-repo reference is a test or doc line — two unit tests in `context/mod.rs`, one
assertion in `tests/smoke.rs`, one in `tests/cross_tenant_access_contract.rs`, and one doc mention at
`COOKBOOK.md:422`. Each test moves to `is_cross_tenant_allowed_for(&destination)`, where the
destination is already in scope at every call site, and its `#[allow(deprecated)]` is deleted; the
`COOKBOOK.md` deprecated parenthetical is deleted, leaving the `is_cross_tenant_allowed_for` entry.
No production caller exists.)

#### Scenario: The permit-presence predicate is absent

- GIVEN `ServiceContext` after CORE-036
- WHEN the workspace is scanned with `rg 'is_cross_tenant_allowed\b' crates/ COOKBOOK.md` (excluding `_for`)
- THEN the search returns zero matches, and no `#[deprecated]` cross-tenant predicate exists

#### Scenario: Destination-scoped check authorizes only its exact destination

- GIVEN a `ServiceContext` holding a cross-tenant permit scoped to `tenant-b`
- WHEN `is_cross_tenant_allowed_for(&tenant_b)` and `is_cross_tenant_allowed_for(&tenant_c)` are evaluated
- THEN the first returns `true` and the second returns `false`

#### Scenario: Migrated callers use the destination-scoped predicate

- GIVEN the tests previously calling `is_cross_tenant_allowed()` (`context/mod.rs`, `tests/smoke.rs`, `tests/cross_tenant_access_contract.rs`)
- WHEN they are migrated to `is_cross_tenant_allowed_for(&destination)` and their `#[allow(deprecated)]` attributes are removed
- THEN `cargo test --workspace` passes and no `#[allow(deprecated)]` remains in those files

### Requirement: Macro-Visibility Hatches Are Retained As Intentional Codegen Surface

The `#[doc(hidden)] pub` items that exist solely so `ego-service-sdk-macros`-generated code can
reach otherwise-`pub(crate)` internals — `RuntimeInner::logger`, `RuntimeInner::authorization_provider`,
`RuntimeInner::record_security_denial`, `pub use async_trait`, and `pub use ego_security_sdk as
security` — MUST be retained unchanged. They are NOT deprecated surface: they carry no `#[deprecated]`
attribute, are hidden from rustdoc, and are a required part of the code-generation contract. A
no-shims audit MUST NOT treat them as removable.

#### Scenario: Codegen hatches survive the cleanup

- GIVEN the CORE-036 cleanup
- WHEN it completes
- THEN `RuntimeInner::{logger,authorization_provider,record_security_denial}`, `pub use async_trait`, and `pub use ego_security_sdk as security` are byte-unchanged and still `#[doc(hidden)] pub`

#### Scenario: Hatches carry no deprecation marker

- GIVEN the retained macro-visibility hatches
- WHEN their source is inspected
- THEN none carries a `#[deprecated]` attribute, so the no-shims policy does not flag them

### Requirement: Legacy Flat trace_id Mirror Is Retained And Documented

The private legacy flat `trace_id` mirror on `ServiceContext` MUST be retained unchanged. It is NOT
deprecated: it is authoritative-by-construction under `TraceContext` (PROD-003 ADR-4) — the field is
private and can only be mutated through builders that keep it in sync with
`trace_context().trace_id()`. Any change to it is a PROD-003 concern and is out of scope for the
deprecated-API cleanup. A no-shims audit MUST NOT treat it as a removable alias.

#### Scenario: The trace_id mirror is unchanged and undeprecated

- GIVEN CORE-036 completes
- WHEN `crates/service-sdk/src/context/mod.rs` is inspected
- THEN the private `trace_id` field and its `with_trace_id`/`trace_id` accessors are byte-unchanged and carry no `#[deprecated]` attribute

#### Scenario: The mirror stays consistent with TraceContext

- GIVEN a `ServiceContext` built with `with_trace_context(tc)`
- WHEN `trace_id()` is read
- THEN it equals `trace_context().trace_id()` rendered as W3C hex (invariant preserved by construction)
