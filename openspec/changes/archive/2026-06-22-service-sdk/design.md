# Design: service-sdk

## Architecture Overview

The SDK splits into four layers, none coupled to transport:

```
   #[service] trait  ──macro──▶  {TraitName}Ref (proxy)  ──┐
   #[service] struct ──macro──▶  Injectable + factory       │
                                                            ▼
   RuntimeBuilder ──build()──▶  Runtime { registry, ctx authority }
        │  (Kahn sort)                  │
        └── factories ──▶ Arc<dyn Trait> instances ──▶ ServiceRegistry
                                                            │
   caller ── runtime.resolve::<dyn Trait>() ──▶ {TraitName}Ref ──invoke──▶ impl
                                  └─ runtime enters scope + tenant check ─┘
```

A call always flows proxy → `Runtime::enforce_tenant()` → interceptor chain → `ServiceContext::scope` → impl. The **Runtime is the sole tenant-enforcement authority and policy owner**. Generated proxies MUST call `Runtime::enforce_tenant()` before every dispatch — this is mandatory, not defensive. No public API may expose a callable service implementation without passing through a Runtime-managed proxy.

## Module Structure (final)

```
service-sdk/src/
  lib.rs                 re-exports ContractDescriptor/FieldDescriptor + DI primitives
  context/mod.rs         ServiceContext (no serde) + cancellation_token + tenant guard
  contract/
    descriptor.rs        CANONICAL descriptors (+ idempotency/readonly/required flags)
    service_contract.rs  ServiceContract trait (kept)
    version.rs           ContractVersion + new VersionReq (semver range)
    mod.rs
  di/mod.rs              NEW: ProjectionRef<P>, AdapterRef<A>, ConfigValue<T>, Injectable
  error/
    service_error.rs     ServiceError enum + ServiceErrorTrait (object-safe)
    category.rs          ErrorCategory only (DomainError removed here)
    domain_error.rs      single DomainError + IntoServiceError
  implementation.rs      Service (no init/shutdown) + LifecycleManaged
  interceptor/chain.rs   hooks take &dyn ServiceErrorTrait
  registry/registry.rs   type-keyed live registry
  runtime/runtime_builder.rs  RuntimeBuilder + Runtime
  reference.rs           DELETED (ServiceRef<T> gone)
```

**Deleted** (descriptor consolidation): `contract/contract.rs`, `service/` (service.rs+mod), `operation/` (operation.rs+mod), `version/version.rs` (folder; semver moves under `contract/version.rs`), `reference.rs`. The canonical survivors are `contract/descriptor.rs` + `contract/version.rs`.

## Component Designs

### ServiceRegistry

```rust
struct RegistryKey { type_id: TypeId, version: ContractVersion }

pub struct ServiceRegistry {
    // value is Arc<dyn Any + Send + Sync> holding the raw implementation (NOT the proxy).
    // Proxies ({TraitName}Ref) are created by Runtime::resolve(), never stored here.
    entries: HashMap<TypeId, Vec<(ContractVersion, Arc<dyn Any + Send + Sync>)>>,
}

impl ServiceRegistry {
    pub fn register<S: ?Sized + 'static>(
        &mut self, version: ContractVersion, impl_arc: Arc<dyn Any + Send + Sync>,
    ) -> Result<(), RegistryError>; // Err(DuplicateService) if (TypeId,version) exists
    pub fn resolve_raw<S: ?Sized + 'static>(
        &self, req: &VersionReq,
    ) -> Result<Arc<dyn Any + Send + Sync>, RegistryError>; // returns raw impl, not proxy
    pub fn merge(&mut self, other: ServiceRegistry) -> Result<(), RegistryError>;
}
```

Key = `TypeId` of the **trait object marker** (`TypeId::of::<dyn OrderService>` is unstable, so we key on a generated zero-sized `OrderServiceTag` type emitted by the macro). Version range expressed by new `VersionReq { min, max_excl, op }` matched against the per-type version list; `resolve` picks the highest satisfying version. Duplicate detection fires inside `register` on exact `(tag, version)` collision. `merge` re-runs `register` per entry so bundle merging inherits duplicate rejection.

### `{TraitName}Ref` Proxy Codegen

