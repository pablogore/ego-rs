# Tasks: Service SDK

**Feature**: `008-service-sdk`  
**Plan**: [plan.md](./plan.md) | **Spec**: [spec.md](./spec.md)  
**Generated**: 2026-06-08

## Dependency Graph

```
Setup (Phase 1)
  │
  ▼
Foundational (Phase 2) — descriptor types, error types, context types
  │
  ├─► US1 (Phase 3) — Service Contracts + proc-macro
  │     │
  │     ▼
  ├─► US2 (Phase 4) — Service Registry (depends on US1)
  │     │
  │     ▼
  ├─► US3 (Phase 5) — Dependency Injection & Wiring (depends on US2)
  │     │
  │     ▼
  ├─► US4 (Phase 6) — Runtime Composition (depends on US3)
  │
  └─► US5 (Phase 7) — Interceptors + Context + Tenant (parallel with US3-4)
        │
        ▼
Polish (Phase 8) — Integrations, docs, final validation
```

US5 can run in parallel with US3-US4 since it depends on US1/US2 but not on DI or runtime composition.

## Implementation Strategy

**MVP Scope** (Phases 1–4): Service contracts, registry with version resolution, and basic ServiceRef resolution. This delivers the core value — define services and resolve them.

**Incremental Delivery**:
1. **MVP**: US1 + US2 → contracts and registry (transport-agnostic)
2. **v0.2**: US3 → dependency injection and field-declared wiring
3. **v0.3**: US4 → runtime builder integration
4. **v0.4**: US5 → interceptors, context, tenant isolation
5. **v1.0**: Polish, docs, full coverage

---

_## Phase 1: Setup (Project Initialization)

Goal: Create the two new crates, configure workspace, establish module layout.

- [x] T001 [P] Create `crates/service-sdk-macros/Cargo.toml` with `proc-macro = true`, dependencies on `syn`, `quote`, `proc-macro2`
- [x] T002 [P] Create `crates/service-sdk/Cargo.toml` with dependencies on `ego-domain`, `persistent-entity`, `async-trait`, `tokio`, `uuid`, `thiserror`, `ego-service-sdk-macros`
- [x] T003 Add `crates/service-sdk` and `crates/service-sdk-macros` to workspace `Cargo.toml` members array
- [x] T004 Create `crates/service-sdk-macros/src/lib.rs` — proc-macro crate root with `#[proc_macro_attribute]` entry points for `service` and `operation`
- [x] T005 Create `crates/service-sdk/src/lib.rs` — crate root with module declarations and re-exports_

---

## Phase 2: Foundational Types (Blocking Prerequisites)

Goal: Core types that all user stories depend on. Must complete before any user story phase.

- [x] T006 [P] Create `crates/service-sdk/src/contract/version.rs` — `ContractVersion` struct (major, minor, patch) with `Display`, `PartialOrd`, `FromStr` impls
- [x] T007 [P] Create `crates/service-sdk/src/contract/descriptor.rs` — `ServiceDescriptor`, `OperationDescriptor`, `OperationCategory`, `ContractDescriptor`, `FieldDescriptor` structs
- [x] T008 [P] Create `crates/service-sdk/src/contract/mod.rs` — Re-export version and descriptor types
- [x] T009 [P] Create `crates/service-sdk/src/error/category.rs` — `ErrorCategory` enum (Validation, NotFound, Conflict, Authorization, BusinessRule, Infrastructure)
- [x] T010 [P] Create `crates/service-sdk/src/error/domain_error.rs` — `DomainError` trait with `code()`, `category()`, requiring `std::error::Error + Send + Sync`
- [x] T011 [P] Create `crates/service-sdk/src/error/mod.rs` — Re-export `DomainError`, `ErrorCategory`
- [x] T012 Create `crates/service-sdk/src/contract/service_contract.rs` — `ServiceContract` trait with `type_id()`, `name()`, `version()`, `descriptor()` methods
- [x] T013 Create `crates/service-sdk/src/context/context.rs` — `ServiceContext` struct with fields: tenant_id, correlation_id, trace_id, deadline, timeout, metadata, cancellation; `current()` returning `Option<&ServiceContext>` (never panics, returns None outside invocation scope); `scope()` method using `tokio::task::TaskLocal`

---

## Phase 3: User Story 1 — Declare an Application Service Contract (P1)

Goal: Developers can declare service contracts via `#[service]` and `#[operation]` attributes. Descriptors are auto-generated. Contracts are transport-agnostic.

