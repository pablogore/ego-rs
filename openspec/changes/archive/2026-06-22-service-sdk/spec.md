# Spec: service-sdk

## Overview

Complete SPEC-008 by replacing the hollow scaffold in `feature/SPEC-008-service-sdk` with a behaviorally complete Service SDK: a type-keyed registry that holds live implementations, a 100% macro-generated `{TraitName}Ref` proxy with interceptor chain and context propagation, a wired `RuntimeBuilder` with Kahn cycle detection, and the structural consolidation (descriptor deduplication, `LifecycleManaged` split, `ServiceContext` hardening, `ServiceErrorTrait`) required before all of that is coherent.

---

## Requirements

### REQ-001: Type-keyed live registry — registration

The registry stores live `Arc<dyn Trait>` implementations keyed by `(TypeId, ContractVersion)`, not by string and not by descriptor alone.

**Given** a `ServiceRegistry` and a concrete `Arc<dyn OrderService>` at version `1.0.0`
**When** `registry.register::<OrderService>(impl_arc, ContractVersion::new(1,0,0))` is called
**Then** the call returns `Ok(())` and the implementation is retrievable by type and version

**Test**: `registry::tests::register_stores_live_implementation` — register a test impl, assert `resolve` returns an `Arc` pointing to the same object.

---

### REQ-002: Type-keyed live registry — duplicate rejection

**Given** an implementation already registered for `(OrderService, 1.0.0)`
**When** a second `register::<OrderService>(other_impl, ContractVersion::new(1,0,0))` is called
**Then** the call returns `Err(RegistryError::DuplicateService { name, version })`; the original implementation remains registered

**Test**: `registry::tests::register_rejects_duplicate` — register twice, assert `Err(DuplicateService)` on second call, assert first impl still resolves.

---

### REQ-003: Type-keyed live registry — exact version resolution

**Given** implementations registered at `1.0.0` and `2.0.0` for the same trait
**When** `registry.resolve::<OrderService>(VersionConstraint::Exact("1.0.0"))` is called
**Then** the `Arc` for version `1.0.0` is returned, not `2.0.0`

**Test**: `registry::tests::resolve_exact_version` — register two versions, resolve each independently.

---

### REQ-004: Type-keyed live registry — semver range resolution

**Given** implementations at `1.2.0` and `2.0.0`
**When** `registry.resolve::<OrderService>(VersionConstraint::Range("^1"))` is called
**Then** the `Arc` for `1.2.0` is returned (highest satisfying patch)

**Test**: `registry::tests::resolve_semver_range` — assert correct version returned for `^1`, `>=1.1`, `<2`.

---

### REQ-005: Type-keyed live registry — unsatisfied resolution

**Given** no implementation registered for a type, or a registered version that does not satisfy the constraint
**When** `resolve` is called
**Then** the call returns `Err(RegistryError::ServiceNotFound)`

**Test**: `registry::tests::resolve_returns_not_found` — resolve on empty registry; resolve with mismatched version constraint.

---

### REQ-006: `{TraitName}Ref` proxy — macro generation from `#[service]` on trait

`#[service]` applied to a trait named `OrderService` MUST emit, alongside the existing `ServiceContract` impl:
- a concrete struct `OrderServiceRef` containing `Arc<dyn OrderService>` and an `InterceptorChain`
- `impl OrderService for OrderServiceRef` with one typed forwarding method per `#[operation]`-annotated method

No developer writes proxy code by hand. `ServiceRef<T>` is deleted.

**Given** a trait annotated with `#[service]` and at least one `#[operation]` method
**When** the crate compiles
**Then** `{TraitName}Ref` exists as a public type, implements the original trait, and accepts `Arc<dyn TraitName>` plus `InterceptorChain` in its constructor

**Test**: `macros::tests::service_on_trait_generates_ref_struct` — verify generated type via `trybuild` expansion or `cargo expand` snapshot; confirm the struct compiles and the impl satisfies the trait bound.

---

### REQ-007: `{TraitName}Ref` proxy — typed forwarding with interceptor chain

Every method in `impl TraitName for {TraitName}Ref` MUST run the interceptor chain transparently before and after the underlying implementation call.

**Given** an `OrderServiceRef` wrapping a live `Arc<dyn OrderService>` and a non-empty `InterceptorChain`
**When** a typed operation method is called on the ref
**Then**
  1. `interceptor.on_request(ctx)` is called for each interceptor in order
  2. the underlying impl method is awaited
  3. `interceptor.on_response(ctx, result)` or `interceptor.on_error(ctx, err)` is called for each interceptor