Given:
```rust
#[service(version="1.0.0")]
trait OrderService { #[operation] async fn create_order(&self, c: CreateOrder)
    -> Result<OrderId, OrderError>; }
```
the macro emits (in addition to the existing `ServiceContract` impl):

```rust
pub struct OrderServiceTag;            // type key for the registry

pub struct OrderServiceRef {
    inner: Arc<dyn OrderService>,
    chain: Arc<InterceptorChain>,
    runtime: Weak<RuntimeInner>,       // tenant-enforcement authority
}

#[async_trait::async_trait]
impl OrderService for OrderServiceRef {
    async fn create_order(&self, c: CreateOrder) -> Result<OrderId, OrderError> {
        let ctx = ServiceContext::current().unwrap_or_default();
        if let Some(rt) = self.runtime.upgrade() { rt.enforce_tenant(&ctx)?; } // sole barrier
        ctx.scope(|| async {
            if let Err(e) = self.chain.on_request(&ctx).await { /* map */ }
            match self.inner.create_order(c).await {
                Ok(v)  => { self.chain.on_response(&ctx).await.ok(); Ok(v) }
                Err(e) => { self.chain.on_error(&ctx, &e).await.ok(); Err(e) }
            }
        }).await
    }
}
```

`async_trait` already boxes futures, so the proxy `impl` is just another `#[async_trait]` impl of the SAME trait — callers depend only on `dyn OrderService`. The chain lives behind `Arc<InterceptorChain>` (shared, cheap clone). Per-call `ServiceContext::scope` re-enters the task-local so nested service-to-service calls inherit context (FR-021o). Tenant enforcement maps to the domain error via `IntoServiceError`/the trait's error; where the trait error cannot represent it, the proxy returns the runtime error through a generated `From`. Registry stores the raw `Arc<dyn OrderService>` implementation as `Arc<dyn Any>` keyed by `OrderServiceTag`. The proxy (`OrderServiceRef`) is constructed on demand by `Runtime::resolve()`, wrapping the raw impl with the interceptor chain and a `Weak<RuntimeInner>` for tenant enforcement.

### `#[service]` on Structs

Macro branches on `parse::<ItemTrait>()` vs `ItemStruct`. For structs it classifies each field by the **last path segment** of its type:

| Field type pattern | Treated as | Resolution |
|---|---|---|
| `EntityRef<T>` (from entity_sdk) | entity dep | runtime.resolve_entity::<T> |
| `ProjectionRef<P>` | projection dep | runtime.resolve_projection::<P> |
| `AdapterRef<A>` | adapter dep | runtime.resolve_adapter::<A> |
| `ConfigValue<T>` / annotated config | config | runtime.config::<T> |
| anything else | plain | `Default::default()` (no injection) |

Emits an `Injectable` impl carrying the dependency list (for the Kahn graph) and a factory:

```rust
impl Injectable for OrderServiceImpl {
    fn dependencies() -> Vec<DepKey> { vec![DepKey::entity::<Order>()] }
    fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> {
        Ok(Self { orders: rt.entity_ref::<Order>()? })
    }
}
```

### DI Primitives

```rust
pub struct ProjectionRef<P> { inner: Arc<P> }   // entity_sdk owns EntityRef<T>
pub struct AdapterRef<A>    { inner: Arc<A> }
pub struct ConfigValue<T>   { value: Arc<T> }
```
Each derefs to its inner and reads `ServiceContext::current()` at call time, so the active tenant/correlation scope propagates automatically (task-local, no field plumbing). Import path: `use entity_sdk::EntityRef;` — never redefined locally.

### RuntimeBuilder & Runtime

Builder stores **factories**, not instances:

```rust
pub struct RuntimeBuilder {
    factories: Vec<RegisteredFactory>,   // { tag: TypeId, version, deps: Vec<DepKey>, make: BoxFn }
    bundles:   Vec<RuntimeBuilder>,      // merged flat at build()
    allow_cross_tenant: bool,
}
```

