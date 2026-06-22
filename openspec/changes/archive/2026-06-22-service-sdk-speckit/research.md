# Research: Service SDK

**Feature**: 008-service-sdk  
**Date**: 2026-06-08

## 1. Registry Type-Erased Service Storage

**Decision**: Type-map keyed by `(TypeId, Option<String>, Version)` with `Box<dyn Any + Send + Sync>` storage.

**Rationale**: Services are heterogeneous — each implements a different contract trait. The registry must store them generically and resolve them back to the concrete contract type. A `HashMap` keyed by a compound key provides O(1) lookup. `Box<dyn Any>` allows type-erased storage with downcast on resolution. `Arc` wrapping provides shared ownership needed by `ServiceRef<T>` handles.

**Alternatives considered**:
- `anymap` crate: Adds dependency, same semantics as manual HashMap<TypeId, Box<dyn Any>>. Rejected for simplicity.
- Generated enum: Would require the proc-macro to generate a single dispatch enum for all services, coupling compilation units. Rejected — breaks multi-module composition.
- Trait object map (`HashMap<TypeId, Box<dyn ServiceTrait>>`): Requires a common base trait, adding complexity. Services have no common base trait by design (each defines its own contract).

**Key types**:
```rust
type RegistryKey = (TypeId, Option<String>, Version);
type RegistryStore = HashMap<RegistryKey, Arc<dyn Any + Send + Sync>>;
```

## 2. Proc-Macro Design for Attribute-Based Descriptors

**Decision**: Single `#[service]` attribute proc-macro with context-dependent behavior. On traits: generates `ServiceDescriptor` + `OperationDescriptor`. On structs: generates `DependencyMetadata` + `DependencyGraph` + `Injectable` wiring metadata. `#[operation]` attribute on trait methods marks individual operations.

**Rationale**: 
- Single attribute for both contract and implementation — minimal DX surface, consistent mental model.
- On trait: parses the trait name, version attribute, and `#[operation]` methods to build `ServiceDescriptor` with `OperationDescriptor` entries.
- On struct: scans field types for recognized dependency patterns (`EntityRef<T>`, `ServiceRef<T>`, `Arc<T>`, etc.) and generates dependency manifest.
- This follows the pattern of `async-graphql` (single `#[Object]` for both schema types and resolvers).

**Alternatives considered**:
- Separate `#[service_contract]` / `#[service_impl]` attributes: Rejected — more concepts to learn, less ergonomic, not aligned with code-first philosophy.
- `#[service]` on traits only, manual registration for structs: Rejected — less automation, reduced DX, requires more runtime work.

**Edge cases**: Generic service traits (e.g., `trait Repository<T>`) — initial version supports only monomorphic services. Generic support deferred to future work.

## 3. Interceptor Chain Pattern

**Decision**: Tower-inspired middleware chain but simpler. `Interceptor` trait with three hook points: `on_request` (pre-invocation), `on_response` (post-success), `on_error` (post-error). Interceptors are composed into a chain and applied by `ServiceRef<T>` at invocation time.

**Rationale**: 
- Tower's `Service` trait is oriented toward HTTP request/response and requires implementing both `Service` and `Layer`. Overly complex for CORE-008's needs.
- The three-hook model maps directly to interception needs: tracing start/end, metrics counting, error logging.
- Interceptors receive `&ServiceContext` and `&OperationDescriptor` for context-aware instrumentation.

**Trait design**:
```rust
#[async_trait]
pub trait Interceptor: Send + Sync {
    async fn on_request(&self, ctx: &ServiceContext, op: &OperationDescriptor);
    async fn on_response(&self, ctx: &ServiceContext, op: &OperationDescriptor);
    async fn on_error(&self, ctx: &ServiceContext, op: &OperationDescriptor, error: &dyn DomainError);
}
```

**Chain**: A `InterceptorChain` holds `Vec<Box<dyn Interceptor>>` and calls each sequentially for each hook point. Interceptor failure is logged but never fails the invocation.

**Alternatives considered**:
- Tower `Layer`/`Service` pattern: Too complex; requires boxing and service wrapper types. Rejected.
- Single `intercept(fn)` method: Not granular enough for pre/post/error distinction. Rejected.

## 4. ServiceContext Propagation

**Decision**: `tokio::task::TaskLocal<ServiceContext>`. The runtime sets the context at the invocation boundary; it propagates automatically to spawned tasks via `tokio::spawn`. ServiceRef<T> reads the current context and passes it to the first interceptor and the underlying service.

**Rationale**: 
- Task-local storage is the idiomatic Rust async equivalent of thread-local storage.
- `tokio::task::TaskLocal` provides scoped access: `SERVICE_CONTEXT.scope(ctx, async { ... }).await`.
- Doesn't require modifying every service method signature — services access context via `ServiceContext::current()`.
- Automatically propagates across `tokio::spawn` within the scoped future, enabling correct propagation across async boundaries.

**Alternatives considered**:
- Thread-local (`std::thread::LocalKey`): Doesn't work with async — a single thread may serve multiple tasks interleaved. Rejected.
- `tracing::Span` for context: Tracing spans carry metadata but aren't designed for business context (tenant IDs, deadlines). Rejected as primary mechanism, though tracing integration is a natural interceptor use case.
- Explicit parameter passing: Rejected per Q1 clarification (context belongs to invocation, not service signature).

**Limitation**: `TaskLocal` doesn't automatically propagate across `tokio::spawn` calls from third-party code. This is acceptable — the Service SDK spawns tasks under its own control.