**Test**: `tests/interceptor_invocation.rs` — existing test updated to use a generated ref; assert interceptor hooks fire in declared order.

---

### REQ-008: `{TraitName}Ref` proxy — automatic `ServiceContext` scope propagation

Each method invocation on `{TraitName}Ref` MUST establish a new `ServiceContext` scope for the duration of the call. The caller's context (from `ServiceContext::current()`) is propagated as the scope for the nested call.

**Given** a `ServiceContext` set as task-local via `scope()`
**When** an operation is called on a `{TraitName}Ref`
**Then** `ServiceContext::current()` inside the implementation returns the same context that was active in the caller; the scope is torn down after the call returns

**Test**: `tests/context_cross_service.rs` — updated to assert context identity across a cross-service call through a generated ref.

---

### REQ-009: `#[service]` on structs — field-type detection

`#[service]` applied to a struct MUST parse each field and identify dependency types by their category:
- `EntityRef<T>` → entity dependency
- `ProjectionRef<P>` → projection dependency
- `AdapterRef<A>` → adapter dependency
- Other annotated fields → config-value injection

**Given** a struct annotated with `#[service]`
**When** the crate compiles
**Then** the macro emits DI metadata listing each detected dependency by category and type

**Test**: `macros::tests::service_on_struct_detects_fields` — struct with one field of each category; verify DI metadata is emitted via macro expansion snapshot.

---

### REQ-010: `#[service]` on structs — factory generation

**Given** a struct annotated with `#[service]` whose fields are resolvable from the registry
**When** the generated factory is called with a resolved registry
**Then** an instance of the struct is constructed with all fields populated from the registry; the call returns `Err(RegistryError::DependencyNotFound)` if any required dependency is absent

**Test**: `tests/smoke.rs` — updated end-to-end test: register all deps, call factory, assert struct is constructed correctly.

---

### REQ-011: DI primitives

`ProjectionRef<P>` and `AdapterRef<A>` are defined in service-sdk as injection primitives. `EntityRef<T>` is imported from `entity_sdk` (never defined locally). All three are re-exported from `service_sdk::lib`.

**Given** a struct field typed as `EntityRef<Order>`, `ProjectionRef<OrderView>`, or `AdapterRef<PaymentGateway>`
**When** the macro processes the struct
**Then** the field is recognized as an injectable dependency without any manual annotation beyond its type

**Test**: `registry::tests::di_primitives_are_recognizable` — assert each wrapper type is detected from a parsed struct field.

---

### REQ-012: RuntimeBuilder — factory collection and bundle merging

`RuntimeBuilder` collects factories via `with_entity::<E>()`, `with_projection::<P>()`, `with_service::<S>()`, and `with_service_bundle(bundle)`. Bundle merging flattens all factories from a bundle into the builder's factory set.

**Given** a `RuntimeBuilder` with two services and a bundle containing two more
**When** `build()` is called
**Then** the runtime is constructed with all four services; no service is duplicated

**Test**: `runtime::tests::builder_collects_and_merges_bundles` — assert the resulting `Runtime` can resolve all four service types.

---

### REQ-013: RuntimeBuilder — dependency validation before construction

`RuntimeBuilder::build()` MUST verify that every declared dependency of every registered service is satisfiable by another registered factory before constructing any instance.

**Given** a service `A` that declares a dependency on type `B`, but `B` is not registered
**When** `build()` is called
**Then** the call returns `Err(RuntimeError::DependencyNotFound { service, dependency })` before constructing any instance

**Test**: `runtime::tests::build_fails_on_missing_dependency` — register `A` without registering `B`; assert `DependencyNotFound`.

---

### REQ-014: RuntimeBuilder — cycle detection via Kahn's algorithm

**Given** services `A → B → C → A` (circular dependency chain)
**When** `RuntimeBuilder::build()` is called
**Then** the call returns `Err(RuntimeError::DependencyCycle { cycle: Vec<String> })` naming all participants in the cycle, before constructing any instance

**Test**: `runtime::tests::build_detects_dependency_cycle` — register a three-node cycle; assert `DependencyCycle` with the correct participant list.

---

### REQ-015: RuntimeBuilder — ordered instance construction

