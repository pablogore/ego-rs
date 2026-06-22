# Proposal: CORE-010B — Optional Security Capability

Make security an **optional runtime capability** instead of a mandatory assumption.
After this change, the EGO runtime can build and run with no security configured —
internal services, batch jobs, schedulers, projections, and prototypes work with zero
auth setup. Security activates only when explicitly installed via `RuntimeBuilder`, and
every future auth feature (JWT, OAuth, RBAC, service-to-service) is purely additive on
top of this optional foundation. The runtime core stays security-agnostic.

This is the architectural sibling of CORE-010A. CORE-010A removed *ambient* context
access (no `task_local`, no `current()`, no `scope()`); CORE-010B removes the *mandatory*
security assumption while inheriting every explicit-propagation rule CORE-010A established.

## Intent

| Question | Answer |
|----------|--------|
| What problem | The runtime implicitly assumes a `SecurityContext`/principal is always available. Code paths and future capabilities are written as if identity always exists. |
| Why now | CORE-011+ (JWT, RBAC, service-to-service auth) will be built on top of this. If "no security" and "anonymous" are not distinguishable from day one, those capabilities become ambiguous and force callers to overbuild. |
| What success looks like | `RuntimeBuilder::new().build()` produces a fully functional runtime with no security. `RuntimeBuilder::new().with_security(authn, authz).build()` enables it. Domain handlers see `None` and keep working; identity-requiring components fail explicitly with `Result`, never panic. |
| Nature of change | **Architectural prevention refactoring**, NOT a functional migration. No new concrete auth providers are added in this change. |

## Problem

The current execution context carries `security: Option<Arc<SecurityContext>>`, so the
*field* is already optional — but there is no runtime-level switch (the builder) that
controls whether the capability is installed at all. Several call sites are written
assuming security will always be configured, and `authorize_in_context` treats `None`
as a propagation bug rather than a valid deployment state. CORE-010B makes
"security not installed" a first-class, explicitly supported runtime configuration.

## Codebase Reconciliation (verified — read before spec/design)

The source brief describes the *target shape*. The current code differs in load-bearing
ways. Spec and design MUST treat the following as the real starting point, not assume the
brief's types already exist.

| Brief assumption | Verified current reality | Implication |
|------------------|--------------------------|-------------|
| `ExecutionContext { service_context, security_context: Option<SecurityContext> }` | **No `ExecutionContext` type exists.** Security lives directly on `ServiceContext.security: Option<Arc<SecurityContext>>` (`crates/service-sdk/src/context/mod.rs:58`). | **Resolved D1/B**: no `ExecutionContext` wrapper. `ServiceContext` evolves in-place adding `require_security()`; existing `security()` accessor unchanged. |
| `SecurityContext` is an enum `{ Anonymous, Authenticated(Principal) }` | **`SecurityContext` is a struct** with non-optional `principal: Principal` and a documented invariant "if it exists, a Principal exists" (`crates/security-sdk/src/context/mod.rs:15-21`). | **Resolved GAP-008**: `SecurityContext` struct is unchanged. No type migration, no enum, no rename. `Option<Arc<SecurityContext>>` already encodes both CORE-010B states. Enum deferred to CORE-011. |
| Provider traits do not exist yet | **`AuthenticationProvider` and `AuthorizationProvider` traits already exist** (`crates/security-sdk/src/authentication/mod.rs`, `crates/security-sdk/src/authorization/mod.rs`). | FR-010 is already satisfied. CORE-010B does not re-create them; it wires optionality around them. |
| No concrete implementations | **`BasicAuthenticationProvider` and `RbacProvider` already shipped** in prior changes (`crates/security-sdk/src/providers/`). | "No concrete implementations in CORE-010B" means *do not add new ones*; it does not mean the crate is empty today. |
| Error is `AuthorizationError::SecurityNotEnabled` | Existing error is `SecurityError::MissingContext` (`crates/security-sdk/src/error/mod.rs:31`). There is no `AuthorizationError` type. | **Resolved D3/A**: add `SecurityError::CapabilityNotEnabled` alongside `MissingContext`. Two distinct variants for two distinct failure modes. |
| `RuntimeBuilder` exists with `.with_security(...)` | `RuntimeInner` exists; the public `RuntimeBuilder` is explicitly **deferred** ("TASK-013 / TASK-014", `crates/service-sdk/src/runtime/runtime_builder.rs:7-9`). | The builder surface and `.with_security(...)` must be designed from the deferred stub, not merely extended. |

