# Service SDK — Context Propagation Specification

## Purpose

Defines the requirements for `ServiceContext` lifecycle, propagation, and proxy dispatch
within `crates/service-sdk`. This spec was created as part of CORE-010A and captures the
explicit-context invariant after removal of all ambient context APIs.

---

## Requirements

### Requirement: No Ambient Context APIs

The system MUST NOT provide any mechanism to obtain a `ServiceContext` through ambient state.
Specifically, `ServiceContext::current()`, `ServiceContext::scope(...)`, and any
`tokio::task_local!` declaration for `ServiceContext` MUST NOT exist in the codebase.

`ServiceContext` MUST NOT be stored or retrieved through any of the following mechanisms:
task-local storage, thread-local storage, `OnceCell`, `LazyLock`, static global registries,
or any indirect ambient lookup abstraction. The only valid access model is direct ownership:
the caller received `ServiceContext` as a parameter, constructor argument, or owned field,
and passes it forward explicitly.

The following patterns are EXPLICITLY FORBIDDEN at the workspace level:

| Forbidden Pattern | Reason |
|---|---|
| `ServiceContext::current()` | Hidden dependency; breaks propagation across spawn boundaries |
| `ServiceContext::scope(...)` | Implicit scoping violates explicit-dependency invariant |
| `task_local! { static CURRENT_CONTEXT: ServiceContext }` | Task-local state hides execution inputs |
| `thread_local! { ... ServiceContext ... }` | Non-deterministic propagation across threads |
| `OnceCell<ServiceContext>` / `LazyLock<ServiceContext>` | Singleton ambient context |
| Global Context Registry for `ServiceContext` | Singleton ambient context |
| Proxy-owned hidden `ServiceContext` field | Hides dependency from call-site |
| Runtime-owned hidden `ServiceContext` field | Hides dependency from call-site |
| Interceptor-owned hidden `ServiceContext` field | Hides dependency from call-site |
| Context Provider abstraction with ambient lookup | Ambient access under different name |

#### Scenario: Ambient API removal verified at compile time

- GIVEN the workspace compiles successfully
- WHEN `grep -rn "ServiceContext::current\|ServiceContext::scope\|CURRENT_CONTEXT" crates/` is run
- THEN zero matches are returned

#### Scenario: Task-local declaration absent

- GIVEN the workspace compiles successfully
- WHEN the file `crates/service-sdk/src/context/mod.rs` is inspected
- THEN no `tokio::task_local!` block declaring a `ServiceContext` binding is present

#### Scenario: Broader ambient storage patterns absent from production code

- GIVEN the workspace compiles successfully
- WHEN the following commands are run against `crates/` with `--type rust`:
  - `rg "task_local!"`
  - `rg "thread_local!"`
  - `rg "OnceCell" crates/ --type rust`
  - `rg "LazyLock" crates/ --type rust`
  - `rg "once_cell" crates/ --type rust`
  - `rg "lazy_static" crates/ --type rust`
- THEN no results reference `ServiceContext` in any match
- AND the only `LazyLock` match (`crates/domain/src/actor.rs` — `actor_id!` macro for `ActorId` interning) is unrelated to context propagation and is explicitly exempt

---

### Requirement: Explicit Context in Proxy Dispatch

Generated proxy methods MUST receive `ServiceContext` as an explicit parameter. The macro
`crates/service-sdk-macros/src/lib.rs` MUST generate forwarding methods with the signature:

```rust
async fn <method>(&self, ctx: ServiceContext, request: <RequestType>) -> Result<<ResponseType>>
```

For an operation marked `#[tenant_scoped]`, tenant enforcement MUST be called as the fallible
`rt.enforce_tenant(&mut ctx)?` (CORE-008A AD-009) — a `mut` binding is required because
`enforce_tenant` is the sole writer of the context's resolver-derived canonical tenant on
success. This call MUST be placed before the inner operation call, so the operation body is
never entered when enforcement fails (FR-009). An operation with no `#[tenant_scoped]` marker
keeps the pre-existing best-effort call, whose `Result` is discarded (D1's valid tenant-less
system/single-tenant execution mode). Interceptor hooks MUST receive the context explicitly:
`interceptor.on_request(&ctx)`, `interceptor.on_response(&ctx)`, `interceptor.on_error(&ctx)`.
No ambient read (`current()`) or scope wrap (`scope()`) is permitted inside the generated body.

#### Scenario: Generated proxy compiles with explicit ctx parameter

- GIVEN a service trait annotated with the proxy derive macro
- WHEN the macro expands the forwarding method
- THEN the generated method accepts `ctx: ServiceContext` as the first user-visible parameter
- AND, for a `#[tenant_scoped]` method, the body calls `rt.enforce_tenant(&mut ctx)?` before
  forwarding the request

#### Scenario: Interceptors receive context from parameter, not ambient state

- GIVEN a proxy dispatch flow with one or more interceptors registered
- WHEN a service method is called with an explicit `ServiceContext`
- THEN `on_request`, `on_response`, and `on_error` hooks each receive `&ctx` sourced from the
  parameter — no call to `ServiceContext::current()` occurs inside the generated body

#### Scenario: Tenant enforcement behavior preserved

- GIVEN a `#[tenant_scoped]` operation and a `ServiceContext` whose authenticated `Principal`
  has `tenant_id = "tenant-a"`
- WHEN a proxy-generated method is called with that context
- THEN `enforce_tenant` derives the canonical tenant from the `Principal`, exposed via
  `ctx.canonical_tenant()`, before the operation body runs
- AND a context whose caller-supplied tenant hint disagrees with the Principal's tenant fails
  the call with `SecurityError::TenantMismatch` before the operation body is entered — the
  fallible check can actually prevent execution, not merely log or ignore the disagreement

---

### Requirement: Explicit Propagation Through Spawned Tasks

Spawned tasks MUST receive `ServiceContext` through captured ownership or explicit parameter
passing. A task MUST NOT rely on ambient propagation to access a `ServiceContext`.

#### Scenario: Context captured before spawn

- GIVEN a `ServiceContext` value in the current scope
- WHEN a new task is spawned with `tokio::spawn(async move { ... })`
- THEN the task accesses `ServiceContext` only via the captured move binding
- AND no call to `ServiceContext::current()` appears inside the async block

#### Scenario: Context passed as function argument to spawned work

- GIVEN a function `async fn do_work(ctx: ServiceContext) -> Result<()>`
- WHEN called from a spawned task or directly
- THEN the function receives `ctx` as a typed parameter visible in the function signature

#### Scenario: Spawned task invariant enforced — no ambient lookup after spawn boundary

