# Explore: service-sdk (SPEC-008)

## Summary

The `feature/SPEC-008-service-sdk` branch contains a structural scaffold covering approximately 35–40% of SPEC-008 requirements as compilable code. The core abstractions are present (ServiceContext, descriptors, interceptor chain, ContractVersion, RuntimeBuilder skeleton, `#[service]` macro for traits), but almost all behavioral requirements are hollow stubs. The most significant gaps are: no type-safe `{TraitName}Ref` proxy generation, no live-implementation registry, no actual dependency injection or circular-dependency detection, no RuntimeBuilder wiring, no EntityRef/ProjectionRef/AdapterRef types, and `#[service]` does not handle structs.

---

## Implementation Coverage

### ✅ Fully Implemented

- **FR-002**: Domain types as inputs/outputs — no serialization annotations on contract traits
- **FR-003**: Zero transport library imports confirmed across all source files
- **FR-021d–g**: `Interceptor` trait with `on_request`/`on_response`/`on_error` hooks; `InterceptorChain` runs them in order; pluggable via `Arc<dyn Interceptor>`
- **FR-021l / FR-021n / FR-021n1**: `ServiceContext::current()` returns `Option<ServiceContext>` via `tokio::task_local!`, never panics; `scope()` correctly sets and tears down per-task context
- **FR-021m**: ServiceContext fields: `tenant_id`, `correlation_id`, `trace_id`, `deadline`, `timeout`, `additional_context` — all present with builder methods
- **FR-021u/v**: `deadline: Option<SystemTime>` + `is_deadline_expired()` polling; `timeout: Option<Duration>` present; both tested
- **FR-022e**: `ContractVersion` with full semver (major/minor/patch), `Display`, `FromStr`, `Ord` — implemented and tested
- **FR-022f**: Descriptors are entirely transport-free
- **FR-025a**: Domain error pattern demonstrated in example (`OrderError` with `thiserror`)
- **FR-025b/c**: `DomainError` trait (`code()` + `category()`) and `ErrorCategory` enum exist
- **FR-025e**: No mandatory shared error enum enforced

### ⚠️ Partially Implemented

- **FR-001**: Trait-as-contract concept exists but identity is string-keyed, not type-keyed
- **FR-004**: No version constraint resolution (semver range queries absent); only exact version equality implied
- **FR-005 (trait side only)**: `#[service]` on traits generates a blanket `ServiceContract` impl — works. On structs: macro always parses `input as ItemTrait` — will compile-error on structs
- **FR-010**: `ServiceRegistry` exists with `HashMap<String, ServiceDescriptor>` — stores descriptors only, no live implementations, no `register()` method
- **FR-011**: `RegistryError::DuplicateService` variant defined but no enforcement code path
- **FR-012**: Multi-version concept in error enum but no actual multi-version storage
- **FR-013**: No resolve-by-type or version-constraint query
- **FR-019**: `RuntimeBuilder` has `.with_entity::<T>()`, `.with_projection::<P>()`, `.with_service::<S>()`, `.with_service_bundle()` — but `.build()` returns `Ok(Runtime {})` where `Runtime` is an empty struct
- **FR-021c**: `Service::initialize()` and `shutdown()` exist as default no-ops — graceful drain not implemented
- **FR-021q**: `allow_cross_tenant` flag exists and is tested, but no runtime enforcement
- **FR-022a**: `ServiceDescriptor`, `OperationDescriptor`, `ContractDescriptor`, `FieldDescriptor` all exist — but `ContractDescriptor`/`FieldDescriptor` are not re-exported from `lib.rs`
- **FR-022b**: `OperationDescriptor` missing `idempotency` and `read-only vs mutating` flags
- **FR-022c**: `FieldDescriptor` missing required/optional designation
- **FR-022g (trait side)**: `#[service]` on traits auto-generates descriptor — done. Struct side missing
- **FR-026**: `testing.rs` provides `TestService`, `TestServiceFactory`, `TestInterceptor`; no mock generation utilities

### ❌ Missing / Not Implemented

- **FR-005 (struct side)**: `#[service]` on structs for field dependency declaration
- **FR-006**: `EntityRef<T>` — not defined anywhere
- **FR-007**: Read-side projection handler dependency
- **FR-008**: External adapter field dependency
- **FR-009**: Configuration value field dependency
- **FR-014**: Dependency validation before runtime starts
- **FR-015**: Module bundle merging logic
- **FR-015a–e**: No `{TraitName}Ref` generated proxy — `ServiceRef<T>` has wrong shape and returns `Err("not implemented")` for every call
- **FR-016**: Runtime-managed dependency resolver
- **FR-017**: Declaration-based injection (field annotation parsing in macros)
- **FR-018**: Circular dependency detection — `RegistryError::DependencyCycle` exists but no algorithm
- **FR-020**: Pre-start component validation
- **FR-021a**: `Service::initialize()`/`shutdown()` on the base trait **contradicts spec** — must be removed or moved to a separate `LifecycleManaged` trait
- **FR-021b**: No lifecycle contract distinction between runtime-managed and app service components
- **FR-021o**: Context propagation across service-to-service calls not auto-propagated via proxy
- **FR-021p**: No transport adapter integration point for populating `ServiceContext`
- **FR-021r–t**: Cross-tenant enforcement is a boolean flag only — no runtime rejection
- **FR-021w/x**: No `CancellationToken` — only deadline polling; no push-style cancellation
- **FR-022d**: No transport adapter consuming descriptors
- **FR-022g1**: Struct-side descriptor generation entirely absent
- **FR-025d**: Interceptors receive concrete `&ServiceError`, not a trait — violates "operate on trait, not concrete type"
- **FR-027**: No dedicated registry isolation test utilities

