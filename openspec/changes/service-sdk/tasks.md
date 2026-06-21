# Tasks: service-sdk

## Review Workload Forecast

- Estimated changed lines: ~1 450
  - Phase 1 (Cleanup & Foundation): ~260 lines changed/deleted
  - Phase 2 (Registry): ~220 lines added
  - Phase 3 (Macro — Proxy Codegen): ~350 lines added/modified in macros crate
  - Phase 4 (Runtime Wiring): ~380 lines added/modified
  - Phase 5 (End-to-End & Edge Cases): ~240 lines added (tests only)
- Chained PRs recommended: **Yes** — 1 450 lines exceeds the 400-line budget. Recommended split: PR-1 (Phases 1–2), PR-2 (Phase 3), PR-3 (Phases 4–5).
- Decision needed before apply: **Yes** — confirm chain strategy (`stacked-to-main` vs `feature-branch-chain`) and `entity_sdk` availability before starting Phase 3.

---

## Phase 1: Cleanup & Foundation

### [x] TASK-001 — Cargo.toml: add tokio-util, drop uuid/serde from service-sdk

**Spec**: REQ-019, REQ-021, INV-003
**Description**: Add `tokio-util` (with `sync` feature) to `[dependencies]` in `crates/service-sdk/Cargo.toml`; keep `serde` in `[dev-dependencies]` only (needed by integration tests that set up fixtures); remove the `uuid` dependency; remove `serde` from the `uuid` feature flags; confirm `async-trait` and `semver` are present (add `semver = "1"` if missing).
**Files**: `crates/service-sdk/Cargo.toml`
**Test**: `cargo build -p ego-service-sdk` compiles with no `serde` in prod dependency set and `tokio_util::sync::CancellationToken` is resolvable.
**Estimated lines**: ~15 changed

---

### [x] TASK-002 — Delete structural debt: reference.rs, service/, operation/, version/, contract/contract.rs

**Spec**: REQ-021, INV-005
**Description**: Delete `crates/service-sdk/src/reference.rs`, `crates/service-sdk/src/service/`, `crates/service-sdk/src/operation/`, `crates/service-sdk/src/version/`, and `crates/service-sdk/src/contract/contract.rs`; remove the corresponding `pub mod` declarations from `contract/mod.rs` and `lib.rs`; fix any re-export lines that referenced the deleted modules so `cargo build` is green before any new code is written.
**Files**: `crates/service-sdk/src/lib.rs`, `crates/service-sdk/src/contract/mod.rs`, deleted files above
**Test**: `cargo build --workspace` compiles after deletion (no dead-module errors).
**Estimated lines**: ~180 deleted, ~20 changed in mod files

---

### [x] TASK-003 — Descriptor consolidation: canonical `descriptor.rs` + new flags

**Spec**: REQ-021, INV-005
**Description**: Make `crates/service-sdk/src/contract/descriptor.rs` the single definition. Update `OperationDescriptor` to add `idempotent: bool` (default `false`) and `mutating: bool` (default `true`); update `FieldDescriptor` to add `required: bool` (default `true`); remove all `#[derive(Serialize, Deserialize)]` from descriptor types; adjust `OperationDescriptor.input` from `String` to `Vec<String>` to match the macro output and spec; update the `contract/mod.rs` to re-export only from `descriptor.rs`; update `lib.rs` re-exports (`ContractDescriptor`, `FieldDescriptor`). Write tests first: `descriptor_fields::tests::operation_descriptor_has_idempotency_flag` and `descriptor_fields::tests::field_descriptor_has_required_flag`.
**Files**: `crates/service-sdk/src/contract/descriptor.rs`, `crates/service-sdk/src/contract/mod.rs`, `crates/service-sdk/src/lib.rs`
**Test**: Unit tests `operation_descriptor_has_idempotency_flag` and `field_descriptor_has_required_flag` pass; `cargo build --workspace` is green.
**Estimated lines**: ~60 changed

---

### [x] TASK-004 — `ServiceContext` hardening: remove serde, add CancellationToken