`build()` algorithm:
1. Flatten bundles into one factory list (duplicate `(tag,version)` → `DuplicateService`).
2. Build graph `adj: HashMap<TypeId, Vec<TypeId>>` + `in_degree`. Missing dep → `DependencyNotFound`.
3. **Kahn**: queue zero-in-degree nodes; pop, push to `order`, decrement neighbors. If `order.len() != node_count` → `DependencyCycle { remaining }` (names the unresolved nodes).
4. Construct raw implementations in `order` via factories, register each raw `Arc<dyn Trait>` impl into the registry (NOT the proxy), drive `LifecycleManaged::initialize()` on managed components only. Proxies are created lazily at `Runtime::resolve()` time.
5. Return `Runtime`.

```rust
pub struct Runtime { inner: Arc<RuntimeInner> }
struct RuntimeInner { registry: ServiceRegistry, allow_cross_tenant: bool, /* entity/projection/adapter maps */ }
impl Runtime {
    // resolve() fetches the raw impl from registry, wraps it in a {TraitName}Ref proxy on the spot.
    // The proxy holds Weak<RuntimeInner> for mandatory enforce_tenant() on every call.
    pub fn resolve<S: ?Sized + 'static>(&self, req: &VersionReq) -> Result<{TraitName}Ref, RuntimeError>;
    pub fn enforce_tenant(&self, ctx: &ServiceContext) -> Result<(), RuntimeError>;
}
```

### Cross-Tenant Enforcement

`enforce_tenant` fires **before impl dispatch**, inside the proxy but reading the runtime as authority (proxy holds `Weak<RuntimeInner>`). Rule: if `!allow_cross_tenant` (runtime-level OR `ctx.allow_cross_tenant`) and the resolved component's owning tenant differs from `ctx.tenant_id`, return `RuntimeError::CrossTenantDenied { expected, actual }`. Because every resolution path obtains the proxy from the registry which carries the `Weak` runtime, direct `registry.resolve()` cannot bypass the check at call time. Opt-in: `RuntimeBuilder::allow_cross_tenant()` sets the runtime flag; per-call override via `ServiceContext::allow_cross_tenant()`.

### Cleanup Tracks

- **LifecycleManaged**: `#[async_trait] pub trait LifecycleManaged: Send + Sync { async fn initialize(&self)->ServiceResult<()> {Ok(())} async fn shutdown(&self)->ServiceResult<()> {Ok(())} }`. `initialize`/`shutdown` deleted from `Service`. Runtime drives it only for entity/projection/adapter components.
- **ServiceErrorTrait**: `pub trait ServiceErrorTrait: Send + Sync { fn code(&self)->&str; fn category(&self)->ErrorCategory; fn message(&self)->String; }` — object-safe; `ServiceError` and any `DomainError` impl it. Interceptor `on_error` takes `&dyn ServiceErrorTrait`.
- **CancellationToken**: `pub cancellation_token: Option<tokio_util::sync::CancellationToken>` added to `ServiceContext`; `tokio-util` added to `Cargo.toml`. `serde` derive removed from `ServiceContext`, `ServiceRegistry`, `RuntimeBuilder`, descriptors.
- **Deletions**: `reference.rs`, `contract/contract.rs`, `service/*`, `operation/*`, `version/*`, duplicated `DomainError` in `category.rs`. Canonical set: `contract/descriptor.rs` + `contract/version.rs`.

## Data Flow

`caller` holds `OrderServiceRef` (from `runtime.resolve`) → calls typed method → reads/creates `ServiceContext` → `runtime.enforce_tenant` → `ctx.scope` re-enters task-local → `chain.on_request` → `inner.create_order` (impl reads `EntityRef`, which itself sees the scoped context) → `on_response`/`on_error` → domain `Result` returns unchanged through the trait.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| `TypeId::of::<dyn Trait>` unstable | Generate a `{Trait}Tag` ZST as the registry key. |
| `async_trait` + interceptor wrapping edge cases | TDD with trybuild/expansion snapshots before runtime wiring. |
| `entity_sdk` (CORE-006) not yet published | Import-only temporary dep on correct crate; never a local `EntityRef`. |
| Removing serde/`ServiceRef`/lifecycle is source-breaking | `cargo test --workspace` gate; fix call sites in-change. |
| Generated/error branches hard to cover (95%) | Behavioral tests exercise cycle/duplicate/cross-tenant/error paths explicitly. |