**Given** a valid dependency graph `A → B → C` (no cycles, all deps present)
**When** `build()` succeeds
**Then** instances are constructed in reverse topological order: `C` first, then `B` (receiving `C`), then `A` (receiving `B`); the returned `Runtime` holds live instances

**Test**: `runtime::tests::build_constructs_in_dependency_order` — assert construction order via a side-effecting factory, verify the Runtime resolves `A` successfully.

---

### REQ-016: Runtime — proxy resolution after build

**Given** a successfully built `Runtime` with `OrderService` registered
**When** `runtime.resolve::<OrderService>()` is called
**Then** a ready-to-call `OrderServiceRef` (with wired interceptor chain and context propagation) is returned

**Test**: `runtime::tests::runtime_resolves_proxy_after_build` — build a runtime, resolve a proxy, call a typed method, assert correct result.

---

### REQ-017: LifecycleManaged split

The `Service` trait MUST NOT declare `initialize()` or `shutdown()`. A separate `LifecycleManaged` trait provides those hooks and is only implemented by runtime-managed components (entities, projections, adapters). The runtime calls `LifecycleManaged::initialize()` on startup and `shutdown()` on teardown in reverse construction order.

**Given** a struct that implements only `Service` (not `LifecycleManaged`)
**When** the runtime starts and stops
**Then** no lifecycle hooks are invoked on that struct

**Given** a struct that implements `LifecycleManaged`
**When** the runtime starts
**Then** `initialize()` is called; on teardown, `shutdown()` is called in reverse initialization order

**Test**: `tests/smoke.rs` — assert `Service` impl compiles without `initialize`/`shutdown`; `LifecycleManaged` hooks fire for an entity adapter.

---

### REQ-018: Cross-tenant enforcement — Runtime as sole authority

The `Runtime` MUST reject any service invocation where the calling context's `tenant_id` differs from the service's registered tenant scope and `allow_cross_tenant` is `false` in the active `ServiceContext`. Generated `{TraitName}Ref` proxies MAY perform defensive pre-call checks (tenant present, context valid) but are never the sole enforcement barrier. No invocation path — including direct registry resolution, internal calls, or test harnesses — bypasses runtime tenant validation.

**Given** a `Runtime` with tenant isolation active and a `ServiceContext` with `tenant_id = "A"` calling a service registered under `tenant_id = "B"` with `allow_cross_tenant = false`
**When** the operation is invoked
**Then** the call returns `Err(RuntimeError::CrossTenantViolation { caller_tenant, service_tenant })` before reaching the implementation

**Given** the same setup but `allow_cross_tenant = true`
**When** the operation is invoked
**Then** the call proceeds normally

**Test**: `tests/tenant_isolation.rs` — updated to exercise runtime enforcement, not just the flag.

---

### REQ-019: `ServiceContext` — no `serde`, plus `CancellationToken`

`ServiceContext` MUST NOT derive or implement `Serialize`/`Deserialize`. Serialization is a transport-adapter concern. `ServiceContext` MUST include `cancellation_token: Option<tokio_util::sync::CancellationToken>` for push-style cancellation alongside the existing deadline polling.

**Given** a `ServiceContext` with a `CancellationToken` that has been cancelled
**When** `ctx.is_cancelled()` is called inside a running operation
**Then** `true` is returned, allowing the operation to abort early

**Given** `ServiceContext` source code
**When** inspected for `serde` attributes or imports
**Then** none are present

**Test**: `tests/cancellation.rs` — existing test extended to use push-style `CancellationToken::cancel()`; assert the operation checks cancellation and aborts. Compile-time test: `ServiceContext` does not implement `serde::Serialize`.

---

### REQ-020: `ServiceErrorTrait` for interceptors

Interceptors MUST program against a `ServiceErrorTrait` (object-safe) rather than the concrete `ServiceError` enum. Domain errors flow through interceptors unchanged. The `ServiceError` enum MAY implement `ServiceErrorTrait`. The duplicated `DomainError` trait (currently in both `error/domain_error.rs` and `error/category.rs`) MUST be consolidated to a single definition.

**Given** an interceptor that matches on error category via `ServiceErrorTrait`
**When** a domain error is returned by the implementation
**Then** the interceptor receives `&dyn ServiceErrorTrait`, can inspect `code()` and `category()`, and the original error value is forwarded unchanged to the caller

**Test**: `tests/interceptor_error.rs` — updated interceptor test: assert `on_error` receives `&dyn ServiceErrorTrait`; confirm the interceptor can call `.code()` and `.category()` without knowing the concrete type.