**Spec**: REQ-019, INV-003, INV-004
**Description**: Remove `use serde::{Deserialize, Serialize}` and `#[derive(Serialize, Deserialize)]` from `crates/service-sdk/src/context/mod.rs`; add `pub cancellation_token: Option<tokio_util::sync::CancellationToken>` field; add `pub fn is_cancelled(&self) -> bool` (returns `self.cancellation_token.as_ref().map(|t| t.is_cancelled()).unwrap_or(false)`); add `pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self` builder method; update `Default`/`new()` so `cancellation_token: None`. Write tests first in `crates/service-sdk/tests/cancellation.rs`: assert `is_cancelled()` returns `false` before cancel and `true` after `CancellationToken::cancel()`; assert compile-time absence of `Serialize` impl (use `static_assertions` or a `trybuild` compile-fail test).
**Files**: `crates/service-sdk/src/context/mod.rs`, `crates/service-sdk/tests/cancellation.rs`
**Test**: `cancellation::push_style_cancellation_token_observed` passes; workspace compiles without serde on `ServiceContext`.
**Estimated lines**: ~35 changed, ~40 added (test)

---

### [x] TASK-005 — LifecycleManaged split: remove init/shutdown from Service

**Spec**: REQ-017, INV-006
**Description**: In `crates/service-sdk/src/implementation.rs`, remove `async fn initialize(&self)` and `async fn shutdown(&self)` from the `Service` trait; introduce a new `#[async_trait] pub trait LifecycleManaged: Send + Sync` with default implementations for both hooks; remove the inner `TestService` struct's implicit reliance on those methods (the existing test still passes because `Service` no longer requires them). Write tests first in the existing `tests/smoke.rs`: assert a minimal `struct NoLifecycleService` implementing only `Service` compiles and has no `initialize`/`shutdown`; assert a struct implementing `LifecycleManaged` has the hooks callable.
**Files**: `crates/service-sdk/src/implementation.rs`, `crates/service-sdk/tests/smoke.rs`
**Test**: `smoke::service_trait_has_no_lifecycle_hooks` compiles; existing `implementation` unit test passes.
**Estimated lines**: ~30 changed, ~25 added (test)

---

### [x] TASK-006 — ServiceErrorTrait + DomainError deduplication

**Spec**: REQ-020, INV-003
**Description**: Define `pub trait ServiceErrorTrait: Send + Sync` (object-safe) in `crates/service-sdk/src/error/service_error.rs` (or a new `service_error_trait.rs`) with methods `fn code(&self) -> &str`, `fn category(&self) -> ErrorCategory`, `fn message(&self) -> String`; implement it for the existing `ServiceError` enum; delete the duplicated `DomainError` definition from `crates/service-sdk/src/error/category.rs` (keep the one in `domain_error.rs`); ensure `DomainError` also implements `ServiceErrorTrait` via a blanket or explicit impl; update `error/mod.rs` re-exports; update `Interceptor::on_error` signature to take `&dyn ServiceErrorTrait` instead of `&ServiceError` (in `interceptor/chain.rs`); fix `InterceptorChain::on_error` accordingly. Write tests first: `tests/interceptor_error.rs::on_error_receives_service_error_trait` — assert `.code()` and `.category()` are callable on `&dyn ServiceErrorTrait` inside `on_error`; assert the caller receives the original typed error unchanged.
**Files**: `crates/service-sdk/src/error/service_error.rs` (or new file), `crates/service-sdk/src/error/category.rs`, `crates/service-sdk/src/error/domain_error.rs`, `crates/service-sdk/src/error/mod.rs`, `crates/service-sdk/src/interceptor/chain.rs`, `crates/service-sdk/src/interceptor/mod.rs`, `crates/service-sdk/tests/interceptor_error.rs`
**Test**: `interceptor_error::on_error_receives_service_error_trait` passes; `cargo build --workspace` green after signature change.
**Estimated lines**: ~110 changed/added, ~50 added (test)

---

## Phase 2: Registry

### [x] TASK-007 — Type-keyed ServiceRegistry: data model + VersionReq