- GIVEN a `ServiceContext` is in scope before a `tokio::spawn` call
- WHEN the spawned task requires the context
- THEN the context is captured via `async move { ... }` or passed as an argument
- AND the spawned task body contains no call to any ambient lookup method
- AND this is verified by: `rg "ServiceContext::current|ServiceContext::scope|CURRENT_CONTEXT" crates/ --type rust` returning zero matches

---

### Requirement: Test Suite Uses Explicit Construction Only

All tests in the `crates/service-sdk/tests/` directory MUST construct and propagate
`ServiceContext` explicitly. Tests MUST NOT call `ServiceContext::current()` or
`ServiceContext::scope(...)` as test harness or assertion helpers.

#### Scenario: Rewritten context_scope tests pass

- GIVEN the file `tests/context_scope.rs` no longer references `scope()` or `current()`
- WHEN `cargo test --workspace` is run
- THEN all tests in that file pass with green status

#### Scenario: Rewritten context_propagation tests pass

- GIVEN the file `tests/context_propagation.rs` no longer references `scope()` or `current()`
- WHEN `cargo test --workspace` is run
- THEN all tests in that file pass with green status

#### Scenario: Rewritten context_cross_service tests pass

- GIVEN the file `tests/context_cross_service.rs` no longer references `scope()`
- WHEN `cargo test --workspace` is run
- THEN all tests in that file pass with green status

---

### Requirement: Build and Lint Gates Pass

The workspace MUST pass all three gates after the change is applied.

#### Scenario: cargo fmt is clean

- GIVEN the full workspace source after the change
- WHEN `cargo fmt --check` is run
- THEN exit code is 0 (no formatting differences)

#### Scenario: cargo clippy passes with no errors

- GIVEN the full workspace source after the change
- WHEN `cargo clippy --all-targets --all-features` is run
- THEN exit code is 0 with no error-level diagnostics

#### Scenario: full workspace test suite passes

- GIVEN the full workspace source after the change
- WHEN `cargo test --workspace` is run
- THEN exit code is 0 and all tests pass

---

### Requirement: ServiceContext Is Part of the Public Operation Contract

`ServiceContext` MUST appear as the first user-visible parameter in every generated operation
signature. This is an intentional, permanent API contract established by CORE-010A. Consumers
of generated service proxies MUST pass an explicit `ServiceContext` at every call site.

This is NOT an implementation detail — it is the public interface. Any code generation,
documentation tooling, or client SDK wrapping these services MUST preserve this parameter.

#### Scenario: Operation signature communicates context dependency

- GIVEN a service operation `fn charge(ctx: ServiceContext, amount: u64) -> Result<String>`
- WHEN a consumer calls the proxy
- THEN the consumer MUST construct or receive a `ServiceContext` before making the call
- AND the compiler enforces this — no call without an explicit `ctx` argument compiles

#### Migration guidance

Services that previously relied on `ServiceContext::current()` inside their implementations
must be updated to receive context as a parameter. The migration pattern is:

**Before (ambient — removed):**
```rust
async fn charge(&self, amount: u64) -> Result<String> {
    let ctx = ServiceContext::current().unwrap_or_default();
    // ...
}
```

**After (explicit — required):**
```rust
async fn charge(&self, ctx: ServiceContext, amount: u64) -> Result<String> {
    // ctx is explicit — no lookup needed
    // ...
}
```

---

### Requirement: ServiceContext security accessor methods

`ServiceContext` MUST expose the existing `security()` method (returns `Option<&SecurityContext>`) for optional access (Layer 1), and MUST additionally expose `require_security()` for fail-fast controlled access (Layer 2):

```rust
// Layer 1 — existing, unchanged
pub fn security(&self) -> Option<&SecurityContext>;

// Layer 2 — new
pub fn require_security(&self) -> Result<&SecurityContext, SecurityError>;
```

`security()` returns `None` when the capability is not installed. `require_security()` returns `Err(SecurityError::CapabilityNotEnabled)` when not installed — never panics. Both read from the internal `security: Option<Arc<SecurityContext>>` field. No ambient or global state is consulted.

#### Scenario: Optional access returns None for unconfigured runtime

- GIVEN a `ServiceContext` with `security == None`
- WHEN `security()` is called
- THEN `None` is returned

#### Scenario: Optional access returns Some for configured runtime

- GIVEN a `ServiceContext` with `security == Some(Arc::new(security_ctx))`
- WHEN `security()` is called
- THEN `Some(&SecurityContext)` is returned referencing the expected security context

#### Scenario: Required access fails when security not installed

- GIVEN a `ServiceContext` with `security == None`
- WHEN `require_security()` is called
- THEN `Err(SecurityError::CapabilityNotEnabled)` is returned

#### Scenario: Required access succeeds when security installed

- GIVEN a `ServiceContext` with `security == Some(Arc::new(security_ctx))`
- WHEN `require_security()` is called
- THEN `Ok(&SecurityContext)` is returned

### Requirement: RuntimeBuilder optional security registration

`RuntimeBuilder` MUST support optional security provider registration:

```rust
pub fn with_security(
    self,
    authn: Arc<dyn AuthenticationProvider>,
    authz: Arc<dyn AuthorizationProvider>,
) -> Self;
```

`build()` MUST succeed whether or not `.with_security()` was called. When `.with_security()` IS called, the runtime registers the authentication and authorization providers and is marked as security-capable. Creating a `SecurityContext` requires an authenticated `Principal` — without a future authentication entrypoint (CORE-011), `ServiceContext.security` remains `None` and no `SecurityContext` is fabricated. When NOT called, no providers are registered. No global or ambient provider state is introduced — capability is instance-scoped to the runtime.

#### Scenario: Registering providers does not create a SecurityContext

- GIVEN `RuntimeBuilder::new().with_security(authn_provider, authz_provider).build()`
- WHEN a new `ServiceContext::new()` is created
- THEN `service_ctx.security() == None` (no `SecurityContext` is fabricated; only providers are registered)

#### Scenario: Build without security succeeds

- GIVEN `RuntimeBuilder::new()`
- WHEN `.build()` is called without calling `.with_security()`
- THEN a valid `Runtime` is returned with no security configured
- AND every `ServiceContext` in the runtime has `security == None`

#### Scenario: Build with security succeeds

- GIVEN `RuntimeBuilder::new()`
- WHEN `.with_security(authn_provider, authz_provider).build()` is called
- THEN a valid `Runtime` is returned with security configured
- AND the runtime stores the registered providers
- AND newly created `ServiceContext` values have `security == None` (no `SecurityContext` is fabricated until CORE-011)

#### Scenario: No global security state

- GIVEN a `Runtime` built with `.with_security()`
- WHEN grep gates are run for `static SECURITY_PROVIDER`, `lazy_static!`, `OnceCell`, `task_local!` in `crates/service-sdk/src/`
- THEN zero matches related to security or provider state are returned