**Cross-crate propagation (EntityRef)**: EntityRef<T> reads ServiceContext from the invocation scope (TaskLocal) transparently. No API changes to `persistent-entity` are needed. No wrapping layer is created. The runtime sets the context scope before any service invocation, and EntityRef picks it up naturally within that scope. This resolves the critical cross-crate coupling question identified in the architecture review.

**Deadline/cancellation**: The runtime checks `deadline` and `cancellation` fields on ServiceContext before each operation. These are cached at scope entry to avoid repeated TaskLocal reads. Expired deadline or cancellation signal produces a framework-level error, not a domain error.

## 5. Dependency Resolution via Generated Metadata

**Decision**: Proc-macro scans service struct fields for recognized dependency types (`EntityRef<T>`, `Arc<T: ReadStore>`, `ServiceRef<T>`, etc.) and generates a `Dependencies` associated type listing each dependency by its `TypeId`. The runtime builder matches registered resources to declared dependencies at startup.

**Rationale**: 
- Field scanning at compile time avoids runtime reflection.
- Generated `Dependencies` type acts as a manifest — the runtime can verify all dependencies are satisfiable before construction.
- TypeId-based matching is deterministic and fast (simple HashMap lookup).
- Supports cross-module dependency resolution: service in module A can depend on service in module B as long as both are registered.

**Dependency types recognized by the framework**:
- `EntityRef<T>` — persistent entity reference (from `persistent-entity`)
- `ServiceRef<T>` — other service reference (from `ego-service-sdk`)
- `Arc<T: ReadStore>` — read-side handler
- `Arc<T>` — generic external adapter (any trait object)
- `Configuration` — resolved configuration values

**Alternatives considered**:
- Runtime reflection via `Any`: Requires services to implement trait methods listing their deps. Rejected — more verbose for developers.
- Constructor functions with explicit parameter lists: Rejected per Q2 clarification (runtime resolution, not manual wiring).
- Spring-style XML/annotation DI: Anti-pattern in Rust. Rejected.

## 6. Version Resolution Strategy

**Decision**: Compound resolution key `(TypeId, Option<String>, Option<VersionConstraint>)`. Resolution algorithm: filter by TypeId, filter by optional name, select highest version satisfying constraint. If no version constraint, return latest. If exact version specified and missing, return error.

**Rationale**: 
- `TypeId` provides compile-time contract identity.
- Optional name enables multiple implementations of the same contract (e.g., `EmailNotifier` vs `SmsNotifier` under `NotificationService`).
- Semantic versioning with constraint matching supports both "give me latest" (default) and "give me exactly 1.2.0" (explicit).
- Version is a semver string (`MAJOR.MINOR.PATCH`), not a framework-enforced constraint (per Q3 clarification).

**Resolution algorithm**:
```text
1. Collect all entries matching (TypeId, name) where name is None matches all names
2. If version_constraint is None: return entry with highest semver version
3. If version_constraint is Some(exact): return entry with exact match or error
4. If version_constraint is Some(range): return highest version in range
```

Initial implementation supports `None` and exact match. Range constraints deferred.

## 7. Crate Architecture and Dependency Direction

**Decision**: Two new crates with strict dependency direction:
- `ego-service-sdk` (crates/service-sdk/) — the SDK itself, depends on `ego-domain` and `persistent-entity`
- `ego-service-sdk-macros` (crates/service-sdk-macros/) — proc-macro crate, depends on `syn`, `quote`, `proc-macro2`

**Dependency graph**:
```
ego-service-sdk-macros ──► (standalone, code-generation only)

ego-domain ◄── ego-service-sdk ──► persistent-entity
      ▲              ▲
      │              │
ego-runtime ─────────┘  (runtime integrates service SDK)
ego-application         (future: application may depend on service SDK)
```

**Rationale**: 
- Separates code-generation (proc-macro) from runtime code for faster compilation.
- `ego-service-sdk` depends on `ego-domain` for base types (ServiceContext fields, DomainError trait) and `persistent-entity` for `EntityRef<T>`.
- `ego-runtime` optionally integrates the service SDK via the builder pattern extension.
- No reverse dependencies — domain does NOT depend on services; services depend on domain.

## 8. Testing Strategy

**Decision**: All components testable with mock dependencies. `mockall` crate for auto-mocking service traits. In-memory registry for integration tests.

**Key testing patterns**:
- **Service contract tests**: Define a mock service, verify descriptor generation.
- **Registry tests**: Register mock services, resolve, verify version filtering, detect duplicates.
- **Interceptor tests**: Register interceptors, invoke a mock service, verify hooks were called.
- **Context tests**: Set ServiceContext, invoke service, verify context propagation.
- **Tenant isolation tests**: Set tenant context, attempt cross-tenant access, verify rejection.
- **Deadline tests**: Set deadline, verify runtime terminates invocation on expiry.

All tests use `#[tokio::test]` with mock dependencies only — no real entities, databases, or external services per constitution §8.

## Summary of Key Decisions

| Topic | Decision | Rationale |
|-------|----------|-----------|
| Registry storage | HashMap<(TypeId, Name, Version), Arc<dyn Any>> | Type-safe, O(1) lookup, supports multi-module |
| Metadata generation | #[service] proc-macro on trait | Code-first, developer-friendly |
| Interceptor chain | Tower-inspired 3-hook chain | Simple, sufficient for CORE-008 |
| Context propagation | tokio::task::TaskLocal | Async-safe, transparent to services |
| Dependency resolution | Compile-time field scanning → TypeId manifest | No runtime reflection, fast startup |
| Version resolution | Semver string, latest-by-default, exact optional | Simple, extensible |
| Crate layout | ego-service-sdk + ego-service-sdk-macros | Separate compilation, clean dependency graph |
| Testing | mockall + in-memory registry | Constitution-compliant, deterministic |