**Spec**: REQ-001–REQ-005, INV-001
**Description**: Rewrite `crates/service-sdk/src/registry/registry.rs`: define `pub struct ServiceRegistry` with `entries: HashMap<TypeId, Vec<(ContractVersion, Arc<dyn Any + Send + Sync>)>>`; introduce a `{Trait}Tag` pattern (document that tags are emitted by the macro — for now provide a `registry_tag!` test helper macro so tests can exercise the registry without the full macro); define `pub enum VersionConstraint { Exact(String), Range(String) }` (uses `semver::VersionReq` from the `semver` crate internally); implement `pub fn register<Tag: 'static>(&mut self, version: ContractVersion, impl_arc: Arc<dyn Any + Send + Sync>) -> Result<(), RegistryError>`; implement `pub fn resolve_raw<Tag: 'static>(&self, constraint: &VersionConstraint) -> Result<Arc<dyn Any + Send + Sync>, RegistryError>`; implement `pub fn merge(&mut self, other: ServiceRegistry) -> Result<(), RegistryError>`; update `RegistryError` variants to include structured fields: `DuplicateService { name: String, version: String }` and `ServiceNotFound`. Remove `serde` derives from `RegistryError`. Write all unit tests BEFORE implementation: `registry::tests::register_stores_live_implementation`, `registry::tests::register_rejects_duplicate`, `registry::tests::resolve_exact_version`, `registry::tests::resolve_semver_range`, `registry::tests::resolve_returns_not_found`.
**Files**: `crates/service-sdk/src/registry/registry.rs`, `crates/service-sdk/src/registry/mod.rs`, `crates/service-sdk/src/contract/version.rs` (add `VersionConstraint`), `crates/service-sdk/Cargo.toml` (confirm `semver = "1"`)
**Test**: All five registry unit tests pass under `cargo test -p ego-service-sdk registry`.
**Estimated lines**: ~220 added/rewritten

---

### [x] TASK-008 — DI Primitives module: ProjectionRef, AdapterRef, ConfigValue

**Spec**: REQ-011, REQ-009
**Description**: Create `crates/service-sdk/src/di/mod.rs` with `pub struct ProjectionRef<P> { inner: Arc<P> }`, `pub struct AdapterRef<A> { inner: Arc<A> }`, `pub struct ConfigValue<T> { value: Arc<T> }`; implement `Deref` for each to their inner type; add a `pub enum DepKey` discriminating entity/projection/adapter/config by `TypeId`; add `pub trait Injectable: Send + Sync` with `fn dependencies() -> Vec<DepKey>` and `fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> where Self: Sized`; re-export everything from `lib.rs`. Confirm `entity_sdk::EntityRef` import path (add `entity-sdk` to `Cargo.toml` if available, otherwise add a `// TODO: import entity_sdk::EntityRef` placeholder that compiles via a local shim struct with the correct name and visibility so downstream phases are unblocked). Write tests first: `registry::tests::di_primitives_are_recognizable` — assert each wrapper type's `TypeId` is distinguishable via `DepKey`.
**Files**: `crates/service-sdk/src/di/mod.rs` (new), `crates/service-sdk/src/lib.rs`, `crates/service-sdk/Cargo.toml`
**Test**: `registry::tests::di_primitives_are_recognizable` passes.
**Estimated lines**: ~110 added

---

## Phase 3: Macro — Proxy Codegen

> **Prerequisite**: Phase 1 and 2 must be green before this phase. The macro crate has zero runtime dep; changes here do not affect Phase 2's registry correctness.

### [x] TASK-009 — Macro: `#[service]` on traits — emit `{TraitName}Tag` ZST + `{TraitName}Ref` struct