---

### Requirement: RuntimeInner Not Publicly Constructible

`RuntimeInner::new()` MUST be `pub(crate)`. Any `Default` implementation for `RuntimeInner` MUST be either removed or `pub(crate)` — it MUST NOT be `pub`. No public constructor for `RuntimeInner` may exist outside the `service-sdk` crate.

The only construction path reachable from outside `crates/service-sdk` is `RuntimeBuilder::build()` (via `RuntimeInner::new_with_logger`, already `pub(super)`).

#### Scenario: External crate cannot construct RuntimeInner directly

- GIVEN a crate outside `service-sdk` (e.g. an application or integration test crate depending on `service-sdk` as a library)
- WHEN that crate attempts to call `RuntimeInner::new(...)` or `RuntimeInner::default()`
- THEN compilation fails with a visibility error

#### Scenario: RuntimeBuilder::build() remains the sole construction path

- GIVEN the `service-sdk` crate after this change
- WHEN `rg "RuntimeInner\s*\{|RuntimeInner::new\(|RuntimeInner::default\(\)" crates/` is run
- THEN every match resolves to `RuntimeBuilder::build()`'s internal call chain (`new_with_logger`) or a `#[cfg(test)]` / `pub(crate)` test helper inside `service-sdk`
- AND no match originates from a crate other than `service-sdk`

#### Scenario: In-crate test helper stays crate-private

- GIVEN a test inside `crates/service-sdk` needs a `RuntimeInner` state not reachable through `RuntimeBuilder::build()`
- WHEN such a helper is added
- THEN it is gated `#[cfg(test)]` and/or `pub(crate)`
- AND it is never re-exposed as `pub`

---

### Requirement: RuntimeBuilder::build() Behavior Is Unchanged

Restricting `RuntimeInner`'s constructors MUST NOT alter the observable behavior of `RuntimeBuilder::build()` for correctly-built runtimes: logger wiring, ordered teardown registration, and security-provider installation behave identically before and after this change.

#### Scenario: Logger wiring unchanged

- GIVEN a `RuntimeBuilder` configured with `.with_logger(logger)`
- WHEN `.build()` is called
- THEN the resulting `Runtime`'s `RuntimeInner::logger()` returns the same logger instance as before this change

#### Scenario: Teardown ordering unchanged

- GIVEN a `RuntimeBuilder` with infrastructure registered that pushes teardown entries
- WHEN `.build()` is called and the runtime is later shut down
- THEN teardown entries drain in the same reverse-construction order as before this change

#### Scenario: Security provider installation unchanged

- GIVEN a `RuntimeBuilder` configured with `.with_security(authn, authz)`
- WHEN `.build()` is called
- THEN `RuntimeInner::authorization_provider()` returns the same provider as before this change

#### Scenario: Build without security still succeeds

- GIVEN a `RuntimeBuilder` with no `.with_security(...)` call
- WHEN `.build()` is called
- THEN a valid `Runtime` is returned with `security_providers == None`, identical to pre-change behavior

---

### Requirement: Reference Host Example Materializes Configuration Through kit-config

The reference host example (`examples/reference-app`) MUST materialize
application configuration through `kit-config` at its composition root,
before any `RuntimeBuilder` construction begins. It MUST hand `RuntimeBuilder`
only materialized configuration, delivered through `ConfigurationProvider` —
never a raw configuration source (unparsed file, raw environment map, or
config-loading intermediate).

This confirms, with a real example, the frozen constraint already established
in `openspec/changes/archive/2026-07-03-CORE-016-app-config-model/spec.md:148`.
It does not redefine that constraint.

#### Scenario: build_runtime wires real kit-config output

- GIVEN `examples/reference-app` depends on `kit-config` as a git dependency
- WHEN `build_runtime()` executes
- THEN configuration is materialized via `kit-config`, delivered to
  `RuntimeBuilder` through `ConfigurationProvider`, and a logger derived from
  it is installed via `.with_logger(...)`

#### Scenario: No raw configuration source reaches RuntimeBuilder

- GIVEN the reference-app composition root after this change
- WHEN every value passed into `RuntimeBuilder`'s builder methods is reviewed
- THEN none of them is an unparsed config source — only materialized
  configuration delivered via `ConfigurationProvider` reaches it

#### Scenario: Existing framework contract remains untouched

- GIVEN `crates/service-sdk`'s `ConfigurationProvider`, `build_logger`, and
  `RuntimeBuilder` implementations
- WHEN this change is applied
- THEN `crates/service-sdk` and `crates/service-sdk/examples/logging_bootstrap.rs`
  show zero diff

---

## Tenant Enforcement & Cross-Tenant Access (CORE-008A)

This section describes the canonical tenant model, resolution authority, fail-closed
enforcement, and authorization-gated cross-tenant access built by CORE-008A and
subsequently closed out (FR-006 consumption gap) by later work. It supersedes the
narrower, now-stale "TenantResolver does not re-validate..." section previously here,
which covered only one delta (CORE-024) against an already-obsolete `resolve()`
signature.

**Resolution seam.** `TenantResolver::resolve` (`crates/service-sdk/src/runtime/tenant.rs`)
is the single algorithm mandated below. It takes one argument, a closed
`EstablishedTenantFacts<'a>` value:

```rust
pub(crate) struct EstablishedTenantFacts<'a> {
    security: Option<&'a SecurityContext>,
    hint: Option<&'a str>,
    cross_tenant_grant: Option<&'a CrossTenantGrant>,
}

impl<'a> EstablishedTenantFacts<'a> {
    pub(crate) fn new(
        security: Option<&'a SecurityContext>,
        hint: Option<&'a str>,
        cross_tenant_grant: Option<&'a CrossTenantGrant>,
    ) -> Self;
}

impl TenantResolver {
    pub(crate) fn resolve(
        &self,
        facts: EstablishedTenantFacts<'_>,
    ) -> Result<CanonicalTenant, SecurityError>;
}
```

`RuntimeInner::enforce_tenant` gathers `facts` from `ServiceContext` (`ctx.security()`,
`ctx.tenant_hint()`, `ctx.cross_tenant_grant()`) and calls `resolve` once per
tenant-scoped operation. **AD-013 (Fact Establishment vs. Policy Evaluation)** governs
this seam: `TenantResolver::resolve` is a Policy Evaluator — it derives its decision
exclusively from the closed, immutable `facts` it was handed, and never itself fetches,
queries, or authorizes anything during evaluation. Establishing a cross-tenant grant is
a separate, upstream Fact Establishment step (`RuntimeInner::issue_cross_tenant_permit`
+ `ServiceContext::with_cross_tenant_access`) that must complete before `resolve` ever
runs — see FR-006 below.

---