## Goals

- Security is an opt-in capability registered through `RuntimeBuilder`.
- The runtime core compiles, builds, and runs with no security present.
- The two runtime states (`None` = not installed, `Some(SecurityContext)` = installed) are
  unambiguously representable and distinguishable via `Option<Arc<SecurityContext>>`.
- Absence of security is a **valid runtime state** that propagates as `Result`, never a
  panic.
- All security state propagates explicitly, inheriting CORE-010A's rules.
- Existing call sites that assume mandatory security are migrated to the optional model.

## Non-Goals (explicitly out of scope)

- JWT, OAuth2, OpenID Connect.
- RBAC/ABAC policy engines, LDAP, Keycloak, Auth0 (beyond the trait abstractions and the
  already-shipped basic/RBAC providers, which are NOT modified to add features here).
- Claims, roles, permissions modeling beyond what already exists.
- Auth middleware / interceptors that perform authentication.
- Any new concrete authentication or authorization logic.
- `ServiceContext` redesign beyond what optionality requires.

## Requirements

| ID | Requirement | Verified status today |
|----|-------------|-----------------------|
| FR-001 | Runtime builds without a security provider. | New surface (builder deferred). |
| FR-002 | Runtime builds with security providers via `RuntimeBuilder::with_security(authn, authz)`. | New surface. |
| FR-003 | Absence of security does not prevent actor execution, service invocation, timers, scheduler, projections, or persistence. | To enforce/verify. |
| FR-004 | Runtime core assumes no `current_user`, principal, identity, claims, roles, or permissions. | Partially holds; must be guaranteed. |
| FR-005 | Execution context exposes security via existing `security() -> Option<&SecurityContext>` and new `require_security() -> Result<&SecurityContext, SecurityError>` on `ServiceContext`. | Field already exists; `security()` method exists; `require_security()` added. |
| FR-006 | Domain components support the `security == None` state. | Must be guaranteed across call sites. |
| FR-007 | A `SecurityNotEnabled` condition is surfaced as a `Result`, never a panic. | New variant/type. |
| FR-008 | No global security configuration: no `static SECURITY_PROVIDER`, no `lazy_static!`, no `OnceCell<SecurityProvider>`. | Inherit + enforce. |
| FR-009 | Security follows CORE-010A rules: no `task-local`, no `thread-local`, no ambient context. | Inherit + enforce. |
| FR-010 | Security implementations depend on interfaces (`AuthenticationProvider`, `AuthorizationProvider` traits). | **Already satisfied** — runtime depends on traits, not concretes. |

## Key Architectural Decision — Two-State Security Semantic

The execution context distinguishes two states via `Option<Arc<SecurityContext>>` on
`ServiceContext` — no type changes required:

| Value | Meaning | Who sees it |
|-------|---------|-------------|
| `None` | Security capability is **not installed** in this runtime. | Internal services, batch jobs, schedulers, prototypes. |
| `Some(security_ctx)` | Capability installed; `security_ctx.principal()` is always valid. | Authenticated requests; consumed by CORE-011+ JWT/RBAC. |

`Anonymous` (capability installed, no identity) is **intentionally deferred to CORE-011**.
No code path in CORE-010B constructs an anonymous context — no request-entry interceptor
or factory is in scope. Shipping a dead variant before its producer exists adds complexity
without value. CORE-011 introduces both the interceptor and the enum variant together.

`SecurityContext` remains its current struct. No rename, no enum, no call-site migration
for type shape. The `Option` wrapper on `ServiceContext.security` is already the correct
two-state encoding.

## Key Architectural Decision — Two-Layer Access API (no public fields)

Security state is read through methods, never raw fields, so the optional/required
distinction is encoded in the type system:

```rust
// Layer 1 — explicit optional access (for conditional logic) — existing, unchanged
service_ctx.security() -> Option<&SecurityContext>

// Layer 2 — fail-fast controlled access (for components that require identity) — new
service_ctx.require_security() -> Result<&SecurityContext, SecurityError>
```

- Domain handlers that *optionally* react to identity use the existing
  `if let Some(security) = service_ctx.security() { ... }`.
- Components that *require* identity use `let security = service_ctx.require_security()?;`
  and propagate `SecurityError::CapabilityNotEnabled` as a recoverable error.

`require_security()` returns `Result`, never panics. Panics are reserved exclusively for
internal invariant violations, never for the legitimate "security not installed" state
(FR-007).

## Architecture Impact

| Area | Decision |
|------|----------|
| Execution context | `ServiceContext` evolves in-place (D1/B). Existing `security() -> Option<&SecurityContext>` accessor is unchanged; `require_security() -> Result<&SecurityContext, SecurityError>` added. No new `ExecutionContext` wrapper. |
| `SecurityContext` | **Unchanged** (GAP-008). The existing struct with non-optional `principal` remains as-is. `Option<Arc<SecurityContext>>` on `ServiceContext` already encodes both CORE-010B states. Enum and `Anonymous` variant deferred to CORE-011. |
| `RuntimeBuilder` | Design the public builder with `.with_security(authn, authz)` accepting `(Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>)` (GAP-009). `build()` works with or without it. No combined `SecurityCapability` trait. Capability registration is instance-scoped — no globals. |
| Provider traits | Reuse existing `AuthenticationProvider` / `AuthorizationProvider`. Runtime depends on `Arc<dyn ...>`, not concretes. |
| Error model | `SecurityError::CapabilityNotEnabled` added alongside `SecurityError::MissingContext` (D3/A). Distinct failure modes, no new error type proliferation. |
| Propagation | All security state flows explicitly by parameter/ownership/clone — inherits CORE-010A. |

## No Global / Ambient State (inherited from CORE-010A)

Forbidden, enforced by the same gates CORE-010A introduced:

- `static SECURITY_PROVIDER`
- `lazy_static!` for security
- `OnceCell<SecurityProvider>` / global once-init
- `task-local` or `thread-local` security