**Spec**: REQ-006, INV-002
**Description**: Extend `crates/service-sdk-macros/src/lib.rs` `service` proc-macro for `ItemTrait`: after emitting the existing `ServiceContract` impl, also emit `pub struct {TraitName}Tag;` (the registry key ZST) and `pub struct {TraitName}Ref { inner: std::sync::Arc<dyn {TraitName}>, chain: std::sync::Arc<ego_service_sdk::interceptor::InterceptorChain>, runtime: std::sync::Weak<ego_service_sdk::runtime::RuntimeInner>, }` with a `pub fn new(inner: Arc<dyn {TraitName}>, chain: Arc<InterceptorChain>, runtime: Weak<RuntimeInner>) -> Self` constructor. Write the test FIRST: add a `trybuild` test case `tests/ui/service_on_trait_generates_tag_and_ref.rs` (or use `macro_rules!` expansion assertions) that verifies `OrderServiceTag` and `OrderServiceRef` are public types after macro expansion and that `OrderServiceRef::new(...)` compiles.
**Files**: `crates/service-sdk-macros/src/lib.rs`, `crates/service-sdk-macros/tests/ui/` (new trybuild fixtures)
**Test**: `macros::tests::service_on_trait_generates_ref_struct` passes (trybuild expansion).
**Estimated lines**: ~120 added in macro, ~30 added in test fixtures

---

### [x] TASK-010 — Macro: `#[service]` on traits — emit typed forwarding `impl TraitName for {TraitName}Ref`

**Spec**: REQ-007, REQ-008, INV-002
**Description**: For each `#[operation]`-annotated method on the trait, emit inside `#[async_trait::async_trait] impl {TraitName} for {TraitName}Ref`: (1) read/create `ServiceContext::current().unwrap_or_default()`; (2) call `if let Some(rt) = self.runtime.upgrade() { rt.enforce_tenant(&ctx)?; }`; (3) wrap body in `ctx.scope(|| async { ... }).await`; (4) call `self.chain.on_request(&ctx).await` before dispatch; (5) `match self.inner.{method}(args).await { Ok(v) => { chain.on_response; Ok(v) } Err(e) => { chain.on_error(&e as &dyn ServiceErrorTrait); Err(e) } }`. Every `#[operation]` method MUST appear in the generated impl — missing one is a compile error enforced by the trait bound. Write tests FIRST: extend `tests/interceptor_invocation.rs` to use a generated `PaymentServiceRef` with a `SpyInterceptor`; assert `on_request` → impl → `on_response` order; assert `on_error` fires when impl returns `Err`.
**Files**: `crates/service-sdk-macros/src/lib.rs`, `crates/service-sdk/tests/interceptor_invocation.rs`, `crates/service-sdk/tests/context_cross_service.rs`
**Test**: `interceptor_invocation::interceptors_fire_in_order_via_generated_ref` passes; `context_cross_service::context_propagates_across_service_boundary` passes.
**Estimated lines**: ~150 added in macro, ~80 changed in tests

---

### [x] TASK-011 — Macro: `#[service]` on structs — field-type detection + Injectable + factory

**Spec**: REQ-009, REQ-010
**Description**: In `service` proc-macro, branch on `parse::<ItemStruct>()` (when `#[service]` is applied to a struct); for each field, inspect the last path segment of its type to classify it as `EntityRef<T>`, `ProjectionRef<P>`, `AdapterRef<A>`, `ConfigValue<T>`, or plain; emit `impl Injectable for {StructName}` with `fn dependencies() -> Vec<DepKey>` listing each classified dep and `fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError>` calling the appropriate resolver for each dep. Write tests FIRST: `macros::tests::service_on_struct_detects_fields` — define a struct with one field of each category and assert the emitted `Injectable::dependencies()` vec contains the correct `DepKey` variants.
**Files**: `crates/service-sdk-macros/src/lib.rs`, `crates/service-sdk-macros/tests/ui/` (new struct fixture)
**Test**: `macros::tests::service_on_struct_detects_fields` passes (trybuild or compile test).
**Estimated lines**: ~100 added in macro, ~40 in test fixtures

---

## Phase 4: Runtime Wiring

> **Prerequisite**: Phases 1–3 must be green. This phase takes the live registry from Phase 2 and the generated proxy from Phase 3 and wires them into a working Runtime.

### TASK-012 — RuntimeInner + RuntimeBuilder factory model (no build() yet)