### Requirement: Tenant-Scoped Fail-Closed Enforcement Is Operation-Level, Not Global (FR-001)

Tenant-scoped operations MUST fail closed when the canonical tenant cannot be resolved
and validated for that operation. A valid tenant-less system/single-tenant execution
mode MUST remain available; fail-closed enforcement applies only to operations
classified as tenant-scoped, not to every operation in the runtime. Classification is
the `#[tenant_scoped]` macro attribute (see "Explicit Context in Proxy Dispatch" above,
which resolves the mechanism this requirement's archived form left open) — unmarked
operations never call `enforce_tenant` at all.

#### Scenario: Tenant-scoped operation fails closed without resolvable tenant

- GIVEN an operation annotated `#[tenant_scoped]`
- WHEN it is invoked and `RuntimeInner::enforce_tenant` cannot resolve a canonical
  tenant for the call
- THEN the call fails with an explicit `SecurityError` and the operation is not executed

#### Scenario: Non-tenant-scoped operation is unaffected by missing tenant

- GIVEN an operation with no `#[tenant_scoped]` marker, running in a valid
  system/single-tenant execution mode
- WHEN it is invoked with no tenant present
- THEN the call proceeds and executes normally; no tenant error occurs

---

### Requirement: Principal Is the Canonical Tenant Authority on the Authenticated Path (FR-002)

When a request is authenticated (a `Principal` exists via JWT/API key/OIDC),
`Principal.tenant_id` (`Option<TenantId>`, already validated at `Principal`
construction) MUST be treated as canonical. `TenantResolver::resolve` MUST derive the
tenant visible to the service operation from `Principal.tenant_id` automatically — it
MUST NOT re-validate that value via `TenantId::new()` or any equivalent; it is cloned
directly into the returned `CanonicalTenant`. If a caller-supplied hint
(`facts.hint`) is present, non-blank after trimming, and disagrees with
`Principal.tenant_id`, the call MUST fail with `SecurityError::TenantMismatch` — the
resolver MUST NOT silently prefer either value (unless FR-006's cross-tenant grant
covers exactly that hint's destination — see below). A blank or whitespace-only hint is
treated as absent, not as a mismatch. If the authenticated Principal carries no tenant
claim at all (`Principal.tenant_id` is `None`), the resolver MUST NOT treat any
caller-supplied hint as a substitute for it — the call MUST fail closed with
`SecurityError::MissingContext`, regardless of whether a hint is present or absent, and
this check MUST be evaluated before the hint-agreement check (a present-but-conflicting
hint must never be evaluated against an absent Principal tenant claim).

#### Scenario: Derivation from Principal succeeds without manual tenant assignment

- GIVEN a `SecurityContext` wrapping a `Principal` with `tenant_id = Some(TenantId::new("tenant-a").unwrap())` and no conflicting hint
- WHEN `resolver.resolve(EstablishedTenantFacts::new(Some(&security), None, None))` is called
- THEN `Ok(CanonicalTenant::scoped(tenant))` is returned where `tenant` is a clone of the Principal's `TenantId` — no call to `TenantId::new()` occurs during this resolution

#### Scenario: Caller-supplied tenant conflicting with Principal is a hard error

- GIVEN a `SecurityContext` wrapping a `Principal` with `tenant_id = Some(TenantId::new("tenant-a").unwrap())`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(Some(&security), Some("tenant-b"), None))` is called
- THEN `Err(SecurityError::TenantMismatch { expected: "tenant-a", actual: "tenant-b" })` is returned; neither value is silently chosen

#### Scenario: Blank hint is treated as absent, not a mismatch

- GIVEN a `SecurityContext` wrapping a `Principal` with `tenant_id = Some(TenantId::new("tenant-a").unwrap())`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(Some(&security), Some(""), None))` is called
- THEN `Ok(CanonicalTenant::scoped(tenant))` is returned for `"tenant-a"` — a blank hint never triggers `TenantMismatch`

#### Scenario: Authenticated Principal without a tenant claim fails closed regardless of a caller-supplied hint

- GIVEN a `SecurityContext` wrapping a `Principal` with `tenant_id = None`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(Some(&security), Some("tenant-x"), None))` is called
- THEN `Err(SecurityError::MissingContext)` is returned; the hint is never used as a substitute for the missing Principal tenant claim

#### Scenario: No validation call reachable on the Principal-derived path (structural)

- GIVEN the source of `TenantResolver::resolve()`
- WHEN the Principal-derived branch (the `Some(security)` match arm) is inspected
- THEN no call to `TenantId::new(...)` appears on the path that handles `security.principal().tenant_id` — the only operation performed on that value is a clone into `CanonicalTenant::scoped(...)`

**Tests**: `tenant::tests::resolve_authenticated_hint_absent_resolves_to_principal_tenant`, `tenant::tests::resolve_authenticated_hint_agrees_resolves_to_principal_tenant`, `tenant::tests::resolve_authenticated_blank_hint_resolves_to_principal_tenant`, `tenant::tests::resolve_authenticated_hint_disagrees_is_tenant_mismatch`, `tenant::tests::resolve_authenticated_no_principal_tenant_fails_closed_even_with_hint`, `tenant::tests::resolve_authenticated_no_principal_tenant_fails_closed_without_hint`. The "no re-validation" property is verified by code inspection at review time, not a runtime assertion — once `Principal.tenant_id` is `Option<TenantId>`, there is no invalid value a unit test could construct to distinguish "validated once" from "re-validated every call".

#### Out of Scope for This Requirement

- **No change to `ServiceContext.tenant_id` / `tenant_hint()`** (`crates/service-sdk/src/context/mod.rs`). That is a deliberately-raw ingress hint per AD-011, a different concept from the authenticated Principal's tenant claim. `testkit::TestContextBuilder`, which builds this hint, is likewise untouched.
- **`TenantEnforcementMode` variants and the hint-mismatch/agreement decision logic are unchanged** by the CORE-024 validate-once delta — only the source of validation for the Principal-derived value was removed, not the resolution algorithm's branches.

---

### Requirement: Explicit System/Internal Request Mode (FR-003)

An unauthenticated call (no `Principal`, `facts.security == None`) MUST be routed
through a distinct, explicit system/internal branch of `TenantResolver::resolve` rather
than being treated as a variant of FR-002's mismatch case. A caller-supplied hint is
valid in this mode only when the runtime was configured with
`TenantEnforcementMode::AllowSystemInternal` (via
`RuntimeBuilder::with_tenant_enforcement_mode`; the default is `AuthenticatedOnly`).
This is the ONE remaining raw-string parse in `resolve()`: `TenantId::new(hint.trim())`
— the hint is trimmed of leading/trailing whitespace before validation so incidental
whitespace (e.g. from a transport header) does not mint a `TenantId` that silently
fails to `==` a clean one downstream.

#### Scenario: Internal mode accepts caller-supplied tenant when explicitly permitted

- GIVEN `TenantResolver::new(TenantEnforcementMode::AllowSystemInternal)` and no `SecurityContext`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(None, Some("tenant-c"), None))` is called
- THEN `Ok(CanonicalTenant::scoped(tenant))` is returned for `"tenant-c"`, without being treated as a `TenantMismatch`