**Independent Test**: Define an OrderService trait with `create_order` and `get_order`, verify `ServiceDescriptor` is generated with correct operations and no transport annotations.

### Tests for US1
- [x] T014 [P] [US1] Write test for `#[service]` proc-macro output in `crates/service-sdk-macros/tests/service_macro.rs` — verify generated `ServiceContract` impl has correct name, version, operations
- [x] T015 [P] [US1] Write test for `#[operation]` macro in `crates/service-sdk-macros/tests/operation_macro.rs` — verify `OperationDescriptor` metadata (name, category)
- [x] T016 [P] [US1] Write test for version metadata in `crates/service-sdk-macros/tests/version_metadata.rs` — verify `#[service(version = "1.2.3")]` produces correct `ContractVersion`
- [x] T017 [US1] Write contract declaration integration test in `crates/service-sdk/tests/contract_declaration.rs` — define service trait, assert descriptor available at runtime

### Implementation for US1
- [x] T018 [P] [US1] Implement `#[service]` proc-macro in `crates/service-sdk-macros/src/lib.rs` — parse trait, extract name, version, operations; generate `ServiceContract` impl
- [x] T019 [P] [US1] Implement `#[operation]` proc-macro attribute in `crates/service-sdk-macros/src/lib.rs` — parse method signature, extract name, input/output types, category
- [x] T020 [US1] Implement `ServiceContract` trait methods on generated impl in `crates/service-sdk-macros/src/lib.rs` — `type_id()` returns `TypeId::of::<Self>()`, `name()` returns trait name as `&'static str`
- [x] T021 [US1] Implement `descriptor()` generation in `crates/service-sdk-macros/src/lib.rs` — collect `OperationDescriptor` vec, build `ServiceDescriptor` with all metadata
- [x] T022 [US1] Create `crates/service-sdk/src/contract/` module with re-exports of macros (`pub use ego_service_sdk_macros::service;`)

---

## Phase 4: User Story 2 — Resolve Services From a Central Registry (P2 in spec, P1 priority)

Goal: Developers register service implementations and resolve them by contract type. Registry validates dependencies, rejects duplicates, supports versioning.

**Independent Test**: Register two service implementations, resolve each by type, verify correct implementation returned. Duplicate registration rejected.

### Tests for US2
- [x] T023 [P] [US2] Write registry registration test in `crates/service-sdk/tests/registry_register.rs` — register service, resolve, assert handle is valid
- [x] T024 [P] [US2] Write duplicate rejection test in `crates/service-sdk/tests/registry_duplicate.rs` — register same type+name+version twice, assert error
- [x] T025 [P] [US2] Write version resolution test in `crates/service-sdk/tests/registry_version.rs` — register v1 and v2, resolve latest vs exact
- [x] T026 [US2] Write bundle merge test in `crates/service-sdk/tests/registry_bundle.rs` — create two bundles, merge, resolve services from each

### Implementation for US2
- [x] T027 [P] [US2] Create `crates/service-sdk/src/registry/entry.rs` — `RegistryEntry` struct holding `Arc<dyn Any + Send + Sync>`, `ServiceDescriptor`, dependency `Vec<TypeId>`
- [x] T028 [P] [US2] Create `crates/service-sdk/src/registry/error.rs` — `RegistryError` enum (DuplicateRegistration, NotFound, MissingDependency, VersionConflict, BundleMergeConflict)
- [x] T029 [US2] Create `crates/service-sdk/src/registry/registry.rs` — `ServiceRegistry` struct with `HashMap<RegistryKey, RegistryEntry>`, `register()`, `resolve()`, `validate()`, `add_interceptor()`
- [x] T030 [US2] Implement `register()` in `crates/service-sdk/src/registry/registry.rs` — check for duplicates, insert entry with `TypeId`, optional name, version
- [x] T031 [US2] Implement `resolve()` in `crates/service-sdk/src/registry/registry.rs` — filter by `(TypeId, name)`, select by version constraint (latest or exact). Downcast registry entry once during resolution. Construct and return generated proxy type (e.g., `OrderServiceRef`). The invocation path MUST NOT perform runtime downcasting.
- [x] T032 [US2] Implement `validate()` in `crates/service-sdk/src/registry/registry.rs` — check all registered dependencies are satisfiable, return `Vec<RegistryError>` on failure
- [x] T033 [US2] Create `crates/service-sdk/src/registry/bundle.rs` — `ServiceBundle` struct with `module_name`, `register()`, consumed by `ServiceRegistry::merge()`
- [x] T034 [US2] Implement `merge()` in `crates/service-sdk/src/registry/registry.rs` — merge bundle entries, reject version conflicts
- [x] T035 [US2] Create `crates/service-sdk/src/registry/mod.rs` — Re-export registry, bundle, entry, error