**Spec**: REQ-012, REQ-015
**Description**: Rewrite `crates/service-sdk/src/runtime/runtime_builder.rs`: define `pub struct RuntimeInner { registry: ServiceRegistry, allow_cross_tenant: bool }` with `pub fn enforce_tenant(&self, ctx: &ServiceContext) -> Result<(), RuntimeError>`; define `pub struct Runtime { inner: Arc<RuntimeInner> }` and `pub struct RuntimeBuilder { factories: Vec<RegisteredFactory>, bundles: Vec<RuntimeBuilder>, allow_cross_tenant: bool }` where `RegisteredFactory = { tag: TypeId, version: ContractVersion, deps: Vec<DepKey>, make: Box<dyn Fn(&RuntimeInner) -> Result<Arc<dyn Any + Send + Sync>, RuntimeError> + Send + Sync> }`; implement `with_entity`, `with_projection`, `with_service`, `with_service_bundle` builder methods that push `RegisteredFactory` entries; implement `allow_cross_tenant()` builder setter. Remove all `serde` derives. Write tests FIRST: `runtime::tests::builder_collects_and_merges_bundles` — assert factory count after merging a two-factory bundle into a builder with two existing factories equals four.
**Files**: `crates/service-sdk/src/runtime/runtime_builder.rs`, `crates/service-sdk/src/runtime/mod.rs`, `crates/service-sdk/src/lib.rs`
**Test**: `runtime::tests::builder_collects_and_merges_bundles` passes.
**Estimated lines**: ~180 rewritten

---

### TASK-013 — RuntimeBuilder::build() — validation + Kahn cycle detection

**Spec**: REQ-013, REQ-014
**Description**: Implement `RuntimeBuilder::build()`: (1) flatten `self.bundles` into one factory list, reject duplicate `(tag,version)` → `RuntimeError::DuplicateService`; (2) build adjacency graph from `deps` fields; for any dep not present in the factory set → `RuntimeError::DependencyNotFound { service, dependency }`; (3) Kahn's algorithm: queue zero-in-degree nodes, pop to `order`, decrement neighbors; if `order.len() != node_count` → `RuntimeError::DependencyCycle { cycle: Vec<String> }` naming unresolved nodes; (4) instantiate in topological order by calling each factory's `make` closure and inserting raw `Arc<dyn Any + Send + Sync>` into a `ServiceRegistry`; (5) return `Ok(Runtime { inner: Arc::new(RuntimeInner { registry, allow_cross_tenant }) })`. Write tests FIRST: `runtime::tests::build_fails_on_missing_dependency`, `runtime::tests::build_detects_dependency_cycle` (three-node A→B→C→A), `runtime::tests::build_constructs_in_dependency_order` (side-effecting factory asserts construction order).
**Files**: `crates/service-sdk/src/runtime/runtime_builder.rs`
**Test**: All three runtime build unit tests pass under `cargo test -p ego-service-sdk runtime`.
**Estimated lines**: ~200 added

---

### TASK-014 — Runtime::resolve() + enforce_tenant() + cross-tenant test

**Spec**: REQ-016, REQ-018, INV-007
**Description**: Implement `Runtime::resolve::<Tag: 'static, Trait: ?Sized + 'static>(&self, constraint: &VersionConstraint)` that calls `self.inner.registry.resolve_raw::<Tag>(constraint)`, downcasts the `Arc<dyn Any>` to `Arc<dyn Trait>`, constructs a `{TraitName}Ref::new(arc, chain, Arc::downgrade(&self.inner))`, and returns it; implement `RuntimeInner::enforce_tenant(&self, ctx: &ServiceContext) -> Result<(), RuntimeError>` that compares `ctx.tenant_id` against the registered component's tenant scope and returns `Err(RuntimeError::CrossTenantViolation { caller_tenant, service_tenant })` when `!ctx.allow_cross_tenant` and there is a mismatch. Write tests FIRST: `runtime::tests::runtime_resolves_proxy_after_build`; update `tests/tenant_isolation.rs` with `tenant_isolation::cross_tenant_denied_when_flag_false` and `tenant_isolation::cross_tenant_allowed_when_flag_true`.
**Files**: `crates/service-sdk/src/runtime/runtime_builder.rs`, `crates/service-sdk/tests/tenant_isolation.rs`
**Test**: `runtime::tests::runtime_resolves_proxy_after_build` passes; both tenant isolation tests pass.
**Estimated lines**: ~130 added, ~60 changed (tests)

