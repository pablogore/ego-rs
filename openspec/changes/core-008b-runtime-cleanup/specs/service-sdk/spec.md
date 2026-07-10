# Delta for service-sdk

## MODIFIED Requirements

### Requirement: Exactly One Canonical In-Runtime Tenant Representation (FR-008)

Exactly one representation of tenant MUST be canonical inside the runtime at the point
an operation executes: `CanonicalTenant` (`crates/service-sdk/src/runtime/tenant.rs`).
It wraps a private `Repr` enum (`Scoped(TenantId)` for a resolved tenant, `Systemwide`
for D1's valid tenant-less mode); its constructors are `pub(super)`, reachable only
within `crate::runtime`, so only `TenantResolver::resolve` may mint one. `Principal.tenant_id`,
`ServiceContext.tenant_id` (the ingress hint), and `ClaimSet::tenant()` are ingress/legacy
carriers only — none is independently authoritative for the same operation at execution
time. `Principal.tenant_id` is the authoritative *input* on the authenticated path;
`TenantResolver`'s output is the authoritative *runtime* value; `ServiceContext.tenant_id`
is demoted to a non-authoritative ingress hint (read via `ctx.tenant_hint()`).

(Previously: also listed domain `ExecutionContext` among ingress/legacy tenant carriers.
That type is deleted by this change and no longer exists.)

#### Scenario: Divergent ingress values converge to one authoritative value

- GIVEN a request where the Principal's tenant claim and a caller-supplied hint could disagree
- WHEN `RuntimeInner::enforce_tenant` runs
- THEN exactly one `CanonicalTenant` is produced and stored via `ctx.set_resolved_tenant`, and every downstream tenant-aware read (`ctx.canonical_tenant()`) observes that same value

#### Scenario: Only the runtime can construct a CanonicalTenant

- GIVEN code outside `crate::runtime` in `service-sdk`
- WHEN it attempts to construct a `CanonicalTenant` directly (e.g. `CanonicalTenant::scoped(...)`)
- THEN compilation fails with a visibility error — `scoped`/`systemwide` are `pub(super)`

**Tests**: `tenant::tests::canonical_tenant_scoped_is_constructible_within_runtime`, `tenant::tests::canonical_tenant_systemwide_is_constructible_within_runtime`.

---

## ADDED Requirements

### Requirement: Tenant Access MUST Match the Pipeline Stage

Tenant access is a convention, not a compiler-enforced restriction: tenant reads MUST
use either `tenant_hint()` or `canonical_tenant()`, and presence checks use
`has_tenant_hint()`, selected by pipeline stage:

| What the code exercises | Correct accessor |
|---|---|
| Context construction | `tenant_hint()` |
| Clone before runtime enforcement | `tenant_hint()` |
| Explicit propagation (task spawn, parameter passing) | `tenant_hint()` |
| Runtime / `TenantResolver` | `canonical_tenant()` |
| Authorization | `canonical_tenant()` |
| Enforcement (`enforce_tenant`, `#[tenant_scoped]`) | `canonical_tenant()` |

`canonical_tenant()` reads `resolved_tenant`, set only by `enforce_tenant()` via
`set_resolved_tenant()`; a `ServiceContext` built directly via `with_tenant_id()` without
running `enforce_tenant()` MUST return `None` from `canonical_tenant()`.

#### Scenario: Deprecated accessors do not exist

- GIVEN the `service-sdk` crate after this change
- WHEN `ServiceContext`'s public API is inspected
- THEN `ServiceContext` exposes no `tenant_id()` or `has_tenant()` methods

#### Scenario: Pre-enforcement code reads the ingress hint

- GIVEN a `ServiceContext` built directly via `with_tenant_id()`, with `enforce_tenant()` not yet called
- WHEN test or propagation code reads the tenant value
- THEN `ctx.tenant_hint()` returns the constructed value and `ctx.canonical_tenant()` returns `None`

#### Scenario: Enforcement-stage code reads the canonical value

- GIVEN a `ServiceContext` after `enforce_tenant()` has run and stored a resolved tenant
- WHEN authorization or `#[tenant_scoped]` logic reads the tenant value
- THEN `ctx.canonical_tenant()` returns the resolved value

**Tests**: `crates/service-sdk/tests/{smoke,context_propagation,context_cross_service,context_explicit_propagation}.rs` reference only `tenant_hint()` and `canonical_tenant()`.

### Requirement: Unused Execution-Context Abstractions Are Removed

`ExecutionContext`, `DomainExecutionContext` (`crates/domain/src/context.rs`), and
`RuntimeExecutionContext` (`crates/runtime/src/context.rs`), including their re-exports,
are removed because they have zero production callers and `CommandContext`
(`crates/persistent-entity/src/command_context.rs`) is the sole execution-context
abstraction with production callers. This reflects the evidence gathered for this
change, not a standing prohibition on ever introducing an execution-context abstraction.

#### Scenario: No workspace reference to the removed types remains

- GIVEN the workspace source after this change
- WHEN searched with `rg "ExecutionContext" crates/ --type rust`
- THEN zero matches are found, and `cargo build --workspace` succeeds

### Requirement: Workspace Contains No Deprecated Tenant Accessors

This is a distinct acceptance concern from pipeline-stage correctness above: one of
CORE-008B's originating goals was eliminating `#[deprecated]` warnings, not merely
picking the right accessor per call site.

#### Scenario: No deprecated-accessor warnings remain

- GIVEN the workspace after this change
- WHEN `cargo build --workspace` and `cargo test --workspace` run
- THEN neither emits a `#[deprecated]` warning for `tenant_id()` or `has_tenant()`, because neither method exists

#### Scenario: Only the field remains, not the deprecated methods

- GIVEN the workspace source
- WHEN searched with `rg "\.tenant_id\(\)|\.has_tenant\(\)" crates/`
- THEN zero matches are found — the only surviving `tenant_id` symbol is the `pub tenant_id: Option<String>` field and its `tenant_hint()` reader

### Requirement: Architecture Documentation Describes the Explicit-Propagation Model Only

`docs/architecture.md` MUST NOT describe `ServiceContext` as TaskLocal-scoped or as
propagating via ambient/task-local state.

#### Scenario: Architecture doc contains no ambient-propagation claim

- GIVEN `docs/architecture.md` after this change
- WHEN searched with `rg "TaskLocal|ambient" docs/architecture.md`
- THEN zero matches describe `ServiceContext` propagation