#### Scenario: Internal-mode hint is trimmed before validation

- GIVEN `TenantResolver::new(TenantEnforcementMode::AllowSystemInternal)` and no `SecurityContext`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(None, Some(" tenant-c "), None))` is called
- THEN `Ok(CanonicalTenant::scoped(tenant))` is returned where `tenant.as_str() == "tenant-c"` — the stored value is trimmed, not the raw untrimmed hint

#### Scenario: Internal mode rejects tenant when not permitted

- GIVEN `TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly)` (the default) and no `SecurityContext`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(None, Some("tenant-c"), None))` is called
- THEN `Err(SecurityError::MissingContext)` is returned — the call does not proceed as an authenticated-tenant call; it is handled per FR-004

**Tests**: `tenant::tests::resolve_unauthenticated_allow_system_internal_with_hint_resolves_to_hint`, `tenant::tests::resolve_unauthenticated_allow_system_internal_trims_whitespace_in_hint`, `tenant::tests::resolve_unauthenticated_authenticated_only_mode_fails_closed`.

---

### Requirement: Neither Authenticated Nor Internal-Permitted Fails Closed (FR-004)

A call that is neither authenticated (no `Principal`) nor covered by a
runtime-permitted system/internal mode MUST fail with `SecurityError::MissingContext`
before a tenant-scoped operation body executes. (The archived spec anticipated a
possible separate `MissingAuthentication` variant; the shipped `SecurityError` enum —
`crates/security-sdk/src/error/mod.rs` — has no such variant. `MissingContext` alone
covers this case, which the archived spec's own wording already permitted: "the three
conditions may surface through `RuntimeError`, `ServiceError`, `SecurityError`, or any
combination design.md chooses.")

#### Scenario: Unauthenticated, non-internal call is rejected

- GIVEN `TenantResolver::new(TenantEnforcementMode::AllowSystemInternal)` and no `SecurityContext`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(None, None, None))` is called
- THEN `Err(SecurityError::MissingContext)` is returned, and the operation body is never entered

**Tests**: `tenant::tests::resolve_unauthenticated_allow_system_internal_without_hint_fails_closed`.

---

### Requirement: CrossTenantPermit Requires Authorized Capability (FR-005)

`CrossTenantPermit` MUST be issued only after `AuthorizationProvider` confirms the
requesting Principal holds an explicit cross-tenant capability. The current mechanism:
`RuntimeInner::issue_cross_tenant_permit(&self, ctx: &ServiceContext, destination: TenantId)`
(`crates/service-sdk/src/runtime/runtime_builder.rs`) builds a `Resource { kind: "tenant", id: Some(destination) }`
/ `Action("cross-tenant-access")` request and calls `authorize_in_context` against the
configured `AuthorizationProvider`. Being authorized for the target resource/action
under a different action name is never checked — only this specific
`"tenant:cross-tenant-access"` capability grants a permit. A `Deny` decision maps to
`SecurityError::CrossTenantDenied`; if no `AuthorizationProvider` is configured, the
call fails with `SecurityError::CapabilityNotEnabled`.

#### Scenario: Permit denied for principal without cross-tenant capability

- GIVEN a Principal whose `AuthorizationProvider` denies the `"tenant:cross-tenant-access"` request
- WHEN `issue_cross_tenant_permit` is called for a destination tenant
- THEN `Err(SecurityError::CrossTenantDenied { .. })` is returned and no `CrossTenantPermit` is issued

#### Scenario: No provider configured fails closed

- GIVEN a runtime with no `AuthorizationProvider` configured
- WHEN `issue_cross_tenant_permit` is called
- THEN `Err(SecurityError::CapabilityNotEnabled)` is returned

**Tests**: `runtime_builder::tests::issue_cross_tenant_permit_denied_without_capability`, `runtime_builder::tests::issue_cross_tenant_permit_denied_even_with_resource_action_alone`, `runtime_builder::tests::issue_cross_tenant_permit_without_provider_is_capability_not_enabled`.

---

### Requirement: Authorized Cross-Tenant Access Succeeds (FR-006)

A Principal holding the `"tenant:cross-tenant-access"` capability, confirmed via
`AuthorizationProvider`, MUST be able to obtain a `CrossTenantPermit` and successfully
execute a cross-tenant operation using it. Per **AD-013**, this is wired as Fact
Establishment feeding Policy Evaluation, not as a callback performed during resolution:

1. `RuntimeInner::issue_cross_tenant_permit` mints a `CrossTenantPermit { destination, issued_to }`
   only on an `Allow` decision (FR-005).
2. `ServiceContext::with_cross_tenant_access(&permit)` attaches it, storing a
   `CrossTenantGrant` (`crates/service-sdk/src/runtime/tenant.rs`) — an AD-013
   Established Fact — scoped to exactly the permit's `destination`. A permit issued for
   `tenant-b` can never authorize a grant for `tenant-c`.
3. `RuntimeInner::enforce_tenant` gathers `EstablishedTenantFacts` (`ctx.security()`,
   `ctx.tenant_hint()`, `ctx.cross_tenant_grant()`) and hands them to
   `TenantResolver::resolve` as a single closed value.
4. Inside `resolve`, ONLY when an authenticated hint disagrees with the Principal's own
   tenant AND a `CrossTenantGrant` is present whose `destination` exactly matches the
   (trimmed) hint, resolution succeeds with `CanonicalTenant::scoped(grant.destination().clone())`
   instead of a hard `TenantMismatch`. `resolve` never fetches, checks, or re-derives
   the grant itself — it only reads the Established Fact it was handed (AD-013). A
   grant scoped to a different destination than the hint still produces
   `TenantMismatch`; an unused grant (hint absent or agreeing) has no effect.

#### Scenario: Authorized cross-tenant access succeeds end to end

- GIVEN a Principal authenticated on `"tenant-a"`, and an `AuthorizationProvider` that allows `"tenant:cross-tenant-access"`
- WHEN the Principal calls `issue_cross_tenant_permit` for `"tenant-b"`, attaches the resulting permit via `ctx.with_cross_tenant_access(&permit).with_tenant_id("tenant-b")`, and `RuntimeInner::enforce_tenant(&mut ctx)` runs
- THEN `enforce_tenant` returns `Ok(())`, and `ctx.canonical_tenant().and_then(CanonicalTenant::tenant_id)` is `Some("tenant-b")` — not rejected as a tenant violation

#### Scenario: Grant scoped to a different destination than the hint still mismatches

- GIVEN a Principal authenticated on `"tenant-a"` holding a grant for `"tenant-c"`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(Some(&security), Some("tenant-b"), Some(&grant)))` is called
- THEN `Err(SecurityError::TenantMismatch { expected: "tenant-a", actual: "tenant-b" })` is returned — the grant does not act as a blanket cross-tenant switch