---

### TASK-015 — LifecycleManaged runtime driving + lib.rs re-export cleanup

**Spec**: REQ-017, REQ-012
**Description**: In `RuntimeBuilder::build()`, after instantiating all components in topological order, iterate managed components (those whose factory registered them as `LifecycleManaged`-implementing) and call `initialize().await` in construction order; store a `shutdown_order: Vec<Arc<dyn LifecycleManaged>>` in `RuntimeInner` for teardown; expose `Runtime::shutdown(&self) -> Result<(), ServiceError>` that calls `shutdown().await` in reverse order. Update `lib.rs` to remove dead module re-exports (`service`, `operation`, `version`, `reference`, `builder` if subsumed) and add `pub mod di` re-export; add `pub use di::*`. Write tests FIRST: extend `tests/smoke.rs` with `smoke::lifecycle_managed_hooks_fire_for_managed_component` asserting `initialize` is called on a tracked entity adapter and `shutdown` is called on teardown.
**Files**: `crates/service-sdk/src/runtime/runtime_builder.rs`, `crates/service-sdk/src/implementation.rs`, `crates/service-sdk/src/lib.rs`, `crates/service-sdk/tests/smoke.rs`
**Test**: `smoke::lifecycle_managed_hooks_fire_for_managed_component` passes.
**Estimated lines**: ~70 added

---

## Phase 5: End-to-End & Edge Cases

> **Prerequisite**: All Phases 1–4 green. Tests in this phase exercise the full assembled stack.

### TASK-016 — End-to-end smoke test (REQ-022 / TS-010)

**Spec**: REQ-022
**Description**: In `crates/service-sdk/tests/smoke.rs`, write and complete `smoke::end_to_end_register_resolve_invoke_interceptor_context_error`: (1) define `OrderService` trait with `#[service]` and a `place_order` `#[operation]`; (2) implement a concrete `OrderServiceImpl` that returns a domain error for invalid input; (3) `ServiceRegistry::register::<OrderServiceTag>(impl_arc, v1)`; (4) `registry.resolve_raw::<OrderServiceTag>(&VersionConstraint::Range("^1"))`; (5) construct `OrderServiceRef` with a `SpyInterceptor`; (6) call `place_order(invalid_input).await`; (7) assert `on_request` fired, `on_error` fired with `&dyn ServiceErrorTrait`, caller received the typed `OrderError`. All six assertions from REQ-022 must pass.
**Files**: `crates/service-sdk/tests/smoke.rs`
**Test**: `smoke::end_to_end_register_resolve_invoke_interceptor_context_error` passes.
**Estimated lines**: ~120 added

---

### TASK-017 — Context propagation tests: cross-service + CancellationToken

**Spec**: REQ-008, REQ-019, TS-003, TS-008
**Description**: Update `tests/context_cross_service.rs` to assert `ServiceContext::current()` inside an impl body returns the same `tenant_id` and `trace_id` set by the outer caller via `ServiceContext::scope(ctx, ...)` through a generated `{TraitName}Ref`; assert `ServiceContext::current()` returns `None` after scope exits. Extend `tests/cancellation.rs` with `cancellation::operation_aborts_on_cancelled_token`: launch an async operation that polls `ctx.is_cancelled()` in a loop; cancel the token from a spawned task; assert the operation terminates and returns the expected error.
**Files**: `crates/service-sdk/tests/context_cross_service.rs`, `crates/service-sdk/tests/cancellation.rs`
**Test**: `context_cross_service::context_propagates_via_generated_ref` passes; `cancellation::operation_aborts_on_cancelled_token` passes.
**Estimated lines**: ~80 changed/added

---

### TASK-018 — Registry edge cases: semver range + TS-001 full lifecycle

