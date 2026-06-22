# Spec: CORE-010A — Remove Ambient ServiceContext

**Change**: remove-ambient-service-context
**Architecture Decision**: Option A — Explicit Context Everywhere
**Core Invariant**: Execution context MUST be visible in API boundaries.

---

## Scope

| Domain | Spec Type | Description |
|--------|-----------|-------------|
| `service-sdk` | New full spec | Explicit context propagation requirements for service-sdk |
| `security-sdk` | Delta — MODIFIED NFR-005 | Extends no-ambient-state prohibition to ServiceContext |

Domain spec files:
- `specs/service-sdk/spec.md`
- `specs/security-sdk/spec.md`

---

## Functional Requirements

| ID | Requirement | Strength |
|----|-------------|----------|
| FR-001 | No runtime component may obtain `ServiceContext` through ambient state | MUST NOT |
| FR-002 | `ServiceContext` accessible only through explicit parameters, owned values, or cloned values | MUST |
| FR-003 | No thread-local, task-local, global singleton, or hidden context mechanism for `ServiceContext` | MUST NOT |
| FR-004 | Existing propagation guarantees (tenant enforcement, interceptor order) are unchanged | MUST |
| FR-005 | Spawned tasks receive `ServiceContext` through explicit capture or ownership transfer | MUST |
| FR-006 | Runtime compiles without any ambient context implementation | MUST |
| FR-007 | Generated proxy code MUST NOT obtain `ServiceContext` through global/thread-local/task-local/singleton/static/registry/hidden lookup | MUST NOT |
| FR-008 | All runtime context propagation MUST use explicit ownership transfer, explicit parameters, or explicit cloning | MUST |
| FR-009 | Spawned tasks MUST receive `ServiceContext` through captured ownership or explicit parameter passing | MUST |

---

## Non-Functional Requirements

### NFR-001: No Behavioral Regression

The removal MUST NOT introduce behavioral changes to existing runtime flows. Tenant
enforcement, interceptor execution order, and security context propagation MUST behave
identically before and after the change.

### NFR-002: No New Synchronization Primitives

The removal MUST NOT introduce `Mutex`, `RwLock`, `Arc<Mutex<...>>`, or any other
synchronization primitive to compensate for the removed task-local state.

### NFR-003: Dependency Visibility

The resulting design MUST improve dependency visibility: every component that requires a
`ServiceContext` MUST declare that dependency explicitly in its public API surface.

### NFR-004: Forbidden Patterns (Absolute Prohibitions)

The following patterns MUST NOT appear anywhere in the workspace after this change:

| Pattern | Category |
|---------|----------|
| `ServiceContext::current()` | Ambient read |
| `ServiceContext::scope(...)` | Ambient scoping |
| `task_local! { static CURRENT_CONTEXT: ServiceContext }` | Task-local storage |
| `thread_local! { ... ServiceContext ... }` | Thread-local storage |
| `OnceCell<ServiceContext>` | Global singleton |
| `LazyLock<ServiceContext>` | Global singleton |
| Global Context Registry for `ServiceContext` | Registry pattern |
| Proxy-owned hidden `ServiceContext` field | Hidden coupling |
| Runtime-owned hidden `ServiceContext` field | Hidden coupling |
| Interceptor-owned hidden `ServiceContext` field | Hidden coupling |
| Context Provider abstraction with ambient lookup | Disguised ambient access |

---

## Migration Requirements

| ID | Requirement |
|----|-------------|
| MR-001 | Remove `tokio::task_local! { static CURRENT_CONTEXT: ServiceContext; }` from `crates/service-sdk/src/context/mod.rs` |
| MR-002 | Delete `ServiceContext::current()` from `crates/service-sdk/src/context/mod.rs` |
| MR-003 | Delete `ServiceContext::scope(...)` from `crates/service-sdk/src/context/mod.rs` |
| MR-004 | Rewrite `tests/context_scope.rs`, `tests/context_propagation.rs`, `tests/context_cross_service.rs` to use explicit construction |
| MR-005 | Remove all workspace references to the deleted ambient context APIs |

---

## Acceptance Criteria

