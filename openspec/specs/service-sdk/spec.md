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

Tenant enforcement MUST be called as `self.enforce_tenant(&ctx)?`. Interceptor hooks MUST
receive the context explicitly: `interceptor.on_request(&ctx)`, `interceptor.on_response(&ctx)`,
`interceptor.on_error(&ctx)`. No ambient read (`current()`) or scope wrap (`scope()`) is
permitted inside the generated body.

#### Scenario: Generated proxy compiles with explicit ctx parameter

- GIVEN a service trait annotated with the proxy derive macro
- WHEN the macro expands the forwarding method
- THEN the generated method accepts `ctx: ServiceContext` as the first user-visible parameter
- AND the body calls `self.enforce_tenant(&ctx)` before forwarding the request

#### Scenario: Interceptors receive context from parameter, not ambient state

- GIVEN a proxy dispatch flow with one or more interceptors registered
- WHEN a service method is called with an explicit `ServiceContext`
- THEN `on_request`, `on_response`, and `on_error` hooks each receive `&ctx` sourced from the
  parameter — no call to `ServiceContext::current()` occurs inside the generated body

#### Scenario: Tenant enforcement behavior preserved

- GIVEN a `ServiceContext` with `tenant_id = "tenant-a"`
- WHEN a proxy-generated method is called with that context
- THEN `enforce_tenant` uses the tenant from the explicit `ctx` parameter
- AND a context with a mismatched tenant returns the same enforcement error as before this change

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

**INV-003 — Tenant Enforcement Preserved**: `enforce_tenant` MUST be called with the same
`ServiceContext` that was passed to the proxy method. No tenant check may be skipped or
reordered.

**INV-004 — Spawned Task Ownership**: Any asynchronous task created through `tokio::spawn`
or equivalent MUST receive `ServiceContext` through ownership transfer, explicit parameter
passing, or cloning at the call site before the spawn boundary. No spawned task MAY perform
an ambient lookup to obtain a `ServiceContext` after crossing the spawn boundary.