**Spec**: REQ-003, REQ-004, REQ-005, TS-001
**Description**: Add or complete integration-test coverage in `crates/service-sdk/tests/simple_tests.rs` (or a dedicated `tests/registry_lifecycle.rs`) covering TS-001 steps 1–7: register v1.0.0 + v2.0.0, resolve with `Exact("1.0.0")`, resolve with `Range("^2")`, attempt duplicate registration at v1.0.0 (assert `DuplicateService`), attempt `Range("^3")` (assert `ServiceNotFound`). Also add `macros::tests::service_on_struct_factory_constructs_with_all_deps` verifying REQ-010: register all deps, call the generated factory, assert struct is constructed correctly.
**Files**: `crates/service-sdk/tests/simple_tests.rs` (or new `tests/registry_lifecycle.rs`), `crates/service-sdk-macros/tests/`
**Test**: All TS-001 assertions pass; factory construction test passes.
**Estimated lines**: ~80 added

---

### TASK-019 — Coverage gate + workspace compile verification

**Spec**: INV-001–INV-008, strict TDD coverage gate
**Description**: Run `cargo tarpaulin --workspace --out Html --fail-under 95` and patch any uncovered branches: ensure cycle-detection branch (`DependencyCycle`), `CrossTenantViolation` branch, `DomainError` passthrough, and `ServiceErrorTrait` blanket impl are each hit by at least one existing test; add minimal targeted tests for any uncovered line without writing new production code. Confirm `cargo build --workspace` produces zero warnings under `#![deny(unused_imports)]` by fixing any leftover dead imports from Phase 1 deletions. This task is verification + cleanup only — no new production logic.
**Files**: Any file with uncovered branches (identified at runtime); no new source files expected.
**Test**: `cargo tarpaulin --workspace --fail-under 95` exits 0; `cargo test --workspace` exits 0.
**Estimated lines**: ~30 (targeted coverage fills)

---

## Parallelism Notes

| Task | Depends on | Can run in parallel with |
|---|---|---|
| TASK-001 | — | TASK-002 |
| TASK-002 | — | TASK-001 |
| TASK-003 | TASK-002 | — |
| TASK-004 | TASK-002 | TASK-003, TASK-005, TASK-006 |
| TASK-005 | TASK-002 | TASK-003, TASK-004, TASK-006 |
| TASK-006 | TASK-002 | TASK-003, TASK-004, TASK-005 |
| TASK-007 | TASK-001, TASK-003 | TASK-008 |
| TASK-008 | TASK-001, TASK-003 | TASK-007 |
| TASK-009 | TASK-007, TASK-008 | — |
| TASK-010 | TASK-006, TASK-009 | TASK-011 |
| TASK-011 | TASK-008, TASK-009 | TASK-010 |
| TASK-012 | TASK-007, TASK-008 | TASK-010, TASK-011 |
| TASK-013 | TASK-012 | — |
| TASK-014 | TASK-010, TASK-013 | TASK-015 |
| TASK-015 | TASK-005, TASK-013 | TASK-014 |
| TASK-016 | TASK-014, TASK-015 | TASK-017, TASK-018 |
| TASK-017 | TASK-010, TASK-014 | TASK-016, TASK-018 |
| TASK-018 | TASK-007, TASK-011 | TASK-016, TASK-017 |
| TASK-019 | TASK-016, TASK-017, TASK-018 | — |

## Invariant Coverage Map

| Invariant | Covered by task(s) |
|---|---|
| INV-001: TypeId-keyed registry | TASK-007 |
| INV-002: Proxy completeness | TASK-010 |
| INV-003: Transport freedom (no serde on domain types) | TASK-001, TASK-004, TASK-007 |
| INV-004: Context isolation (None outside scope) | TASK-017 |
| INV-005: Single descriptor authority | TASK-002, TASK-003 |
| INV-006: Lifecycle separation | TASK-005, TASK-015 |
| INV-007: Runtime sole enforcement authority | TASK-014 |
| INV-008: EntityRef<T> from entity_sdk only | TASK-008 |