---

### REQ-021: Descriptor consolidation

`contract/descriptor.rs` is the single canonical source for all descriptor types (`ServiceDescriptor`, `OperationDescriptor`, `ContractDescriptor`, `FieldDescriptor`). The duplicated definitions in `contract/contract.rs`, `service/service.rs`, `operation/operation.rs`, and `version/version.rs` are deleted. `ContractDescriptor` and `FieldDescriptor` are re-exported from `lib.rs`. `OperationDescriptor` gains `idempotent: bool` and `mutating: bool` flags. `FieldDescriptor` gains `required: bool`.

**Given** the compiled workspace
**When** any module attempts to import a descriptor type
**Then** exactly one definition exists; the import path resolves to `service_sdk::contract::{type}` or its `lib.rs` re-export

**Test**: Compile-time — workspace compiles without duplicate-type errors after consolidation. `descriptor_fields::tests::operation_descriptor_has_idempotency_flag` — assert the field is present and defaults correctly.

---

### REQ-022: End-to-end integration path

A single integration test exercises the full observable path: register → resolve with version constraint → invoke via generated `{TraitName}Ref` → interceptor fires → `ServiceContext` propagates into the nested call → domain error returns through the trait boundary.

**Given** a service `OrderService` with one operation `place_order` that returns a domain error on invalid input
**When** the test runs the full path
**Then**
  1. `registry.register::<OrderService>(impl, v1)` returns `Ok(())`
  2. `registry.resolve::<OrderService>(VersionConstraint::Range("^1"))` returns an `Arc`
  3. an `OrderServiceRef` is constructed with the resolved `Arc` and a logging interceptor
  4. `order_ref.place_order(invalid_input).await` triggers `on_request`, calls the impl, triggers `on_error`
  5. the caller receives the typed domain error
  6. the logging interceptor observed both the request context and the error

**Test**: `tests/smoke.rs` — one end-to-end test covering all six assertions above.

---

## Invariants

**INV-001**: Registry type safety — implementations are always stored and retrieved as `Arc<dyn Trait>` via `TypeId`; no string-keyed resolution path exists.

**INV-002**: Proxy completeness — every `#[operation]`-annotated method on a trait annotated with `#[service]` MUST appear in `impl TraitName for {TraitName}Ref`. It is a compile error for any operation to be missing from the proxy.

**INV-003**: Transport freedom — no type in `service_sdk` (excluding transport-adapter integration points that are explicitly out of scope) imports from any HTTP, gRPC, or messaging library. `ServiceContext` does not implement `Serialize`/`Deserialize`.

**INV-004**: Context isolation — `ServiceContext::current()` returns `None` outside a `scope()` call. It never panics. Task-local state does not leak across tokio task boundaries.

**INV-005**: Single descriptor authority — exactly one definition of each descriptor type exists in the compiled workspace at any time. Duplicate definitions are a compile error.

**INV-006**: Lifecycle separation — no type in the compiled workspace simultaneously implements both `Service` and provides `initialize()`/`shutdown()` through the `Service` trait. Those methods exist only on `LifecycleManaged`.

**INV-007**: Enforcement layering — the `Runtime` is the sole cross-tenant enforcement authority. A proxy MUST NOT be the only barrier; if the proxy check is bypassed, the runtime still rejects the call.

**INV-008**: `EntityRef<T>` origin — no definition of `EntityRef<T>` appears in `service-sdk` source. The type is always imported from `entity_sdk`.

---

## Error Conditions

| Condition | Trigger | Expected Result |
|---|---|---|
| Duplicate registration | `register` called twice for same `(TypeId, ContractVersion)` | `Err(RegistryError::DuplicateService { name, version })` |
| Service not found | `resolve` on unregistered type or unsatisfied semver constraint | `Err(RegistryError::ServiceNotFound)` |
| Missing dependency | `build()` with a declared dep not registered | `Err(RuntimeError::DependencyNotFound { service, dependency })` |
| Circular dependency | `build()` with a cycle in the dependency graph | `Err(RuntimeError::DependencyCycle { cycle: Vec<String> })` |
| Cross-tenant violation | Invocation where caller tenant ≠ service tenant and `allow_cross_tenant = false` | `Err(RuntimeError::CrossTenantViolation { caller_tenant, service_tenant })` |
| Cancelled context | Operation checks `ctx.is_cancelled()` after `CancellationToken::cancel()` | Returns `true`; operation may abort and return an appropriate domain error |
| Expired deadline | `ctx.is_deadline_expired()` polled and deadline has passed | Returns `true`; operation MAY abort — same mechanism as before, now coexists with cancellation token |
| Domain error passthrough | Implementation returns a domain error; interceptor observes it | Error type is unchanged; interceptor receives `&dyn ServiceErrorTrait`; caller receives original `Err(domain_error)` |