#### Scenario: Unused grant does not affect an ordinary same-tenant call

- GIVEN a Principal authenticated on `"tenant-a"` holding a grant for `"tenant-b"`, and no hint supplied
- WHEN `resolver.resolve` is called
- THEN resolution succeeds with `"tenant-a"`, as if no grant existed

**Tests**: `runtime_builder::tests::enforce_tenant_succeeds_for_authorized_cross_tenant_grant` (full issued → attached → consumed → operation-succeeds flow), `runtime_builder::tests::issue_cross_tenant_permit_allowed_yields_destination_scoped_permit`, `tenant::tests::resolve_authorized_cross_tenant_grant_succeeds`, `tenant::tests::resolve_authorized_cross_tenant_grant_succeeds_with_whitespace_in_hint`, `tenant::tests::resolve_grant_for_different_destination_is_still_tenant_mismatch`, `tenant::tests::resolve_unused_grant_does_not_affect_hint_absent_resolution`, `tenant::tests::resolve_redundant_grant_matching_own_tenant_resolves_normally`, `context::tests::is_cross_tenant_allowed_for_matches_only_the_issued_destination`.

---

### Requirement: Runtime Is Transport-Independent for Tenant Resolution (FR-007)

`TenantResolver::resolve` and `RuntimeInner::enforce_tenant` MUST consume only
transport-neutral inputs (`EstablishedTenantFacts`: an already-produced
`SecurityContext`, an optional `&str` hint, an optional `&CrossTenantGrant`). Neither
MUST depend on any transport-specific mechanism (HTTP headers, gRPC metadata, or any
other transport concept) to obtain or validate the tenant.

#### Scenario: Runtime enforcement contains no transport-specific dependency

- GIVEN `crates/service-sdk/src/runtime/tenant.rs` and `runtime_builder.rs`'s `enforce_tenant`
- WHEN reviewed for dependencies
- THEN neither references any HTTP, gRPC, or other transport-specific type, or header/metadata extraction logic — only `SecurityContext`, `&str`, and `CrossTenantGrant`

---

### Requirement: Exactly One Canonical In-Runtime Tenant Representation (FR-008)