---

## Phase 5: User Story 3 — Wire Services With Runtime Dependencies (P2)

Goal: Services declare dependencies as annotated fields. The runtime resolves them automatically at construction time via the builder pattern.

**Independent Test**: Define a service with `EntityRef` field, register with runtime builder, invoke service operation, verify entity processes command.

### Tests for US3
- [x] T036 [P] [US3] Write dependency declaration test in `crates/service-sdk/tests/dependency_declaration.rs` — define service with `EntityRef<T>` field, verify dependency metadata generated
- [x] T037 [P] [US3] Write dependency resolution test in `crates/service-sdk/tests/dependency_resolution.rs` — register service + mock entity, resolve service, verify dependency injected
- [x] T038 [P] [US3] Write missing dependency test in `crates/service-sdk/tests/dependency_missing.rs` — register service without its dependency, validate fails
- [x] T039 [US3] Write circular dependency test in `crates/service-sdk/tests/dependency_circular.rs` — define A depends on B depends on A, assert wiring rejected

### Implementation for US3
- [x] T040 [P] [US3] Implement dependency scanning in proc-macro — detect fields typed `EntityRef<T>`, `ServiceRef<T>`, `Arc<T>`, `Configuration` in `crates/service-sdk-macros/src/lib.rs`
- [x] T041 [P] [US3] Generate `Dependencies` associated type and `resolve_dependencies()` method in proc-macro output in `crates/service-sdk-macros/src/lib.rs`
- [x] T041a [US3] Implement `#[service]` support on structs in `crates/service-sdk-macros/src/lib.rs` — detect struct target type, generate DependencyMetadata, DependencyGraph metadata, Injectable implementation metadata, Runtime wiring metadata. Macro behavior determined by target type (trait vs struct).
- [x] T042 [US3] Create `crates/service-sdk/src/implementation/service_impl.rs` — `ServiceImplementation` marker trait, dependency resolution helpers
- [x] T043 [US3] Create `crates/service-sdk/src/implementation/mod.rs` — module root
- [x] T044 [US3] Implement circular dependency detection in `crates/service-sdk/src/registry/registry.rs` — traverse dependency graph, reject cycles during `validate()` with clear error reporting
- [x] T045 [US3] Implement generated proxy type in proc-macro — `#[service]` on trait generates `{TraitName}Ref` concrete type (e.g., `OrderServiceRef`) in `crates/service-sdk-macros/src/lib.rs`. Proxy holds `Arc<dyn ServiceTrait>` typed reference (NOT Arc<dyn Any> — no type erasure, no runtime downcasting). Implements service trait via direct method calls: `self.inner.create_order(cmd).await`. Each method: enters ServiceContext scope, runs interceptor chain via pre/post/error hooks, calls inner via trait method, returns result. No string-based dispatch. No operation lookup by name. No reflection.
- [x] T046 [US3] Create `crates/service-sdk/src/reference/mod.rs` — module root for ServiceRef

---

## Phase 6: User Story 4 — Compose the Runtime From Wired Modules (P2)

Goal: Runtime builder pattern accepts entities, projections, and services. Validates completeness before startup. Makes services available for invocation.

**Independent Test**: Build runtime with entity + service, start, invoke service via ServiceRef, verify runtime operational.

### Tests for US4

- [x] T047 [P] [US4] Write runtime builder test in `crates/runtime/tests/builder_services.rs` — build runtime with service, assert no error
- [x] T048 [P] [US4] Write missing service startup test in `crates/runtime/tests/builder_missing.rs` — attempt start with missing dependency, assert clear error
- [x] T049 [US4] Write full runtime lifecycle test in `crates/runtime/tests/runtime_lifecycle.rs` — start with services, verify operational, invoke service, shutdown

### Implementation for US4