| ID | Criterion | Verification |
|----|-----------|--------------|
| AC-001 | Zero workspace usages of `ServiceContext::current()` | `grep -rn "ServiceContext::current" crates/` returns 0 matches |
| AC-002 | Zero workspace usages of `ServiceContext::scope(...)` | `grep -rn "ServiceContext::scope" crates/` returns 0 matches |
| AC-003 | No task-local `ServiceContext` implementation | `grep -rn "CURRENT_CONTEXT" crates/` returns 0 matches |
| AC-004 | All runtime tests pass | `cargo test --workspace` exits 0 |
| AC-005 | Spawned task execution paths function through explicit context | Code review: no ambient reads inside `tokio::spawn` blocks |
| AC-006 | Build and lint gates clean | `cargo fmt --check && cargo clippy --all-targets --all-features && cargo test --workspace` all exit 0 |
| AC-007 | Zero workspace references to any ambient API | `grep -rn "ServiceContext::current\|ServiceContext::scope\|CURRENT_CONTEXT" crates/` returns 0 |
| AC-008 | Generated proxy code compiles and functions without ambient context | `cargo build --workspace` exits 0; proxy integration test passes |
| AC-009 | Tenant enforcement behavior unchanged | Existing tenant enforcement integration tests pass unmodified |
| AC-010 | Interceptor execution order unchanged | Existing interceptor order tests pass unmodified |
| AC-011 | All runtime context dependencies discoverable from public API signatures | No function, method, or trait impl obtains `ServiceContext` without it appearing in its signature or constructor |

---

## Acceptance Scenarios

### Scenario: Proxy generated method signature is explicit (AC-008)

- GIVEN a service trait annotated with the proxy derive macro
- WHEN the proc macro expands the forwarding method
- THEN the method signature contains `ctx: ServiceContext` as the first user parameter
- AND `self.enforce_tenant(&ctx)?` appears before the forwarding call
- AND no call to `ServiceContext::current()` or `ServiceContext::scope()` appears in the body

### Scenario: Interceptors receive context from parameter (AC-010)

- GIVEN a proxy dispatch flow with interceptors registered
- WHEN a service method is called with an explicit `ServiceContext`
- THEN `on_request(&ctx)`, `on_response(&ctx)`, and `on_error(&ctx)` each receive the same `ctx`
  value that was passed to the proxy method
- AND interceptor execution order is `on_request` → handler → `on_response` (or `on_error`)

### Scenario: Tenant enforcement preserves behavior (AC-009)

- GIVEN a `ServiceContext` with a specific `tenant_id`
- WHEN a proxy-generated method is called with that context
- THEN `enforce_tenant` receives the same tenant value as before the change
- AND a context with a mismatched tenant returns `Err` with the same error variant as before

### Scenario: Spawned task receives context explicitly (FR-005 / AC-005)

- GIVEN a `ServiceContext` in the outer scope
- WHEN `tokio::spawn(async move { use_ctx(ctx) })` is called
- THEN the spawned task holds `ctx` through the move capture
- AND the task can call any method requiring `ServiceContext` without any ambient read

### Scenario: Test suite passes with explicit construction (AC-004)

- GIVEN all three rewritten test files: `context_scope.rs`, `context_propagation.rs`,
  `context_cross_service.rs`
- WHEN `cargo test --workspace` is run
- THEN all tests in those files pass
- AND no test calls `ServiceContext::current()` or `ServiceContext::scope()`

### Scenario: grep confirms zero ambient API references (AC-007)

- GIVEN the workspace after all changes are applied
- WHEN `grep -rn "ServiceContext::current\|ServiceContext::scope\|CURRENT_CONTEXT" crates/`
  is executed
- THEN the command returns no output and exits with code 1 (no matches)

### Scenario: No new sync primitives introduced (NFR-002)

- GIVEN the diff between the feature branch and `develop`
- WHEN the diff is reviewed for new synchronization primitives
- THEN no new `Mutex`, `RwLock`, `Arc<Mutex<...>>`, `Semaphore`, or `Condvar` is introduced
  in `crates/service-sdk/`

---

## Implementation Targets

| File | Change |
|------|--------|
| `crates/service-sdk/src/context/mod.rs` lines 9-11 | Remove `task_local!` declaration |
| `crates/service-sdk/src/context/mod.rs` lines 187-189 | Delete `current()` method |
| `crates/service-sdk/src/context/mod.rs` lines 199-206 | Delete `scope()` method |
| `crates/service-sdk-macros/src/lib.rs` lines 119-149 | Rewrite proxy codegen to thread `ctx` explicitly |
| `crates/service-sdk/tests/context_scope.rs` | Rewrite: remove `scope()` and `current()` usage |
| `crates/service-sdk/tests/context_propagation.rs` | Rewrite: remove `scope()` and `current()` usage |
| `crates/service-sdk/tests/context_cross_service.rs` | Rewrite: remove `scope()` usage |

---

## Invariants

**INV-001 — Single Context Model**: There is exactly one mechanism for a component to obtain
a `ServiceContext`: it was given one explicitly. There is no ambient fallback.

**INV-002 — Interceptor Order Preserved**: The interceptor chain execution order
(`on_request` → handler → `on_response` / `on_error`) is identical before and after this change.

**INV-003 — Tenant Enforcement Preserved**: `enforce_tenant` MUST be called with the same
`ServiceContext` that was passed to the proxy method. No tenant check may be skipped or
reordered as a result of this change.