---

## Key Findings

1. **Four parallel descriptor hierarchies**: `contract/contract.rs`, `contract/descriptor.rs`, `service/service.rs`, and `operation/operation.rs` all define overlapping types. One canonical source must be designated and the rest deleted.

2. **Registry stores descriptors, not implementations**: `ServiceRegistry` is `HashMap<String, ServiceDescriptor>` — no `Arc<dyn Trait>` storage, no `register()`, no `resolve()`. FR-010–FR-015 requires complete redesign.

3. **RuntimeBuilder is a non-functional stub**: `build()` returns `Ok(Runtime {})` where `Runtime {}` is an empty struct. No graph construction, no validation, no service instantiation.

4. **`ServiceRef<T>` is the wrong shape for FR-015a–e**: Spec requires `OrderServiceRef` — a named type with typed methods. Current `ServiceRef<T>` is generic, unnamed per-service, and `invoke()` always returns `Err`.

5. **`#[service]` macro cannot handle structs**: Always parses `input as ItemTrait`. Applying to a struct will panic at compile time.

6. **`initialize()`/`shutdown()` on `Service` conflicts with FR-021a**: The spec explicitly prohibits init/shutdown on application services. Must be moved to a `LifecycleManaged` marker trait.

7. **`DomainError` duplicated**: Identical trait defined in both `error/domain_error.rs` and `error/category.rs`.

8. **`serde` on `ServiceContext` is a leaky transport concern**: `ServiceContext` derives `Serialize/Deserialize` — serialization belongs in transport adapters, not the domain layer.

9. **No `CancellationToken`**: FR-021w/x requires push-style cancellation. `tokio_util::CancellationToken` not present.

10. **Tests validate structure only, not behavior**: No test exercises the full lifecycle: register → resolve → invoke via proxy → interceptor fires → context propagates → error returns.

---

## Gaps for Proposal

1. **Registry redesign**: Type-keyed registry with live `Arc<dyn Trait>` storage, duplicate rejection, and version-constraint resolution
2. **`{TraitName}Ref` proxy generation**: `#[service]` on traits must emit `{TraitName}Ref` with typed forwarding methods, interceptor chain, and context propagation
3. **`#[service]` on structs**: Extend macro to parse `ItemStruct`, detect field types (`EntityRef<T>`, `ProjectionRef<T>`, `AdapterRef<T>`, config), emit DI metadata
4. **EntityRef / ProjectionRef / AdapterRef**: Define as DI injection primitives with runtime-scope context propagation
5. **RuntimeBuilder wiring**: Implement `.build()` — dependency graph construction, cycle detection (Kahn's algorithm), factory execution, live `Runtime`
6. **Lifecycle contract clarification**: Remove `initialize()`/`shutdown()` from `Service` trait; move to `LifecycleManaged` for runtime-only components
7. **CancellationToken**: Add `cancellation_token: Option<tokio_util::CancellationToken>` to `ServiceContext`; add `tokio-util` to `Cargo.toml`
8. **Descriptor consolidation**: One canonical set in `contract/descriptor.rs`; delete `contract/contract.rs`, `service/service.rs`, `operation/operation.rs`, `version/version.rs`
9. **Remove `serde` from `ServiceContext`**: Move serialization to transport adapter layer
10. **FR-025d compliance**: Define `ServiceErrorTrait` that interceptors program against instead of the concrete `ServiceError` enum

---

## Files Analyzed

- `crates/service-sdk/Cargo.toml`
- `crates/service-sdk-macros/Cargo.toml`
- `crates/service-sdk/src/lib.rs`
- `crates/service-sdk/src/context/mod.rs`
- `crates/service-sdk/src/contract/{mod,contract,descriptor,service_contract,service_contract_trait,version}.rs`
- `crates/service-sdk/src/error/{mod,category,domain_error}.rs`
- `crates/service-sdk/src/implementation.rs`
- `crates/service-sdk/src/interceptor/{mod,chain,builtin/mod}.rs`
- `crates/service-sdk/src/registry/{mod,registry}.rs`
- `crates/service-sdk/src/runtime/{mod,runtime_builder}.rs`
- `crates/service-sdk/src/reference.rs`
- `crates/service-sdk/src/builder.rs`
- `crates/service-sdk/src/dependency/dependency.rs`
- `crates/service-sdk/src/tenant/tenant_id.rs`
- `crates/service-sdk/src/testing.rs`
- `crates/service-sdk/src/lib_tests.rs`
- `crates/service-sdk/src/service_tests.rs`
- `crates/service-sdk/src/logging_example.rs`
- `crates/service-sdk-macros/src/{lib,tests}.rs`
- `crates/service-sdk/tests/{smoke,context_propagation,context_cross_service,context_scope,tenant_isolation,cancellation,deadline_expiry,interceptor_invocation,interceptor_error,simple_tests}.rs`
- `crates/service-sdk/examples/order_service.rs`