- [x] T050 [US4] Create `crates/service-sdk/src/builder/runtime_builder.rs` — extension methods for `RuntimeBuilder`: `with_entity::<E>()`, `with_projection::<P>()`, `with_service::<S>()`, `with_service_bundle()`, `build()`
- [x] T051 [US4] Implement `with_entity::<E>()` in `crates/service-sdk/src/builder/runtime_builder.rs` — register entity type in dependency resolver
- [x] T052 [US4] Implement `with_service::<S>()` in `crates/service-sdk/src/builder/runtime_builder.rs` — auto-construct service, resolve all field-declared dependencies, register in registry
- [x] T053 [US4] Implement `build()` in `crates/service-sdk/src/builder/runtime_builder.rs` — validate all dependencies, construct service graph, return operational runtime
- [x] T054 [US4] Create `crates/service-sdk/src/builder/mod.rs` — module root
- [x] T055 [US4] Integrate builder extension into `crates/runtime/src/runtime/runtime.rs` — add service resolution to existing runtime startup, make services queryable post-start
- [x] T056 [US4] Implement shutdown in `crates/runtime/src/runtime/runtime.rs` — stop accepting invocations, drain in-flight ops, drop service references

---

## Phase 7: User Story 5 — Interceptors, Context & Multi-Tenant Isolation (P3)

Goal: Interceptors instrument invocations. ServiceContext propagates metadata. Tenant isolation enforced by runtime. Deadline/cancellation propagated.

**Independent Test**: Register interceptor, invoke service, verify hooks called. Set tenant context, attempt cross-tenant access, verify rejection.

### Tests for US5

- [x] T057 [P] [US5] Write interceptor test in `crates/service-sdk/tests/interceptor_invocation.rs` — register counting interceptor, invoke, assert `on_request`/`on_response` called
- [x] T058 [P] [US5] Write interceptor error test in `crates/service-sdk/tests/interceptor_error.rs` — service returns error, assert `on_error` called with correct `DomainError`
- [x] T059 [P] [US5] Write ServiceContext propagation test in `crates/service-sdk/tests/context_propagation.rs` — set context with tenant, invoke service, assert service sees context
- [x] T060 [P] [US5] Write cross-service context test in `crates/service-sdk/tests/context_cross_service.rs` — service A calls service B, verify context propagated
- [x] T061 [P] [US5] Write tenant isolation test in `crates/service-sdk/tests/tenant_isolation.rs` — set tenant-A context, attempt entity access in tenant-B, assert rejection
- [x] T062 [P] [US5] Write deadline propagation test in `crates/service-sdk/tests/deadline_expiry.rs` — set deadline 100ms, invoke slow service, assert timeout error
- [x] T063 [US5] Write cancellation test in `crates/service-sdk/tests/cancellation.rs` — signal cancellation mid-invocation, assert invocation terminated

### Implementation for US5

- [x] T064 [P] [US5] Create `crates/service-sdk/src/interceptor/interceptor.rs` — `Interceptor` trait with `on_request`, `on_response`, `on_error` async methods
- [x] T065 [P] [US5] Create `crates/service-sdk/src/interceptor/interceptor.rs` — `InterceptorChain` struct, sequential hook execution, error isolation (interceptor failure never fails invocation)
- [x] T066 [US5] Create `crates/service-sdk/src/interceptor/builtin/tracing.rs` — `TracingInterceptor` that creates tracing spans for `on_request`/`on_response`/`on_error`
- [x] T067 [US5] Create `crates/service-sdk/src/interceptor/builtin/mod.rs` — module root for built-in interceptors
- [x] T068 [US5] Create `crates/service-sdk/src/interceptor/mod.rs` — re-export `Interceptor`, `InterceptorChain`, builtins
- [x] T069 [US5] Integrate interceptor chain into `ServiceRef<T>` in `crates/service-sdk/src/reference/service_ref.rs` — before method delegation, run chain hooks
- [x] T070 [US5] Implement `ServiceContext::scope()` in `crates/service-sdk/src/context/context.rs` — using `tokio::task::TaskLocal`, set context for async scope
- [x] T071 [US5] Implement automatic ServiceContext propagation across service-to-service calls in `crates/service-sdk/src/reference/service_ref.rs` — read current context, pass to callee's scope
- [x] T072 [US5] Implement EntityRef runtime scope propagation in `crates/service-sdk/src/context/context.rs` — EntityRef<T> reads ServiceContext from the runtime invocation scope (TaskLocal) transparently. No API changes to persistent-entity. EntityRef<T> MUST NOT depend on service-sdk types. Tenant ID, trace ID, correlation ID, deadline, and cancellation are bound by the runtime to EntityRef execution boundaries.
- [x] T073 [US5] Implement cross-tenant opt-in in `crates/service-sdk/src/context/tenant.rs` — explicit `allow_cross_tenant()` flag on ServiceContext. Cross-tenant access rejected by default; runtime checks tenant ID from invocation scope before EntityRef operations.
- [x] T074 [US5] Implement deadline enforcement in `crates/service-sdk/src/context/context.rs` — check `deadline` before each operation, return timeout error if expired
- [x] T075 [US5] Implement cancellation propagation in `crates/service-sdk/src/context/context.rs` — check `cancellation` token before each operation, abort if cancelled
- [x] T076 [US5] Create `crates/service-sdk/src/context/mod.rs` — re-export context and tenant types