Exactly one representation of tenant MUST be canonical inside the runtime at the point
an operation executes: `CanonicalTenant` (`crates/service-sdk/src/runtime/tenant.rs`).
It wraps a private `Repr` enum (`Scoped(TenantId)` for a resolved tenant, `Systemwide`
for D1's valid tenant-less mode); its constructors are `pub(super)`, reachable only
within `crate::runtime`, so only `TenantResolver::resolve` may mint one. `Principal.tenant_id`,
`ServiceContext.tenant_id` (the ingress hint), and `ClaimSet::tenant()` are
ingress/legacy carriers only — none is independently authoritative for the
same operation at execution time. `Principal.tenant_id` is the authoritative
*input* on the authenticated path; `TenantResolver`'s output is the
authoritative *runtime* value; `ServiceContext.tenant_id` is demoted to a
non-authoritative ingress hint (read via `ctx.tenant_hint()`).

(Previously: also listed domain `ExecutionContext` among ingress/legacy tenant
carriers. That type is deleted by this change and no longer exists.)

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

### Requirement: Tenant Enforcement Is Fallible and Aborts Before the Operation Body (FR-009)

Unchanged in substance since archival. This is already the enforced contract described
above under "Explicit Context in Proxy Dispatch" (`rt.enforce_tenant(&mut ctx)?` called
before the inner operation, per AD-009) and **INV-003** ("Tenant Enforcement
Preserved"). No further requirement is added here; see those sections and their
scenarios ("Tenant enforcement behavior preserved") for the acceptance contract.

---

### Requirement: ServiceContext Is Not a Parallel Writable Tenant Authority (FR-010)

On the authenticated path, the service-visible tenant MUST be derived per FR-002, not
independently settable by arbitrary code holding a `ServiceContext`. `ServiceContext.tenant_id`
remains a `pub` field (the ingress hint, per AD-011) but mutating it after resolution has
already run has no effect on enforcement: `resolved_tenant` is a separate private field,
written only by the `pub(crate)` `set_resolved_tenant`, whose sole caller is
`RuntimeInner::enforce_tenant`.

#### Scenario: Direct tenant mutation cannot override the derived, authenticated tenant

- GIVEN a `ServiceContext` whose `canonical_tenant()` was already resolved to `"tenant-a"` (derived from `Principal.tenant_id`)
- WHEN code sets `ctx.tenant_id = Some("tenant-b".into())` directly
- THEN `ctx.canonical_tenant()` still returns `"tenant-a"` — the mutated hint field is never read again for an already-resolved operation

---

### Requirement: A Canonical Tenant Is Available Before Operation Execution (FR-011)

Before a tenant-scoped operation executes, a canonical tenant value MUST be available
to the runtime for that operation. This is satisfied by the macro-generated call to
`rt.enforce_tenant(&mut ctx)?` placed before the inner operation call (see "Explicit
Context in Proxy Dispatch" above) — on the authenticated path this happens
automatically via FR-002's derivation, without the calling code manually assigning a
tenant per call.

#### Scenario: A canonical tenant is present at the start of execution without manual per-call assignment

- GIVEN an authenticated request to a `#[tenant_scoped]` operation
- WHEN the generated proxy method runs
- THEN `enforce_tenant` has already populated `ctx.canonical_tenant()` before the inner operation body executes, without the caller having set it manually

**Tests**: `runtime_builder::tests::enforce_tenant_ok_sets_canonical_tenant_on_resolvable_context`.

---

### Requirement: Tenant Error Taxonomy Is Reachable in Code (FR-012)

`SecurityError::TenantMismatch { expected, actual }`, `SecurityError::MissingContext`,
and `SecurityError::CrossTenantDenied { reason }` MUST each be distinguishable by
callers — reachable in code (`crates/security-sdk/src/error/mod.rs`), not only
referenced in documentation. `MissingContext` covers both FR-002's "no tenant claim"
case and FR-004's "neither authenticated nor internal-permitted" case; the archived
spec's own wording ("MissingAuthentication/MissingContext... may surface through
RuntimeError, ServiceError, SecurityError, or any combination") permits this
consolidation — no separate `MissingAuthentication` variant exists or is required.

#### Scenario: Each tenant failure mode is programmatically distinguishable

- GIVEN the three failure conditions defined in FR-002, FR-004, and FR-005
- WHEN each is triggered independently
- THEN a caller can `match` on `SecurityError::TenantMismatch { .. }`, `SecurityError::MissingContext`, or `SecurityError::CrossTenantDenied { .. }` respectively — no two conditions are indistinguishable

---

### Requirement: service-sdk Spec Contract Matches Enforced Behavior (FR-013)

Unchanged in intent since archival, and satisfied by this document itself: this spec
section (and "Explicit Context in Proxy Dispatch" / INV-003 above) describes the
fallible `enforce_tenant` check the code actually enforces, including the FR-006
cross-tenant consumption path that was still an open gap when CORE-008A originally
archived. No further requirement is added here.

---

### Requirement: Tenant Authority Is Immutable During Operation Execution (FR-014)

Once the canonical tenant has been established for an operation (per
FR-002/FR-003/FR-011), the tenant used for enforcement MUST remain stable for the
duration of that operation. `CanonicalTenant` has no setters, no public fields, and no
`&mut` API — it is immutable from the instant `TenantResolver::resolve` returns it
(there is no mutation point to close). `ServiceContext.resolved_tenant` is written
exactly once per operation, only by `set_resolved_tenant` (`pub(crate)`, sole caller
`enforce_tenant`); no downstream code — including a later mutation of the raw
`ctx.tenant_id` hint field on a cloned context — can alter the tenant an in-flight
operation enforces against.

#### Scenario: Downstream mutation attempts do not affect an operation already in progress

- GIVEN an operation whose `ctx.canonical_tenant()` has already been resolved
- WHEN downstream code attempts to alter `ctx.tenant_id` (the hint field) or clones `ctx`
- THEN all subsequent enforcement decisions for that operation observe the original `CanonicalTenant`, not the attempted alteration — there is no API to mutate `resolved_tenant` outside `crate::runtime`

---

## Non-Functional Requirements

### NFR-001: No Behavioral Regression

The change MUST NOT alter the observable behavior of tenant enforcement, interceptor execution
order, or security context propagation. The refactor is purely structural: the same logic
executes, but context reaches each call site via an explicit parameter rather than ambient
lookup.

### NFR-002: No New Synchronization Primitives

The change MUST NOT introduce `Mutex`, `RwLock`, `Arc<Mutex<...>>`, or any other
synchronization primitive to compensate for the removal of task-local state.

### NFR-003: Dependency Visibility

After this change, every component that requires a `ServiceContext` MUST declare that
dependency in its public API signature (parameter, constructor argument, or owned field).
No component MAY acquire a `ServiceContext` through a hidden lookup.

---

## Invariants

**INV-001 — Single Context Model**: There is exactly one mechanism for a component to access
a `ServiceContext`: it was given one explicitly. There is no fallback ambient mechanism.

**INV-002 — Interceptor Order Preserved**: The interceptor chain execution order (`on_request`
→ handler → `on_response` / `on_error`) MUST be identical before and after this change.

**INV-003 — Tenant Enforcement Preserved**: for a `#[tenant_scoped]` operation, `enforce_tenant`
MUST be called with the same `ServiceContext` that was passed to the proxy method, and it is a
**fallible** check (CORE-008A AD-009, FR-009): on failure the operation body MUST NOT be
entered — the caller observes the enforcement error as the outcome of the call. No tenant check
may be skipped or reordered. (An operation with no `#[tenant_scoped]` marker keeps the
pre-existing best-effort, non-blocking call — D1's valid tenant-less execution mode — and is
unaffected by this invariant.)

**INV-004 — Spawned Task Ownership**: Any asynchronous task created through `tokio::spawn`
or equivalent MUST receive `ServiceContext` through ownership transfer, explicit parameter
passing, or cloning at the call site before the spawn boundary. No spawned task MAY perform
an ambient lookup to obtain a `ServiceContext` after crossing the spawn boundary.

---

## Declarative Authorization with `#[authorize]` Macro (CORE-015)

### Requirement: `#[authorize]` Syntax Contract

The macro `#[authorize]` accepts exactly two named arguments: `context = <ident>` and `permission = "<resource>:<action>"`.

**Acceptance criteria:**

- AC-1.1: `#[authorize(context = ctx, permission = "orders:read")]` on a service method inside `#[service]` compiles and generates an authorization guard.
- AC-1.2: The named argument `context` receives an identifier, not an expression or path.
- AC-1.3: The named argument `permission` receives a string literal, not a const reference, macro call, or any other expression form.

---

### Requirement: Named-Argument Form Is Required

**Acceptance criteria:**

- AC-2.1: `#[authorize(ctx, "orders:read")]` (positional) fails compilation with error E4 (`unknown argument`).
- AC-2.2: `#[authorize(context = ctx, perm = "orders:read")]` (unknown key name) fails compilation with error E4.
- AC-2.3: `#[authorize(context = ctx)]` (missing `permission`) fails compilation with error E4b.
- AC-2.4: `#[authorize(permission = "orders:read")]` (missing `context`) fails compilation with error E4b.

---

### Requirement: Compile-Time Structural Validation of Permission Literal

The permission literal must satisfy: exactly one `:`, non-empty string before `:` (resource), non-empty string after `:` (action). No semantic constraints are applied beyond this structure.

**Acceptance criteria:**

- AC-3.1: A permission literal with no `:` (e.g., `"ordersread"`) fails compilation with error E1.
- AC-3.2: A permission literal with more than one `:` (e.g., `"a:b:c"`) fails compilation with error E1b.
- AC-3.3: A permission literal with an empty resource (e.g., `":read"`) fails compilation with error E2.
- AC-3.4: A permission literal with an empty action (e.g., `"orders:"`) fails compilation with error E3.
- AC-3.5: A non-literal value for `permission` (e.g., a const reference `PERM_CONST`) fails compilation with the non-literal error.
- AC-3.6: A valid literal like `"orders:read"` does not trigger E2 (non-empty resource is correctly identified).

---

### Requirement: Guard Execution Order and Behavior

Authorization guard executes BEFORE the method body; exactly one `authorize_in_context` call per annotated method.

**Acceptance criteria:**

- AC-4.1: When the authorization provider denies the request, the service method body does not execute (no observable side effect from the body).
- AC-4.2: The generated proxy contains exactly one call to `authorize_in_context` per `#[authorize]`-annotated method.
- AC-4.3: The authorization guard appears as the first executable step in the generated proxy body, before `enforce_tenant`, interceptor `on_request`, and the inner method call.

---

### Requirement: Fail-Closed Policy When Security Is Enabled

Authorization is fail-closed when security is enabled — absent or unavailable providers must return an error.

| Security state | Guard behavior | Error returned |
|---|---|---|
| `ctx.security()` is `None` (security capability disabled) | Guard not emitted; call proceeds | — |
| Security enabled; `runtime.upgrade()` returns `None` (runtime dropped) | Fail closed | `SecurityError::ProviderError("authorization provider unavailable: runtime dropped")` |
| Security enabled; authorization resolution yields `CapabilityNotEnabled` | Fail closed | `SecurityError::CapabilityNotEnabled` |
| Security enabled; provider present; provider denies | Fail closed | `SecurityError::AuthorizationDenied { .. }` (propagated from provider) |
| Security enabled; provider present; provider allows | Guard passes; body executes | — |

**Acceptance criteria:**

- AC-5.1: When `ctx.security()` is `None`, the method body executes without any authorization check.
- AC-5.2: When the runtime `Weak` reference has been dropped and `ctx.security()` is `Some`, the method returns `Err(E::from(SecurityError::ProviderError(...)))`.
- AC-5.3: When authorization resolution yields `SecurityError::CapabilityNotEnabled`, the generated guard propagates that error and the method body does not execute.
- AC-5.4: When the provider returns `Deny`, the method returns `Err(E::from(SecurityError::AuthorizationDenied { .. }))` and the body does not execute.
- AC-5.5: When the provider returns `Allow`, the method body executes and returns its result.

---

### Requirement: Compile-Time `From<SecurityError>` Bound on Error Type

**Acceptance criteria:**

- AC-6.1: A method whose `Result<_, E>` has an error type `E` that does not implement `From<SecurityError>` fails compilation with error E_from.
- AC-6.2: The compile error is rustc's standard trait bound diagnostic, triggered by the `__assert_from_security_error::<E>()` helper; the span targets the error type with a message identifying the missing `impl From<SecurityError> for E`. No custom `compile_error!` is emitted.

---

### Requirement: `#[authorize]` Outside `#[service]` Emits Compile Error

**Acceptance criteria:**

- AC-7.1: `#[authorize]` applied to a free function (outside any `#[service]` impl block) fails compilation with error E5.
- AC-7.2: `#[authorize]` applied to a function inside a plain `impl` block (not `#[service]`) fails compilation with error E5.
- AC-7.3: When `#[authorize]` is used correctly inside `#[service]`, error E5 is never emitted.

---

### Requirement: Marker Execution Order Is Fixed and Lexical-Order-Independent

The pipeline order is fixed and independent of attribute lexical order:

```
1. authorize
2. [future pre-body marker]
3. enforce_tenant
4. chain.on_request
5. inner.method(args)
6. chain.on_response / on_error
7. [future post-body marker]
8. return result
```

**Acceptance criteria:**

- AC-8.1: A method annotated `#[audit] #[authorize(...)]` generates the same proxy body as `#[authorize(...)] #[audit]` — the order of authorization relative to other markers is determined by the pipeline, not by lexical attribute position.
- AC-8.2: The generated proxy always places the authorization guard at slot 1 (before `enforce_tenant`, before interceptors).

---

### Requirement: `ServiceContext` Remains a Pure DTO

**Acceptance criteria:**

- AC-9.1: No new methods, fields, or trait implementations are added to `ServiceContext` in this change.
- AC-9.2: `ServiceContext` does not expose a reference or accessor to any runtime provider.

---

### Requirement: `RuntimeInner::authorization_provider()` Accessor Added

**Acceptance criteria:**

- AC-10.1: `RuntimeInner` exposes `pub fn authorization_provider(&self) -> Option<Arc<dyn AuthorizationProvider>>`.
- AC-10.2: The method returns `None` when no security providers are configured.
- AC-10.3: The method returns `Some(Arc<dyn AuthorizationProvider>)` (an owned clone) when an authorization provider is configured.
- AC-10.4: The authentication provider remains inaccessible; only the authorization `Arc` is exposed.

**Accessibility contract**: This accessor is `pub` solely to satisfy Rust's visibility rules for code generated by proc-macros. It is not part of the application programming model; application code must not call it directly. Any future public accessor on `RuntimeInner` requires an explicit ADR.

---

### Non-Functional: No New Public API Beyond `RuntimeInner::authorization_provider()` and `#[authorize]`

- No new types, traits, or functions are added to any public crate surface beyond those two items.

---

### Non-Functional: Generated Internals Are Not Public API

The following generated identifiers are implementation details, not part of any stability contract:

| Identifier | Role |
|---|---|
| `__rt` | Temporary `Arc<RuntimeInner>` in the proxy body |
| `__provider` | Temporary `Arc<dyn AuthorizationProvider>` in the proxy body |
| `__assert_from_security_error` | Zero-size helper function enforcing the `From<SecurityError>` bound |

These names MUST NOT appear in hand-written application code. `cargo expand` output is a debugging aid, not a compatibility contract.

---

### Non-Functional: Allocation Overhead Is Accepted

Generated code constructs `Resource { kind: "...".to_string(), .. }` and `Action("...".to_string())` — two `String` allocations per authorized call. These allocations are intentional, reusing the stable `security-sdk` `Resource`/`Action` owned API. Allocation-free variants are deferred to a future `security-sdk` API change.

---

### Diagnostics Contract for `#[authorize]` Errors

All errors are span-targeted at the offending token.

| Code | Trigger | Required message |
|---|---|---|
| E1 | Permission literal has no `:` | `#[authorize] permission "foo" must have the form "resource:action"` |
| E1b | Permission literal has more than one `:` | `#[authorize] permission "a:b:c" must have exactly one ':' (form "resource:action")` |
| E2 | Empty resource (e.g., `":read"`) | `#[authorize] resource in ":read" must not be empty` |
| E3 | Empty action (e.g., `"orders:"`) | `#[authorize] action in "orders:" must not be empty` |
| E4 | Unknown named argument | `#[authorize] unknown argument 'foo'; expected 'context' and 'permission'` |
| E4b | Missing required argument | `#[authorize] missing required argument; both 'context' and 'permission' are required` |
| E5 | `#[authorize]` used outside `#[service]` | `#[authorize] can only be used on methods inside a #[service] trait` |
| E6 | `context = <ident>` names a param not present in the method signature | `#[authorize] context parameter 'ctx' not found in method signature` |
| E_from | Method error type lacks `From<SecurityError>` | rustc trait bound error at error type (e.g., `the trait bound \`OrderError: From<SecurityError>\` is not satisfied`); emitted by `__assert_from_security_error::<E>()` helper — no custom message |
| AD-4 (non-literal) | `permission` value is not a string literal | `#[authorize] permission must be a string literal known at compile time` |
| AD-4 (non-ident) | `context` value is not an identifier | `#[authorize] context must be a parameter name (identifier), not an expression` |