---

## Test Scenarios

### TS-001: Registry — full CRUD lifecycle
1. Create `ServiceRegistry`.
2. Register `OrderService` v1.0.0.
3. Register `OrderService` v2.0.0.
4. Resolve with `Exact("1.0.0")` — assert v1 impl.
5. Resolve with `Range("^2")` — assert v2 impl.
6. Attempt duplicate register at v1.0.0 — assert `DuplicateService`.
7. Resolve with `Range("^3")` — assert `ServiceNotFound`.

### TS-002: Proxy — generated ref forwards with interceptor chain
1. Annotate `PaymentService` with `#[service]` and one `#[operation]` method.
2. Compile; assert `PaymentServiceRef` exists as a public type.
3. Register a concrete impl via the registry.
4. Build a ref with a `SpyInterceptor` (records calls).
5. Call the typed operation.
6. Assert: `on_request` fired, impl invoked, `on_response` fired — all in order.

### TS-003: Proxy — context propagation across service boundary
1. Build a context with `tenant_id = "tenant-42"` and a `trace_id`.
2. Inside `ServiceContext::scope(ctx, ...)`, resolve and call an `OrderServiceRef`.
3. Inside the impl body, assert `ServiceContext::current()` returns the same `tenant_id` and `trace_id`.
4. After the scope exits, assert `ServiceContext::current()` returns `None`.

### TS-004: RuntimeBuilder — happy-path wiring
1. Define three services: `C` (no deps), `B` (depends on `C`), `A` (depends on `B`).
2. Register all three with the builder via `with_service`.
3. Call `build()` — assert `Ok(Runtime)`.
4. Resolve `A` from the runtime — assert the live `A` instance received a live `B` which received a live `C`.

### TS-005: RuntimeBuilder — cycle detection
1. Register `A → B`, `B → C`, `C → A`.
2. Call `build()`.
3. Assert `Err(RuntimeError::DependencyCycle)` with participant names `["A", "B", "C"]` (or any rotation).

### TS-006: RuntimeBuilder — missing dependency
1. Register `A` which declares dependency `B`.
2. Do not register `B`.
3. Call `build()`.
4. Assert `Err(RuntimeError::DependencyNotFound { service: "A", dependency: "B" })`.

### TS-007: Cross-tenant enforcement
1. Build a `Runtime` with isolation active for `tenant-1`.
2. Invoke a service with a context for `tenant-2` and `allow_cross_tenant = false`.
3. Assert `Err(RuntimeError::CrossTenantViolation)`.
4. Repeat with `allow_cross_tenant = true`; assert the call succeeds.

### TS-008: `CancellationToken` integration
1. Create a `CancellationToken` and place it in a `ServiceContext`.
2. Launch an async operation that polls `is_cancelled()` in a loop.
3. Cancel the token from a separate task.
4. Assert the operation observes the cancellation and terminates early.

### TS-009: `ServiceErrorTrait` interceptor decoupling
1. Define a `LoggingInterceptor` that only knows `&dyn ServiceErrorTrait`.
2. Register an `OrderService` whose `place_order` returns a custom `OrderError : ServiceErrorTrait`.
3. Call `place_order` with invalid input.
4. Assert the interceptor's `on_error` is called with a `&dyn ServiceErrorTrait`.
5. Assert the caller receives the original `OrderError` value unchanged.

### TS-010: End-to-end smoke test (REQ-022)
Steps as described in REQ-022 above — the single test that ties all subsystems together. Must pass under `cargo test --workspace` with coverage contribution counted toward the 95% threshold.

---

## Strict TDD Notes

All requirements above MUST have corresponding tests that are written BEFORE the implementation they validate. The test command is `cargo test --workspace`. Coverage gate is `cargo tarpaulin --workspace --out Html --fail-under 95`.

Macro-generated code is counted toward coverage when exercised through behavioral tests (TS-002 through TS-010). Dedicated macro expansion tests (`trybuild` or `cargo expand` snapshots) are required for REQ-006 and REQ-009 to cover the generated shape, and those test files count toward workspace coverage.