---

## Phase 8: Polish & Cross-Cutting Concerns

Goal: Documentation, coverage verification, integration validation, final compliance checks.

- [x] T077 [P] Add rustdoc documentation to all public types in `crates/service-sdk/src/` — `ServiceContract`, `ServiceRegistry`, generated service proxies (`{TraitName}Ref`), `Interceptor`, `DomainError`, `ServiceContext`, `ContractVersion`
- [x] T078 [P] Add rustdoc documentation to proc-macro in `crates/service-sdk-macros/src/lib.rs` — `#[service]`, `#[operation]` usage examples
- [x] T079 [P] Verify SC-007: zero transport dependencies in `crates/service-sdk/Cargo.toml` and `crates/service-sdk-macros/Cargo.toml`
- [x] T080 Run `cargo test --workspace` — verify all tests pass, no regressions in dependent crates
- [x] T081 Run `cargo clippy --workspace` — verify no warnings
- [x] T082 Run `cargo fmt --check --all` — verify formatting
- [x] T083 Verify code coverage >= 85% (cargo tarpaulin or equivalent)
- [x] T084 Write integration smoke test in `crates/service-sdk/tests/smoke.rs` — full flow: declare contract → implement → register → resolve → invoke → verify result
- [x] T085 Update `docs/architecture.md` — document Service SDK layer in the architecture overview, crate boundaries, dependency direction
- [x] T086 Write DX acceptance test in `crates/service-sdk/examples/order_service.rs` — full developer journey: `#[service]` trait + struct, `EntityRef<Order>` field-dep, `EgoRuntime::builder().with_entity().with_service().build()`, service invocation via `orders.create_order(cmd).await?`. Must compile and execute. Verification: no transport deps, no manual descriptors, no manual DI wiring, no string-based invocation. Serves as ergonomics regression target per SC-008.

---

## Parallel Execution Opportunities

### Phase 2 (Foundational) — All parallel
```
T006 ─┬─ T007 ─┬─ T012
T008 ─┘  T009 ─┤
T010 ─┬─ T011 ─┘
T013 ─┘
```

### Phase 3 (US1) — Tests parallel with implementation
```
T014, T015, T016 (tests parallel)
T018, T019 (proc-macro parallel, then T020, T021 sequential)
T017 (integration test, after implementation)
```

### Phase 4 (US2) — Tests parallel, types parallel
```
T023, T024, T025 (tests parallel)
T027, T028 (types parallel)
T029 → T030 → T031 → T032 → T033 → T034 (registry sequential)
T026 (integration test, after)
```

### Phase 5 (US3) — Tests parallel with implementation
```
T036, T037, T038, T039 (tests parallel)
T040, T041 (proc-macro parallel)
T042 → T043 → T044 → T045 (implementation sequential)
```

### Phase 7 (US5) — Interceptors, Context, Tenant all parallel sub-groups
```
T064, T065 → T066 → T067 (interceptors)
T070 → T074, T075 (context)
T072, T073 (tenant, depends on context)
T069 (integration, after interceptors + context)
```

---

## Summary

| Phase | Story | Task Count | Priority |
|-------|-------|-----------|----------|
| Phase 1 | Setup | 5 | — |
| Phase 2 | Foundational | 8 | — |
| Phase 3 | US1 — Service Contracts | 9 | P1 |
| Phase 4 | US2 — Service Registry | 13 | P1 |
| Phase 5 | US3 — Dependency Injection | 12 | P2 |
| Phase 6 | US4 — Runtime Composition | 10 | P2 |
| Phase 7 | US5 — Interceptors & Context | 20 | P3 |
| Phase 8 | Polish | 10 | — |
| **Total** | | **87** | |

### MVP Scope (Phases 1–4): 35 tasks

Delivers: service contract declaration + attribute-based descriptor generation + service registry with version resolution + bundle merging.

### Suggested MVP Validation

```bash
cargo test -p ego-service-sdk-macros
cargo test -p ego-service-sdk -- registry_register registry_duplicate registry_version
```