The security provider is owned by the runtime instance created by `RuntimeBuilder` and
propagated explicitly. Verified by grep gates over the workspace (same mechanism as
CORE-010A's verification report).

## Resolved Decisions

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| D1 | Context shape | **B — evolve `ServiceContext` in-place** | No `ExecutionContext` wrapper. The existing `security()` accessor and new `require_security()` live directly on `ServiceContext`. Avoids an extra layer, conceptual duplication, and unnecessary call-site churn — consistent with the CORE-010A simplification goal. |
| D2 | `SecurityContext` type | **GAP-008 — struct unchanged** | `SecurityContext` remains the existing struct. No `AuthenticatedContext`, no enum, no call-site migration for type shape. `Option<Arc<SecurityContext>>` already encodes both states. Enum deferred to CORE-011 together with its `Anonymous` producer. |
| D3 | Error placement | **A — new variant on `SecurityError`** | Add `SecurityError::CapabilityNotEnabled` alongside the existing `SecurityError::MissingContext`. Two distinct errors for two distinct situations: `MissingContext` = capability exists but was not propagated; `CapabilityNotEnabled` = capability was never installed. Valuable for debugging; avoids proliferating new error types. |
| A1 | `Anonymous` variant | **Defer to CORE-011** | No code path in CORE-010B constructs `Some(Anonymous)` — no interceptor or request-entry factory is in scope. Shipping a dead variant is premature. CORE-011 introduces the layer that produces it. |
| GAP-009 | `with_security(...)` parameter shape | **Tuple of existing traits** | `RuntimeBuilder::with_security(authn: Arc<dyn AuthenticationProvider>, authz: Arc<dyn AuthorizationProvider>)`. No new `SecurityCapability` trait — both providers accepted directly. The combined trait added no architectural value: no equivalent pattern exists in the runtime, and wrapping two independent contracts behind a single trait is parameter-reduction without semantic gain. A single builder method with two parameters keeps the API explicit and avoids forcing users to implement a delegating wrapper. |

## Scope

### In Scope
- Public `RuntimeBuilder` surface with optional `.with_security(authn, authz)` and a `build()`
  that works with or without security.
- Instance-scoped security capability registration (no globals).
- `require_security()` accessor on `ServiceContext` (existing `security()` unchanged).
- `SecurityError::CapabilityNotEnabled` — new variant, surfaced as `Result`.
- Updating `authorize_in_context` to return `CapabilityNotEnabled` (not `MissingContext`) when `security == None`.
- Migrating call sites that assume mandatory security to the optional model.
- No `SecurityContext` type changes (struct unchanged, no enum, no rename).

### Out of Scope
- All Non-Goals above (JWT/OAuth/OIDC, RBAC/ABAC engines, claims/roles/permissions
  modeling, auth middleware, new concrete providers).
- Telemetry, DI framework, or actor-lifecycle redesign beyond optionality wiring.

## Acceptance Criteria

- [ ] Runtime compiles and builds with **no** security provider.
- [ ] Runtime compiles and builds with authentication and authorization providers via `.with_security(authn, authz)`.
- [ ] No mandatory dependency on a present `SecurityContext` anywhere in the runtime core.
- [ ] No ambient or global security state (grep gates pass: no `static`/`lazy_static!`/
      `OnceCell`/`task-local`/`thread-local` for security).
- [ ] All security propagation is explicit (inherits CORE-010A verification).
- [ ] Domain handlers execute correctly when `security == None`.
- [ ] Identity-requiring components fail explicitly with `SecurityError::CapabilityNotEnabled`
      (`Result`, never panic) when the capability is not enabled.
- [ ] The two runtime security states (`None` = not installed, `Some(SecurityContext)` =
      installed) are unambiguously distinguishable. (`Anonymous` deferred to CORE-011.)
- [ ] No `SecurityContext` type migration is required — the existing struct is unchanged.
- [ ] `cargo fmt`, `cargo clippy --all-targets --all-features`, `cargo test --workspace` pass.

## Relationship to CORE-010A

CORE-010A removed ambient context access and made `ServiceContext` an explicit-propagation
value type. CORE-010B builds directly on that baseline and inherits, without weakening:

- No `task-local` / `thread-local` / ambient state.
- Explicit ownership/parameter propagation for all execution inputs.
- Verification by grep gates over the workspace.

CORE-010B extends those rules to the security provider and security context specifically.

## Future Capabilities Enabled

| Future change | What CORE-010B unlocks |
|---------------|------------------------|
| CORE-011 (JWT authentication) | A concrete `AuthenticationProvider` wired via `.with_security(...)`; produces `Authenticated(principal)`. |
| CORE-012 (Authorization / RBAC) | Authorization that distinguishes "not installed" from "anonymous" from "authenticated" without ambiguity. |
| Service-to-service auth | Service principals as a future `PrincipalKind`, anonymous-vs-unconfigured cleanly separated. |
| Multi-tenant security scope | Scope data attached to `Authenticated` without affecting unconfigured deployments. |

All of these are **additive** — they install a capability rather than changing the core's
assumption that security may be absent.

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Brief assumes `ExecutionContext` exists; it does not | Confirmed | Open Decision #1 resolved before design; pick wrapper vs in-place evolution explicitly. |
| Accidental global/ambient provider during builder design | Med | Inherit CORE-010A grep gates; add a security-specific gate. |
| Over-scoping into actual auth logic (JWT/RBAC) | Med | Non-Goals are explicit; acceptance criteria contain no concrete-provider behavior. |

## Rollback Plan

Pure architectural refactor on a feature branch with no data/schema migration. Revert the
branch to restore the struct `SecurityContext`, the prior context field, and the absence
of the public builder. `cargo test --workspace` on `develop` confirms restored state.

## Dependencies

- CORE-010A (explicit, non-ambient context) — completed and archived; CORE-010B inherits
  its rules and verification gates.
