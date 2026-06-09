# Quickstart: Service SDK

**Feature**: 008-service-sdk  
**Date**: 2026-06-08

## Overview

This guide demonstrates how to define a service, register it, resolve it, and invoke it — all transport-agnostic. See [data-model.md](./data-model.md) for entity details and [contracts/README.md](./contracts/README.md) for API signatures.

## Prerequisites

- Rust 1.75+
- Cargo workspace with `ego-service-sdk` and `ego-service-sdk-macros` crates added as dependencies
- `tokio` runtime (workspace dependency)
- Working `ego-domain` crate for base types

## Scenarios

### Scenario 1: Declare a Service Contract

**Goal**: Define a service boundary without transport knowledge.

**Steps**:
1. Create a trait annotated with `#[service]`
2. Mark methods with `#[operation]`
3. Verify descriptor metadata is generated at compile time

**Expected**: `ServiceDescriptor` is generated with operations `create_order` and `get_order`. No transport annotations present. The `#[service]` attribute on the trait generates `ServiceDescriptor`, `OperationDescriptor`, and `ContractDescriptor`. The `#[operation]` attribute on each method generates per-operation metadata (name, input/output types, category).

**Validation command**:
```bash
cargo check -p ego-service-sdk
```

---

### Scenario 2: Implement and Register a Service

**Goal**: Implement a service with field-declared dependencies and register it.

**Steps**:
1. Define a struct implementing the service trait
2. Declare dependencies as fields (`EntityRef<Order>`, read store, etc.)
3. Build a registry and register the service
4. Assert the service is resolvable

**Expected**: `registry.resolve::<OrderService>()` returns `Ok(ServiceRef<OrderService>)` with descriptor populated.

**Validation command**:
```bash
cargo test -p ego-service-sdk -- registry_tests
```

---

### Scenario 3: Resolve With Version Constraints

**Goal**: Register two versions of the same service and resolve each.

**Steps**:
1. Register OrderService v1.0.0
2. Register OrderService v2.0.0
3. Resolve with no version → returns v2.0.0 (latest)
4. Resolve with exact version v1.0.0 → returns v1.0.0
5. Attempt duplicate registration (same type + name + version) → rejected

**Expected**: Resolution returns the correct version. Duplicate registration errors.

**Validation command**:
```bash
cargo test -p ego-service-sdk -- version_resolution
```

---

### Scenario 4: Dependency Validation

**Goal**: Verify the registry detects missing dependencies.

**Steps**:
1. Define a service that depends on another service
2. Register only the dependent service (not the dependency)
3. Run validation → fails with missing dependency error
4. Register the missing dependency
5. Re-run validation → passes

**Expected**: Validation fails with clear error listing the missing dependency by TypeId. After registering the dependency, validation passes.

**Validation command**:
```bash
cargo test -p ego-service-sdk -- dependency_validation
```

---

### Scenario 5: Invocation Interceptors

**Goal**: Verify interceptors fire on service invocation.

**Steps**:
1. Create a mock interceptor that counts invocations
2. Register it with the service registry
3. Invoke a service operation through ServiceRef
4. Assert `on_request` and `on_response` were called
5. Invoke an operation that errors
6. Assert `on_error` was called

**Expected**: Interceptors observe every invocation. Service code is unaware of interceptor presence.

**Validation command**:
```bash
cargo test -p ego-service-sdk -- interceptor_tests
```

---

### Scenario 6: ServiceContext Propagation

**Goal**: Verify context travels with the invocation.

**Steps**:
1. Set a ServiceContext with tenant ID "tenant-A"
2. Invoke a service operation within that context scope
3. From within the service, call `ServiceContext::current()` → tenant is "tenant-A"
4. From within the service, call another service → callee sees same context
5. Outside the scope, `ServiceContext::current()` is unavailable

**Expected**: Context propagates transparently across service-to-service calls. Context is isolated to invocation scope.

**Validation command**:
```bash
cargo test -p ego-service-sdk -- context_propagation
```

---

### Scenario 7: Tenant Isolation Enforcement

**Goal**: Verify cross-tenant access is rejected.

**Steps**:
1. Register an entity scoped to "tenant-A"
2. Set ServiceContext with tenant "tenant-B"
3. Attempt to access the entity → rejected with tenant violation error
4. Set ServiceContext with tenant "tenant-A"
5. Access the entity → succeeds

**Expected**: Entity access is automatically scoped to the current tenant. Cross-tenant access requires explicit opt-in.

**Validation command**:
```bash
cargo test -p ego-service-sdk -- tenant_isolation
```

---

### Scenario 8: Deadline Propagation

**Goal**: Verify deadline expiration terminates invocation.

**Steps**:
1. Set a ServiceContext with a deadline 100ms in the future
2. Invoke a service operation that takes 200ms
3. The runtime detects deadline expiry and terminates the invocation
4. A timeout error is returned (not a domain error)

**Expected**: Deadline is propagated. Expired deadline terminates invocation before completion.

**Validation command**:
```bash
cargo test -p ego-service-sdk -- deadline_expiry
```

---

### Scenario 9: Multi-Module Composition

**Goal**: Verify bundles from separate modules merge correctly.

**Steps**:
1. Create module A's ServiceBundle with OrderService v1
2. Create module B's ServiceBundle with CustomerService v1
3. Merge both bundles into a single ServiceRegistry
4. Resolve both services → both available
5. Attempt to merge a bundle with conflicting version → rejected

**Expected**: Bundles merge without conflicts. Cross-module resolution works.

**Validation command**:
```bash
cargo test -p ego-service-sdk -- bundle_merge
```

---

### Scenario 10: Transport Agnosticism Verification

**Goal**: Verify the SDK has no transport dependencies.

**Steps**:
1. Inspect `ego-service-sdk` Cargo.toml dependencies
2. Confirm no HTTP, gRPC, GraphQL, or WebSocket library is present
3. Inspect `ego-service-sdk-macros` Cargo.toml — only `syn`, `quote`, `proc-macro2`

**Expected**: Zero transport dependencies. Only domain types and proc-macro utilities.

**Validation command**:
```bash
cargo tree -p ego-service-sdk --depth 2 | grep -i -E "http|grpc|graphql|websocket|axum|tonic|actix"
# Expected: no output
```

---

## End-to-End Smoke Test

Run all validation scenarios:

```bash
cargo test -p ego-service-sdk
cargo test -p ego-service-sdk-macros
cargo test --workspace  # verify no regressions in dependent crates
```

Expected: All tests pass, coverage >= 85%.
