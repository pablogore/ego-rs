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
