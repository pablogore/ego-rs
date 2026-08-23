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

For an operation marked `#[tenant_scoped]`, tenant enforcement MUST be called as the fallible
`rt.enforce_tenant(&mut ctx)?` (CORE-008A AD-009) — a `mut` binding is required because
`enforce_tenant` is the sole writer of the context's resolver-derived canonical tenant on
success. This call MUST be placed before the inner operation call, so the operation body is
never entered when enforcement fails (FR-009). An operation with no `#[tenant_scoped]` marker
keeps the pre-existing best-effort call, whose `Result` is discarded (D1's valid tenant-less
system/single-tenant execution mode). Interceptor hooks MUST receive the context explicitly:
`interceptor.on_request(&ctx)`, `interceptor.on_response(&ctx)`, `interceptor.on_error(&ctx)`.
No ambient read (`current()`) or scope wrap (`scope()`) is permitted inside the generated body.

#### Scenario: Generated proxy compiles with explicit ctx parameter

- GIVEN a service trait annotated with the proxy derive macro
- WHEN the macro expands the forwarding method
- THEN the generated method accepts `ctx: ServiceContext` as the first user-visible parameter
- AND, for a `#[tenant_scoped]` method, the body calls `rt.enforce_tenant(&mut ctx)?` before
  forwarding the request

#### Scenario: Interceptors receive context from parameter, not ambient state

- GIVEN a proxy dispatch flow with one or more interceptors registered
- WHEN a service method is called with an explicit `ServiceContext`
- THEN `on_request`, `on_response`, and `on_error` hooks each receive `&ctx` sourced from the
  parameter — no call to `ServiceContext::current()` occurs inside the generated body

#### Scenario: Tenant enforcement behavior preserved

- GIVEN a `#[tenant_scoped]` operation and a `ServiceContext` whose authenticated `Principal`
  has `tenant_id = "tenant-a"`
- WHEN a proxy-generated method is called with that context
- THEN `enforce_tenant` derives the canonical tenant from the `Principal`, exposed via
  `ctx.canonical_tenant()`, before the operation body runs
- AND a context whose caller-supplied tenant hint disagrees with the Principal's tenant fails
  the call with `SecurityError::TenantMismatch` before the operation body is entered — the
  fallible check can actually prevent execution, not merely log or ignore the disagreement

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

### Requirement: RuntimeInner Not Publicly Constructible

`RuntimeInner::new()` MUST be `pub(crate)`. Any `Default` implementation for `RuntimeInner` MUST be either removed or `pub(crate)` — it MUST NOT be `pub`. No public constructor for `RuntimeInner` may exist outside the `service-sdk` crate.

The only construction path reachable from outside `crates/service-sdk` is `RuntimeBuilder::build()` (via `RuntimeInner::new_with_logger`, already `pub(super)`).

#### Scenario: External crate cannot construct RuntimeInner directly

- GIVEN a crate outside `service-sdk` (e.g. an application or integration test crate depending on `service-sdk` as a library)
- WHEN that crate attempts to call `RuntimeInner::new(...)` or `RuntimeInner::default()`
- THEN compilation fails with a visibility error

#### Scenario: RuntimeBuilder::build() remains the sole construction path

- GIVEN the `service-sdk` crate after this change
- WHEN `rg "RuntimeInner\s*\{|RuntimeInner::new\(|RuntimeInner::default\(\)" crates/` is run
- THEN every match resolves to `RuntimeBuilder::build()`'s internal call chain (`new_with_logger`) or a `#[cfg(test)]` / `pub(crate)` test helper inside `service-sdk`
- AND no match originates from a crate other than `service-sdk`

#### Scenario: In-crate test helper stays crate-private

- GIVEN a test inside `crates/service-sdk` needs a `RuntimeInner` state not reachable through `RuntimeBuilder::build()`
- WHEN such a helper is added
- THEN it is gated `#[cfg(test)]` and/or `pub(crate)`
- AND it is never re-exposed as `pub`

---

### Requirement: RuntimeBuilder::build() Behavior Is Unchanged

Restricting `RuntimeInner`'s constructors MUST NOT alter the observable behavior of `RuntimeBuilder::build()` for correctly-built runtimes: logger wiring, ordered teardown registration, and security-provider installation behave identically before and after this change.

#### Scenario: Logger wiring unchanged

- GIVEN a `RuntimeBuilder` configured with `.with_logger(logger)`
- WHEN `.build()` is called
- THEN the resulting `Runtime`'s `RuntimeInner::logger()` returns the same logger instance as before this change

#### Scenario: Teardown ordering unchanged

- GIVEN a `RuntimeBuilder` with infrastructure registered that pushes teardown entries
- WHEN `.build()` is called and the runtime is later shut down
- THEN teardown entries drain in the same reverse-construction order as before this change

#### Scenario: Security provider installation unchanged

- GIVEN a `RuntimeBuilder` configured with `.with_security(authn, authz)`
- WHEN `.build()` is called
- THEN `RuntimeInner::authorization_provider()` returns the same provider as before this change

#### Scenario: Build without security still succeeds

- GIVEN a `RuntimeBuilder` with no `.with_security(...)` call
- WHEN `.build()` is called
- THEN a valid `Runtime` is returned with `security_providers == None`, identical to pre-change behavior

---

### Requirement: Reference Host Example Materializes Configuration Through kit-config

The reference host example (`examples/reference-app`) MUST materialize
application configuration through `kit-config` at its composition root,
before any `RuntimeBuilder` construction begins. It MUST hand `RuntimeBuilder`
only materialized configuration, delivered through `ConfigurationProvider` —
never a raw configuration source (unparsed file, raw environment map, or
config-loading intermediate).

This confirms, with a real example, the frozen constraint already established
in `openspec/changes/archive/2026-07-03-CORE-016-app-config-model/spec.md:148`.
It does not redefine that constraint.

#### Scenario: build_runtime wires real kit-config output

- GIVEN `examples/reference-app` depends on `kit-config` as a git dependency
- WHEN `build_runtime()` executes
- THEN configuration is materialized via `kit-config`, delivered to
  `RuntimeBuilder` through `ConfigurationProvider`, and a logger derived from
  it is installed via `.with_logger(...)`

#### Scenario: No raw configuration source reaches RuntimeBuilder

- GIVEN the reference-app composition root after this change
- WHEN every value passed into `RuntimeBuilder`'s builder methods is reviewed
- THEN none of them is an unparsed config source — only materialized
  configuration delivered via `ConfigurationProvider` reaches it

#### Scenario: Existing framework contract remains untouched

- GIVEN `crates/service-sdk`'s `ConfigurationProvider`, `build_logger`, and
  `RuntimeBuilder` implementations
- WHEN this change is applied
- THEN `crates/service-sdk` and `crates/service-sdk/examples/logging_bootstrap.rs`
  show zero diff

---

### Requirement: Canonical Service Registration

`RuntimeBuilder` MUST provide `with_service::<Tag>(self, svc: Arc<Tag::Service>) -> Result<Self, RegistryError>` where `Tag: Resolvable`. The service version MUST be derived from `<Tag as ServiceContract>::version()` — `with_service` MUST NOT accept a caller-supplied version parameter. Registering the same `(Tag, version)` twice MUST return `Err(RegistryError::DuplicateService)` and MUST NOT silently overwrite the prior registration or panic.

#### Scenario: First registration for a tag succeeds
- GIVEN a fresh `RuntimeBuilder` and `Arc<dyn HelloService>`
- WHEN `.with_service::<HelloServiceTag>(inner)` is called
- THEN `Ok(builder)` is returned and the service is recorded under `(HelloServiceTag, HelloServiceTag::version())`

#### Scenario: Duplicate registration is rejected, not silently replaced
- GIVEN a `RuntimeBuilder` that already registered `HelloServiceTag` at its current version
- WHEN `.with_service::<HelloServiceTag>(another_inner)` is called again
- THEN `Err(RegistryError::DuplicateService)` is returned and the originally registered instance remains the one resolvable later

#### Scenario: Registration and resolution can never disagree on version
- GIVEN `with_service::<Tag>` derives its version exclusively from `<Tag as ServiceContract>::version()`, and `resolve::<Tag>()` queries the registry with that exact same `Tag::version()` call
- WHEN a caller registers and later resolves the same `Tag`
- THEN there is no code path through this API where the version used to register differs from the version used to resolve — a caller cannot supply a mismatched version, because neither `with_service` nor `resolve` accepts one; version mismatch is only reachable through the lower-level `ServiceRegistry` API this wrapper does not expose

### Requirement: Canonical Service Resolution Yields the Concrete Generated Proxy

`Runtime` MUST provide `resolve::<Tag>(&self) -> Result<Tag::Proxy, RuntimeError>` where `Tag: Resolvable`. The returned value MUST be the concrete macro-generated `{Trait}Ref` — never a trait object, and callers MUST NOT need to downcast it. Resolving a tag with no registration for it MUST return `Err(RuntimeError::ServiceNotFound)`. The `{Trait}Ref` produced by `resolve` MUST be identical in construction and guard behavior to the one produced by the hand-rolled `{Trait}Ref::new(inner, chain, weak)` path — same interceptor chain, same weak runtime handle, same generated `create_proxy` body — so the operation guard order (authorize → `enforce_tenant` → interceptor chain → operation body), as fixed by the existing "Explicit Context in Proxy Dispatch" requirement and CORE-015's "Marker Execution Order Is Fixed" requirement (AC-8.2), and the tenant-enforcement invariants INV-003 and FR-002/FR-009 in this spec, apply unchanged and are not bypassable through `resolve`.

#### Scenario: Registered tag resolves to a fully-guarded, invokable proxy
- GIVEN `RuntimeBuilder::new().with_service::<HelloServiceTag>(Arc::new(HelloServiceImpl))?.build()`
- WHEN `rt.resolve::<HelloServiceTag>()` is called
- THEN `Ok(HelloServiceRef)` is returned, and calling `.greet(ServiceContext::new(), "world".into())` on it succeeds exactly as the hand-rolled `HelloServiceRef::new(inner, chain, weak)` path would

#### Scenario: Unregistered tag resolves to a named error, not a panic or trait object
- GIVEN a `Runtime` built with no registration for `OtherServiceTag`
- WHEN `rt.resolve::<OtherServiceTag>()` is called
- THEN `Err(RuntimeError::ServiceNotFound)` is returned

#### Scenario: A tenant-scoped operation resolved via `resolve` still fails closed
- GIVEN a `#[tenant_scoped]` service registered via `with_service` and resolved via `resolve::<Tag>()`, invoked with a `ServiceContext` for which the canonical tenant cannot be resolved
- WHEN the resolved proxy's operation is called
- THEN the call fails with the same `SecurityError` INV-003 and FR-001 require from the hand-rolled path, and the operation body is never entered — resolution introduces no alternate, relaxed code path

### Requirement: Fail-Fast Dependency Validation at `try_build()`

`RuntimeBuilder` MUST provide `with_injectable::<S: Injectable>(self) -> Self` to record a service whose dependencies must be present, and `try_build(self) -> Result<Runtime, RuntimeError>` as a new, separate terminal alongside the existing `build()`. `try_build()` MUST fail with `Err(RuntimeError::DependencyNotFound { .. })` if any adapter or config recorded via `with_injectable` is missing from the builder, and MUST succeed and return an equivalent `Runtime` to `build()` when every recorded dependency is present. This requirement governs only `try_build()` and the new `with_injectable` bookkeeping; it does not alter, restrict, or supersede the existing "RuntimeBuilder::build() Behavior Is Unchanged" requirement — `build()` remains infallible and behaviorally identical for every scenario that requirement already covers, whether or not `with_injectable` was called.

#### Scenario: Missing adapter is caught at try_build(), not at first invocation
- GIVEN `RuntimeBuilder::new().with_injectable::<MyService>()` where `MyService` depends on an adapter that was never registered via `.with_adapter(..)`
- WHEN `.try_build()` is called
- THEN `Err(RuntimeError::DependencyNotFound { .. })` is returned, and no `Runtime` is produced

#### Scenario: All dependencies present succeeds identically to build()
- GIVEN `RuntimeBuilder::new().with_adapter(Arc::new(adapter)).with_config(Arc::new(cfg)).with_injectable::<MyService>()`
- WHEN `.try_build()` is called
- THEN `Ok(rt)` is returned, and `MyService::build(rt.inner())` succeeds using the same resolved adapter/config `build()` would have provided

#### Scenario: build() remains infallible and untouched by with_injectable
- GIVEN a `RuntimeBuilder` with `.with_injectable::<MyService>()` recorded but a required adapter missing
- WHEN `.build()` (not `.try_build()`) is called
- THEN a `Runtime` is returned with no error — `with_injectable` bookkeeping has no effect on `build()`, matching the existing "RuntimeBuilder::build() Behavior Is Unchanged" requirement

#### Scenario: Multiple missing dependencies report only the first, in registration order
- GIVEN `RuntimeBuilder::new().with_injectable::<ServiceA>().with_injectable::<ServiceB>()` where both `ServiceA` and `ServiceB` depend on adapters that were never registered
- WHEN `.try_build()` is called
- THEN `Err(RuntimeError::DependencyNotFound { .. })` is returned naming `ServiceA`'s missing dependency only — validators run in the exact order `with_injectable` was called, not an unordered or hash-based order, and reporting every missing dependency at once is explicitly out of scope for this requirement (deferred)

### Requirement: Diagnosable Dependency Error

`RuntimeError::DependencyNotFound` MUST carry `{ type_name: &'static str, service_name: Option<&'static str> }`, MUST implement `std::fmt::Display` naming both the missing type and, when known, the requesting service, and MUST implement `std::error::Error`.

#### Scenario: Error names the missing type and the requesting service
- GIVEN `try_build()` fails because `MyService` (registered via `with_injectable`) needs an adapter that was never provided
- WHEN the returned `Err(RuntimeError::DependencyNotFound { type_name, service_name })` is formatted with `Display`
- THEN the formatted string names both the missing adapter's type and `MyService` as the requesting service

#### Scenario: DependencyNotFound is a real std::error::Error
- GIVEN a `RuntimeError::DependencyNotFound { .. }` value
- WHEN it is used as `&dyn std::error::Error` (e.g. boxed or propagated via `?` into a `Box<dyn Error>`)
- THEN it compiles and behaves as a standard error, not merely a `Debug`-only value

### Requirement: `{Trait}Ref::new` Escape Hatch Remains Supported

This change MUST NOT remove, deprecate, or `#[doc(hidden)]` the existing macro-generated `{Trait}Ref::new(inner, chain, weak)` constructor. It MUST remain callable and produce a proxy behaviorally identical to one obtained via `resolve`.

#### Scenario: Hand-rolled construction still compiles and runs after this change
- GIVEN the generated `{Trait}Ref` for a service defined before and after this change
- WHEN `{Trait}Ref::new(inner, chain, weak)` is called directly, as in pre-existing tests
- THEN it compiles without a deprecation warning and the resulting proxy behaves identically to before this change

---

### Requirement: Projection Registration Completes The Resolution Contract

`RuntimeBuilder` MUST provide a public method to register a projection instance, making it resolvable via `RuntimeInner::resolve_projection::<P>()` on the built runtime — the same resolution path a service's `Injectable::build` already uses to obtain a `ProjectionRef<P>`. Before this method exists, a service declaring a projection dependency has no production path to satisfy it; after it exists, that dependency is satisfiable exactly like an adapter or config dependency is today.

#### Scenario: A registered projection is resolvable by a dependent service
- GIVEN a projection instance registered on `RuntimeBuilder` for a given type
- WHEN a service declaring a dependency on that projection type is constructed against the built runtime
- THEN it receives that projection instance as `ProjectionRef<P>`

#### Scenario: Resolving an unregistered projection type fails closed, naming the type
- GIVEN a `RuntimeBuilder` with no registration for a given projection type
- WHEN `resolve_projection::<P>()` is called against the built runtime, or a service declaring that dependency is validated or constructed
- THEN the call fails with the existing `DependencyNotFound` error naming that projection type — no panic, and no silently-empty or default projection is fabricated

### Requirement: Duplicate Projection Registration Fails Closed

Registering a second projection instance for a type that was already registered MUST be rejected at build, mirroring the fail-closed contract `RuntimeBuilder::with_service` already applies to a duplicate service registration (`RegistryError::DuplicateService`) — never a silent last-write-wins replacement. This is a deliberate departure from `with_adapter`'s and `with_config`'s existing last-write-wins semantics: those two registration methods are unchanged by this delta, and projection registration does not adopt their replace-on-conflict behavior.

#### Scenario: First registration for a projection type succeeds
- GIVEN a fresh `RuntimeBuilder`
- WHEN a projection instance is registered for a type with no prior registration
- THEN the registration succeeds and the instance is later resolvable

#### Scenario: A second registration of the same projection type is rejected, not silently replaced
- GIVEN a `RuntimeBuilder` that already has a registered projection instance for a given type
- WHEN a second projection instance of the same type is registered
- THEN the registration fails, the originally registered instance remains the one resolvable afterward, and no silent overwrite occurs

### Requirement: A Declared Projection Dependency Is Satisfiable At Build

A service that declares a projection dependency through `Injectable::dependencies()` MUST build and resolve it when the projection was registered, and MUST fail before startup — naming the missing projection type — when it wasn't, using the same `try_build()` / `DependencyNotFound` attribution path already used for adapter and config dependencies.

#### Scenario: try_build succeeds when the declared projection dependency is registered
- GIVEN a service declaring a projection dependency, recorded via `with_injectable`, whose projection type was registered on the same `RuntimeBuilder`
- WHEN `try_build()` is called
- THEN it succeeds, and the service's declared projection dependency resolves during construction

#### Scenario: try_build fails before startup when the declared projection dependency is missing
- GIVEN a service declaring a projection dependency, recorded via `with_injectable`, whose projection type was never registered
- WHEN `try_build()` is called
- THEN it fails with `DependencyNotFound` naming both the missing projection type and the requesting service, and no `Runtime` is produced

### Requirement: App Exposes A Projection Resolution Accessor

`App` MUST provide a read-only `resolve_projection::<P>()` accessor, symmetric with the existing `App::resolve_adapter()`/`App::resolve_config()` accessors, so a caller holding a built `App` (not just a service resolved through it) can resolve a registered projection.

#### Scenario: A built App resolves a registered projection through the accessor
- GIVEN an `App` built with a projection registered via `AppBuilder::projection(...)`
- WHEN `App::resolve_projection::<P>()` is called for that projection's type
- THEN it returns the registered instance as `ProjectionRef<P>`

### Requirement: Entity Runtime Registration Completes The Resolution Contract

`RuntimeBuilder` MUST provide a public method to register a host-constructed entity runtime for a given aggregate/entity type `E`, making it resolvable as `EntityRuntimeRef<E>` — the same resolution shape `ProjectionRef<P>` already provides for projections. `EntityRuntimeRef<E>` is a handle capable of dispatching to any entity instance of type `E`; it is distinct from, and does not replace, `persistent-entity`'s existing per-request handle to one specific entity instance, which is obtained separately and unchanged by this capability. Before this method exists, a service declaring an entity dependency has no production path to satisfy it; after it exists, that dependency is satisfiable exactly like an adapter, config, or projection dependency is today.

#### Scenario: A registered entity runtime is resolvable by a dependent service
- GIVEN a host-constructed entity runtime registered on `RuntimeBuilder` for aggregate type `E`
- WHEN a service declaring a dependency on that aggregate type is constructed against the built runtime
- THEN it receives that entity runtime as `EntityRuntimeRef<E>`

#### Scenario: Resolving an unregistered entity type fails closed, naming the aggregate type
- GIVEN a `RuntimeBuilder` with no registration for a given aggregate type
- WHEN a service declaring that dependency is validated or constructed against the built runtime
- THEN the call fails with the existing `DependencyNotFound` error naming that aggregate type — no panic, and no default or empty entity runtime is fabricated

#### Scenario: A resolved entity runtime handle is distinct from a per-request entity handle
- GIVEN a service holding an `EntityRuntimeRef<E>` resolved through this registration path
- WHEN the service dispatches to one specific entity instance of type `E`
- THEN it does so through `persistent-entity`'s existing per-request handle, obtained from the entity runtime, unchanged by this capability — the composition-time handle and the per-request handle remain two distinct, coexisting concepts

### Requirement: Entity Identity Is Keyed By The Aggregate Type, Not Its Event Type

Entity runtime registration, resolution, and duplicate detection MUST all be keyed by the aggregate/entity type `E` a service declares as its dependency — never by `E`'s associated event type. Two distinct aggregate types that share the same associated event type MUST register and resolve independently, with no collision between them.

#### Scenario: A missing entity dependency names the aggregate type, not its event type
- GIVEN a service declaring a dependency on aggregate type `OrderEntity`, whose entity runtime was never registered
- WHEN construction is attempted
- THEN the resulting `DependencyNotFound` error names `OrderEntity` — never `OrderEntity`'s associated event type

#### Scenario: Two aggregates sharing an event type register and resolve without collision
- GIVEN two distinct aggregate types that share the same associated event type, each with its own entity runtime registered
- WHEN a service declares a dependency on one of the two aggregate types
- THEN it resolves that aggregate's own registered entity runtime, unaffected by the other aggregate's registration despite the shared event type

### Requirement: Duplicate Entity Registration Fails Closed

Registering a second entity runtime for an aggregate type that was already registered MUST be rejected at build, mirroring the fail-closed contract `RuntimeBuilder::with_service` and projection registration already apply to a duplicate registration — never a silent last-write-wins replacement.

#### Scenario: First registration for an aggregate type succeeds
- GIVEN a fresh `RuntimeBuilder`
- WHEN an entity runtime is registered for an aggregate type with no prior registration
- THEN the registration succeeds and the runtime is later resolvable

#### Scenario: A second registration of the same aggregate type is rejected, not silently replaced
- GIVEN a `RuntimeBuilder` that already has a registered entity runtime for a given aggregate type
- WHEN a second entity runtime for the same aggregate type is registered
- THEN the registration fails, the originally registered entity runtime remains the one resolvable afterward, and no silent overwrite occurs

### Requirement: A Declared Entity Dependency Is Satisfiable At Build

A service that declares an entity dependency through `Injectable::dependencies()` MUST build and resolve it when the matching entity runtime was registered, and MUST fail before startup — naming the missing aggregate type — when it wasn't, using the same `try_build()` / `DependencyNotFound` attribution path already used for adapter, config, and projection dependencies.

#### Scenario: try_build succeeds when the declared entity dependency is registered
- GIVEN a service declaring an entity dependency, recorded via `with_injectable`, whose aggregate type's entity runtime was registered on the same `RuntimeBuilder`
- WHEN `try_build()` is called
- THEN it succeeds, and the service's declared entity dependency resolves during construction

#### Scenario: try_build fails before startup when the declared entity dependency is missing
- GIVEN a service declaring an entity dependency, recorded via `with_injectable`, whose aggregate type's entity runtime was never registered
- WHEN `try_build()` is called
- THEN it fails with `DependencyNotFound` naming both the missing aggregate type and the requesting service, and no `Runtime` is produced

### Requirement: App Exposes An Entity Resolution Accessor

`App` MUST provide a read-only `resolve_entity::<E>()` accessor, symmetric with the existing `App::resolve_adapter()`/`App::resolve_config()`/`App::resolve_projection()` accessors, so a caller holding a built `App` (not just a service resolved through it) can resolve a registered entity runtime.

#### Scenario: A built App resolves a registered entity runtime through the accessor
- GIVEN an `App` built with an entity runtime registered via `AppBuilder::entity::<E>(...)`
- WHEN `App::resolve_entity::<E>()` is called for that aggregate type
- THEN it returns the registered runtime as `EntityRuntimeRef<E>`

---

## Tenant Enforcement & Cross-Tenant Access (CORE-008A)

This section describes the canonical tenant model, resolution authority, fail-closed
enforcement, and authorization-gated cross-tenant access built by CORE-008A and
subsequently closed out (FR-006 consumption gap) by later work. It supersedes the
narrower, now-stale "TenantResolver does not re-validate..." section previously here,
which covered only one delta (CORE-024) against an already-obsolete `resolve()`
signature.

**Resolution seam.** `TenantResolver::resolve` (`crates/service-sdk/src/runtime/tenant.rs`)
is the single algorithm mandated below. It takes one argument, a closed
`EstablishedTenantFacts<'a>` value:

```rust
pub(crate) struct EstablishedTenantFacts<'a> {
    security: Option<&'a SecurityContext>,
    hint: Option<&'a str>,
    cross_tenant_grant: Option<&'a CrossTenantGrant>,
}

impl<'a> EstablishedTenantFacts<'a> {
    pub(crate) fn new(
        security: Option<&'a SecurityContext>,
        hint: Option<&'a str>,
        cross_tenant_grant: Option<&'a CrossTenantGrant>,
    ) -> Self;
}

impl TenantResolver {
    pub(crate) fn resolve(
        &self,
        facts: EstablishedTenantFacts<'_>,
    ) -> Result<CanonicalTenant, SecurityError>;
}
```

`RuntimeInner::enforce_tenant` gathers `facts` from `ServiceContext` (`ctx.security()`,
`ctx.tenant_hint()`, `ctx.cross_tenant_grant()`) and calls `resolve` once per
tenant-scoped operation. **AD-014 (Fact Establishment vs. Policy Evaluation)** governs
this seam: `TenantResolver::resolve` is a Policy Evaluator — it derives its decision
exclusively from the closed, immutable `facts` it was handed, and never itself fetches,
queries, or authorizes anything during evaluation. Establishing a cross-tenant grant is
a separate, upstream Fact Establishment step (`RuntimeInner::issue_cross_tenant_permit`
+ `ServiceContext::with_cross_tenant_access`) that must complete before `resolve` ever
runs — see FR-006 below.

---

### Requirement: Tenant-Scoped Fail-Closed Enforcement Is Operation-Level, Not Global (FR-001)

Tenant-scoped operations MUST fail closed when the canonical tenant cannot be resolved
and validated for that operation. A valid tenant-less system/single-tenant execution
mode MUST remain available; fail-closed enforcement applies only to operations
classified as tenant-scoped, not to every operation in the runtime. Classification is
the `#[tenant_scoped]` macro attribute (see "Explicit Context in Proxy Dispatch" above,
which resolves the mechanism this requirement's archived form left open) — unmarked
operations never call `enforce_tenant` at all.

#### Scenario: Tenant-scoped operation fails closed without resolvable tenant

- GIVEN an operation annotated `#[tenant_scoped]`
- WHEN it is invoked and `RuntimeInner::enforce_tenant` cannot resolve a canonical
  tenant for the call
- THEN the call fails with an explicit `SecurityError` and the operation is not executed

#### Scenario: Non-tenant-scoped operation is unaffected by missing tenant

- GIVEN an operation with no `#[tenant_scoped]` marker, running in a valid
  system/single-tenant execution mode
- WHEN it is invoked with no tenant present
- THEN the call proceeds and executes normally; no tenant error occurs

---

### Requirement: Principal Is the Canonical Tenant Authority on the Authenticated Path (FR-002)

When a request is authenticated (a `Principal` exists via JWT/API key/OIDC),
`Principal.tenant_id` (`Option<TenantId>`, already validated at `Principal`
construction) MUST be treated as canonical. `TenantResolver::resolve` MUST derive the
tenant visible to the service operation from `Principal.tenant_id` automatically — it
MUST NOT re-validate that value via `TenantId::new()` or any equivalent; it is cloned
directly into the returned `CanonicalTenant`. If a caller-supplied hint
(`facts.hint`) is present, non-blank after trimming, and disagrees with
`Principal.tenant_id`, the call MUST fail with `SecurityError::TenantMismatch` — the
resolver MUST NOT silently prefer either value (unless FR-006's cross-tenant grant
covers exactly that hint's destination — see below). A blank or whitespace-only hint is
treated as absent, not as a mismatch. If the authenticated Principal carries no tenant
claim at all (`Principal.tenant_id` is `None`), the resolver MUST NOT treat any
caller-supplied hint as a substitute for it — the call MUST fail closed with
`SecurityError::MissingContext`, regardless of whether a hint is present or absent, and
this check MUST be evaluated before the hint-agreement check (a present-but-conflicting
hint must never be evaluated against an absent Principal tenant claim).

#### Scenario: Derivation from Principal succeeds without manual tenant assignment

- GIVEN a `SecurityContext` wrapping a `Principal` with `tenant_id = Some(TenantId::new("tenant-a").unwrap())` and no conflicting hint
- WHEN `resolver.resolve(EstablishedTenantFacts::new(Some(&security), None, None))` is called
- THEN `Ok(CanonicalTenant::scoped(tenant))` is returned where `tenant` is a clone of the Principal's `TenantId` — no call to `TenantId::new()` occurs during this resolution

#### Scenario: Caller-supplied tenant conflicting with Principal is a hard error

- GIVEN a `SecurityContext` wrapping a `Principal` with `tenant_id = Some(TenantId::new("tenant-a").unwrap())`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(Some(&security), Some("tenant-b"), None))` is called
- THEN `Err(SecurityError::TenantMismatch { expected: "tenant-a", actual: "tenant-b" })` is returned; neither value is silently chosen

#### Scenario: Blank hint is treated as absent, not a mismatch

- GIVEN a `SecurityContext` wrapping a `Principal` with `tenant_id = Some(TenantId::new("tenant-a").unwrap())`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(Some(&security), Some(""), None))` is called
- THEN `Ok(CanonicalTenant::scoped(tenant))` is returned for `"tenant-a"` — a blank hint never triggers `TenantMismatch`

#### Scenario: Authenticated Principal without a tenant claim fails closed regardless of a caller-supplied hint

- GIVEN a `SecurityContext` wrapping a `Principal` with `tenant_id = None`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(Some(&security), Some("tenant-x"), None))` is called
- THEN `Err(SecurityError::MissingContext)` is returned; the hint is never used as a substitute for the missing Principal tenant claim

#### Scenario: No validation call reachable on the Principal-derived path (structural)

- GIVEN the source of `TenantResolver::resolve()`
- WHEN the Principal-derived branch (the `Some(security)` match arm) is inspected
- THEN no call to `TenantId::new(...)` appears on the path that handles `security.principal().tenant_id` — the only operation performed on that value is a clone into `CanonicalTenant::scoped(...)`

**Tests**: `tenant::tests::resolve_authenticated_hint_absent_resolves_to_principal_tenant`, `tenant::tests::resolve_authenticated_hint_agrees_resolves_to_principal_tenant`, `tenant::tests::resolve_authenticated_blank_hint_resolves_to_principal_tenant`, `tenant::tests::resolve_authenticated_hint_disagrees_is_tenant_mismatch`, `tenant::tests::resolve_authenticated_no_principal_tenant_fails_closed_even_with_hint`, `tenant::tests::resolve_authenticated_no_principal_tenant_fails_closed_without_hint`. The "no re-validation" property is verified by code inspection at review time, not a runtime assertion — once `Principal.tenant_id` is `Option<TenantId>`, there is no invalid value a unit test could construct to distinguish "validated once" from "re-validated every call".

#### Out of Scope for This Requirement

- **No change to `ServiceContext.tenant_id` / `tenant_hint()`** (`crates/service-sdk/src/context/mod.rs`). That is a deliberately-raw ingress hint per AD-011, a different concept from the authenticated Principal's tenant claim. `testkit::TestContextBuilder`, which builds this hint, is likewise untouched.
- **`TenantEnforcementMode` variants and the hint-mismatch/agreement decision logic are unchanged** by the CORE-024 validate-once delta — only the source of validation for the Principal-derived value was removed, not the resolution algorithm's branches.

---

### Requirement: Explicit System/Internal Request Mode (FR-003)

An unauthenticated call (no `Principal`, `facts.security == None`) MUST be routed
through a distinct, explicit system/internal branch of `TenantResolver::resolve` rather
than being treated as a variant of FR-002's mismatch case. A caller-supplied hint is
valid in this mode only when the runtime was configured with
`TenantEnforcementMode::AllowSystemInternal` (via
`RuntimeBuilder::with_tenant_enforcement_mode`; the default is `AuthenticatedOnly`).
This is the ONE remaining raw-string parse in `resolve()`: `TenantId::new(hint.trim())`
— the hint is trimmed of leading/trailing whitespace before validation so incidental
whitespace (e.g. from a transport header) does not mint a `TenantId` that silently
fails to `==` a clean one downstream.

#### Scenario: Internal mode accepts caller-supplied tenant when explicitly permitted

- GIVEN `TenantResolver::new(TenantEnforcementMode::AllowSystemInternal)` and no `SecurityContext`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(None, Some("tenant-c"), None))` is called
- THEN `Ok(CanonicalTenant::scoped(tenant))` is returned for `"tenant-c"`, without being treated as a `TenantMismatch`

#### Scenario: Internal-mode hint is trimmed before validation

- GIVEN `TenantResolver::new(TenantEnforcementMode::AllowSystemInternal)` and no `SecurityContext`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(None, Some(" tenant-c "), None))` is called
- THEN `Ok(CanonicalTenant::scoped(tenant))` is returned where `tenant.as_str() == "tenant-c"` — the stored value is trimmed, not the raw untrimmed hint

#### Scenario: Internal mode rejects tenant when not permitted

- GIVEN `TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly)` (the default) and no `SecurityContext`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(None, Some("tenant-c"), None))` is called
- THEN `Err(SecurityError::MissingContext)` is returned — the call does not proceed as an authenticated-tenant call; it is handled per FR-004

**Tests**: `tenant::tests::resolve_unauthenticated_allow_system_internal_with_hint_resolves_to_hint`, `tenant::tests::resolve_unauthenticated_allow_system_internal_trims_whitespace_in_hint`, `tenant::tests::resolve_unauthenticated_authenticated_only_mode_fails_closed`.

---

### Requirement: Neither Authenticated Nor Internal-Permitted Fails Closed (FR-004)

A call that is neither authenticated (no `Principal`) nor covered by a
runtime-permitted system/internal mode MUST fail with `SecurityError::MissingContext`
before a tenant-scoped operation body executes. (The archived spec anticipated a
possible separate `MissingAuthentication` variant; the shipped `SecurityError` enum —
`crates/security-sdk/src/error/mod.rs` — has no such variant. `MissingContext` alone
covers this case, which the archived spec's own wording already permitted: "the three
conditions may surface through `RuntimeError`, `ServiceError`, `SecurityError`, or any
combination design.md chooses.")

#### Scenario: Unauthenticated, non-internal call is rejected

- GIVEN `TenantResolver::new(TenantEnforcementMode::AllowSystemInternal)` and no `SecurityContext`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(None, None, None))` is called
- THEN `Err(SecurityError::MissingContext)` is returned, and the operation body is never entered

**Tests**: `tenant::tests::resolve_unauthenticated_allow_system_internal_without_hint_fails_closed`.

---

### Requirement: CrossTenantPermit Requires Authorized Capability (FR-005)

`CrossTenantPermit` MUST be issued only after `AuthorizationProvider` confirms the
requesting Principal holds an explicit cross-tenant capability. The current mechanism:
`RuntimeInner::issue_cross_tenant_permit(&self, ctx: &ServiceContext, destination: TenantId)`
(`crates/service-sdk/src/runtime/runtime_builder.rs`) builds a `Resource { kind: "tenant", id: Some(destination) }`
/ `Action("cross-tenant-access")` request and calls `authorize_in_context` against the
configured `AuthorizationProvider`. Being authorized for the target resource/action
under a different action name is never checked — only this specific
`"tenant:cross-tenant-access"` capability grants a permit. A `Deny` decision maps to
`SecurityError::CrossTenantDenied`; if no `AuthorizationProvider` is configured, the
call fails with `SecurityError::CapabilityNotEnabled`.

#### Scenario: Permit denied for principal without cross-tenant capability

- GIVEN a Principal whose `AuthorizationProvider` denies the `"tenant:cross-tenant-access"` request
- WHEN `issue_cross_tenant_permit` is called for a destination tenant
- THEN `Err(SecurityError::CrossTenantDenied { .. })` is returned and no `CrossTenantPermit` is issued

#### Scenario: No provider configured fails closed

- GIVEN a runtime with no `AuthorizationProvider` configured
- WHEN `issue_cross_tenant_permit` is called
- THEN `Err(SecurityError::CapabilityNotEnabled)` is returned

**Tests**: `runtime_builder::tests::issue_cross_tenant_permit_denied_without_capability`, `runtime_builder::tests::issue_cross_tenant_permit_denied_even_with_resource_action_alone`, `runtime_builder::tests::issue_cross_tenant_permit_without_provider_is_capability_not_enabled`.

---

### Requirement: Authorized Cross-Tenant Access Succeeds (FR-006)

A Principal holding the `"tenant:cross-tenant-access"` capability, confirmed via
`AuthorizationProvider`, MUST be able to obtain a `CrossTenantPermit` and successfully
execute a cross-tenant operation using it. Per **AD-014**, this is wired as Fact
Establishment feeding Policy Evaluation, not as a callback performed during resolution:

1. `RuntimeInner::issue_cross_tenant_permit` mints a `CrossTenantPermit { destination, issued_to }`
   only on an `Allow` decision (FR-005).
2. `ServiceContext::with_cross_tenant_access(&permit)` attaches it, storing a
   `CrossTenantGrant` (`crates/service-sdk/src/runtime/tenant.rs`) — an AD-014
   Established Fact — scoped to exactly the permit's `destination`. A permit issued for
   `tenant-b` can never authorize a grant for `tenant-c`.
3. `RuntimeInner::enforce_tenant` gathers `EstablishedTenantFacts` (`ctx.security()`,
   `ctx.tenant_hint()`, `ctx.cross_tenant_grant()`) and hands them to
   `TenantResolver::resolve` as a single closed value.
4. Inside `resolve`, ONLY when an authenticated hint disagrees with the Principal's own
   tenant AND a `CrossTenantGrant` is present whose `destination` exactly matches the
   (trimmed) hint, resolution succeeds with `CanonicalTenant::scoped(grant.destination().clone())`
   instead of a hard `TenantMismatch`. `resolve` never fetches, checks, or re-derives
   the grant itself — it only reads the Established Fact it was handed (AD-014). A
   grant scoped to a different destination than the hint still produces
   `TenantMismatch`; an unused grant (hint absent or agreeing) has no effect.

#### Scenario: Authorized cross-tenant access succeeds end to end

- GIVEN a Principal authenticated on `"tenant-a"`, and an `AuthorizationProvider` that allows `"tenant:cross-tenant-access"`
- WHEN the Principal calls `issue_cross_tenant_permit` for `"tenant-b"`, attaches the resulting permit via `ctx.with_cross_tenant_access(&permit).with_tenant_id("tenant-b")`, and `RuntimeInner::enforce_tenant(&mut ctx)` runs
- THEN `enforce_tenant` returns `Ok(())`, and `ctx.canonical_tenant().and_then(CanonicalTenant::tenant_id)` is `Some("tenant-b")` — not rejected as a tenant violation

#### Scenario: Grant scoped to a different destination than the hint still mismatches

- GIVEN a Principal authenticated on `"tenant-a"` holding a grant for `"tenant-c"`
- WHEN `resolver.resolve(EstablishedTenantFacts::new(Some(&security), Some("tenant-b"), Some(&grant)))` is called
- THEN `Err(SecurityError::TenantMismatch { expected: "tenant-a", actual: "tenant-b" })` is returned — the grant does not act as a blanket cross-tenant switch

#### Scenario: Unused grant does not affect an ordinary same-tenant call

- GIVEN a Principal authenticated on `"tenant-a"` holding a grant for `"tenant-b"`, and no hint supplied
- WHEN `resolver.resolve` is called
- THEN resolution succeeds with `"tenant-a"`, as if no grant existed

**Tests**: `runtime_builder::tests::enforce_tenant_succeeds_for_authorized_cross_tenant_grant` (full issued → attached → consumed → operation-succeeds flow), `runtime_builder::tests::issue_cross_tenant_permit_allowed_yields_destination_scoped_permit`, `tenant::tests::resolve_authorized_cross_tenant_grant_succeeds`, `tenant::tests::resolve_authorized_cross_tenant_grant_succeeds_with_whitespace_in_hint`, `tenant::tests::resolve_grant_for_different_destination_is_still_tenant_mismatch`, `tenant::tests::resolve_unused_grant_does_not_affect_hint_absent_resolution`, `tenant::tests::resolve_redundant_grant_matching_own_tenant_resolves_normally`, `context::tests::is_cross_tenant_allowed_for_matches_only_the_issued_destination`.

---

### Requirement: Runtime Is Transport-Independent for Tenant Resolution (FR-007)

`TenantResolver::resolve` and `RuntimeInner::enforce_tenant` MUST consume only
transport-neutral inputs (`EstablishedTenantFacts`: an already-produced
`SecurityContext`, an optional `&str` hint, an optional `&CrossTenantGrant`). Neither
MUST depend on any transport-specific mechanism (HTTP headers, gRPC metadata, or any
other transport concept) to obtain or validate the tenant.

#### Scenario: Runtime enforcement contains no transport-specific dependency

- GIVEN `crates/service-sdk/src/runtime/tenant.rs` and `runtime_builder.rs`'s `enforce_tenant`
- WHEN reviewed for dependencies
- THEN neither references any HTTP, gRPC, or other transport-specific type, or header/metadata extraction logic — only `SecurityContext`, `&str`, and `CrossTenantGrant`

---

### Requirement: Exactly One Canonical In-Runtime Tenant Representation (FR-008)

Exactly one representation of tenant MUST be canonical inside the runtime at the point
an operation executes: `CanonicalTenant` (`crates/service-sdk/src/runtime/tenant.rs`).
It wraps a private `Repr` enum (`Scoped(TenantId)` for a resolved tenant, `Systemwide`
for D1's valid tenant-less mode); its constructors are `pub(super)`, reachable only
within `crate::runtime`, so only `TenantResolver::resolve` may mint one. `Principal.tenant_id`,
`ServiceContext.tenant_id` (the ingress hint), and `ClaimSet::tenant()` are
ingress/legacy carriers only — none is independently authoritative for the
same operation at execution time. `Principal.tenant_id` is the authoritative
*input* on the authenticated path; `TenantResolver`'s output is the
authoritative *runtime* value; `ServiceContext.tenant_id` is demoted to a
non-authoritative ingress hint (read via `ctx.tenant_hint()`).

(Previously: also listed domain `ExecutionContext` among ingress/legacy tenant
carriers. That type is deleted by this change and no longer exists.)

#### Scenario: Divergent ingress values converge to one authoritative value

- GIVEN a request where the Principal's tenant claim and a caller-supplied hint could disagree
- WHEN `RuntimeInner::enforce_tenant` runs
- THEN exactly one `CanonicalTenant` is produced and stored via `ctx.set_resolved_tenant`, and every downstream tenant-aware read (`ctx.canonical_tenant()`) observes that same value

#### Scenario: Only the runtime can construct a CanonicalTenant

- GIVEN code outside `crate::runtime` in `service-sdk`
- WHEN it attempts to construct a `CanonicalTenant` directly (e.g. `CanonicalTenant::scoped(...)`)
- THEN compilation fails with a visibility error — `scoped`/`systemwide` are `pub(super)`

**Tests**: `tenant::tests::canonical_tenant_scoped_is_constructible_within_runtime`, `tenant::tests::canonical_tenant_systemwide_is_constructible_within_runtime`.

---

### Requirement: Tenant Access MUST Match the Pipeline Stage

Tenant access is a convention, not a compiler-enforced restriction: tenant reads MUST
use either `tenant_hint()` or `canonical_tenant()`, and presence checks use
`has_tenant_hint()`, selected by pipeline stage:

| What the code exercises | Correct accessor |
|---|---|
| Context construction | `tenant_hint()` |
| Clone before runtime enforcement | `tenant_hint()` |
| Explicit propagation (task spawn, parameter passing) | `tenant_hint()` |
| Runtime / `TenantResolver` | `canonical_tenant()` |
| Authorization | `canonical_tenant()` |
| Enforcement (`enforce_tenant`, `#[tenant_scoped]`) | `canonical_tenant()` |

`canonical_tenant()` reads `resolved_tenant`, set only by `enforce_tenant()` via
`set_resolved_tenant()`; a `ServiceContext` built directly via `with_tenant_id()` without
running `enforce_tenant()` MUST return `None` from `canonical_tenant()`.

#### Scenario: Deprecated accessors do not exist

- GIVEN the `service-sdk` crate after this change
- WHEN `ServiceContext`'s public API is inspected
- THEN `ServiceContext` exposes no `tenant_id()` or `has_tenant()` methods

#### Scenario: Pre-enforcement code reads the ingress hint

- GIVEN a `ServiceContext` built directly via `with_tenant_id()`, with `enforce_tenant()` not yet called
- WHEN test or propagation code reads the tenant value
- THEN `ctx.tenant_hint()` returns the constructed value and `ctx.canonical_tenant()` returns `None`

#### Scenario: Enforcement-stage code reads the canonical value

- GIVEN a `ServiceContext` after `enforce_tenant()` has run and stored a resolved tenant
- WHEN authorization or `#[tenant_scoped]` logic reads the tenant value
- THEN `ctx.canonical_tenant()` returns the resolved value

**Tests**: `crates/service-sdk/tests/{smoke,context_propagation,context_cross_service,context_explicit_propagation}.rs` reference only `tenant_hint()` and `canonical_tenant()`.

---

### Requirement: Unused Execution-Context Abstractions Are Removed

`ExecutionContext`, `DomainExecutionContext` (`crates/domain/src/context.rs`), and
`RuntimeExecutionContext` (`crates/runtime/src/context.rs`), including their re-exports,
are removed because they have zero production callers and `CommandContext`
(`crates/persistent-entity/src/command_context.rs`) is the sole execution-context
abstraction with production callers. This reflects the evidence gathered for this
change, not a standing prohibition on ever introducing an execution-context abstraction.

#### Scenario: No workspace reference to the removed types remains

- GIVEN the workspace source after this change
- WHEN searched with `rg "ExecutionContext" crates/ --type rust`
- THEN zero matches are found, and `cargo build --workspace` succeeds

---

### Requirement: Workspace Contains No Deprecated Tenant Accessors

This is a distinct acceptance concern from pipeline-stage correctness above: one of
CORE-008B's originating goals was eliminating `#[deprecated]` warnings, not merely
picking the right accessor per call site.

#### Scenario: No deprecated-accessor warnings remain

- GIVEN the workspace after this change
- WHEN `cargo build --workspace` and `cargo test --workspace` run
- THEN neither emits a `#[deprecated]` warning for `tenant_id()` or `has_tenant()`, because neither method exists

#### Scenario: Only the field remains, not the deprecated methods

- GIVEN the workspace source
- WHEN searched with `rg "\.tenant_id\(\)|\.has_tenant\(\)" crates/`
- THEN zero matches are found — the only surviving `tenant_id` symbol is the private `tenant_id: Option<String>` field, readable only via `tenant_hint()`

---

### Requirement: Architecture Documentation Describes the Explicit-Propagation Model Only

`ARCHITECTURE.md` MUST NOT describe `ServiceContext` as TaskLocal-scoped or as
propagating via ambient/task-local state.

#### Scenario: Architecture doc contains no ambient-propagation claim

- GIVEN `ARCHITECTURE.md` after this change
- WHEN searched with `rg "TaskLocal|ambient" ARCHITECTURE.md`
- THEN zero matches describe `ServiceContext` propagation

---

### Requirement: Tenant Enforcement Is Fallible and Aborts Before the Operation Body (FR-009)

Unchanged in substance since archival. This is already the enforced contract described
above under "Explicit Context in Proxy Dispatch" (`rt.enforce_tenant(&mut ctx)?` called
before the inner operation, per AD-009) and **INV-003** ("Tenant Enforcement
Preserved"). No further requirement is added here; see those sections and their
scenarios ("Tenant enforcement behavior preserved") for the acceptance contract.

---

### Requirement: ServiceContext Is Not a Parallel Writable Tenant Authority (FR-010)

On the authenticated path, the service-visible tenant MUST be derived per FR-002, not
independently selected by arbitrary code holding a `ServiceContext`. `ServiceContext.tenant_id`
is a private ingress-hint field, writable only through the consuming builder
`ServiceContext::with_tenant_id()` and readable through `tenant_hint()`. `resolved_tenant` is
a separate private field, written only by the `pub(crate)` `set_resolved_tenant`, whose sole
caller is `RuntimeInner::enforce_tenant`. Replacing the ingress hint on an owned context via
`with_tenant_id()` — including after resolution has already run — MUST NOT replace or modify
the canonical tenant: `with_tenant_id()` only ever writes `tenant_id`, never `resolved_tenant`.

#### Scenario: Replacing the ingress hint cannot override the canonical tenant

- GIVEN an owned `ServiceContext` whose `canonical_tenant()` resolved to `"tenant-a"`
- WHEN external code calls `ctx.with_tenant_id("tenant-b")`
- THEN `tenant_hint()` returns `"tenant-b"`
- AND `canonical_tenant()` still returns `"tenant-a"`
- AND no public API can write `resolved_tenant` directly

---

### Requirement: A Canonical Tenant Is Available Before Operation Execution (FR-011)

Before a tenant-scoped operation executes, a canonical tenant value MUST be available
to the runtime for that operation. This is satisfied by the macro-generated call to
`rt.enforce_tenant(&mut ctx)?` placed before the inner operation call (see "Explicit
Context in Proxy Dispatch" above) — on the authenticated path this happens
automatically via FR-002's derivation, without the calling code manually assigning a
tenant per call.

#### Scenario: A canonical tenant is present at the start of execution without manual per-call assignment

- GIVEN an authenticated request to a `#[tenant_scoped]` operation
- WHEN the generated proxy method runs
- THEN `enforce_tenant` has already populated `ctx.canonical_tenant()` before the inner operation body executes, without the caller having set it manually

**Tests**: `runtime_builder::tests::enforce_tenant_ok_sets_canonical_tenant_on_resolvable_context`.

---

### Requirement: Tenant Error Taxonomy Is Reachable in Code (FR-012)

`SecurityError::TenantMismatch { expected, actual }`, `SecurityError::MissingContext`,
and `SecurityError::CrossTenantDenied { reason }` MUST each be distinguishable by
callers — reachable in code (`crates/security-sdk/src/error/mod.rs`), not only
referenced in documentation. `MissingContext` covers both FR-002's "no tenant claim"
case and FR-004's "neither authenticated nor internal-permitted" case; the archived
spec's own wording ("MissingAuthentication/MissingContext... may surface through
RuntimeError, ServiceError, SecurityError, or any combination") permits this
consolidation — no separate `MissingAuthentication` variant exists or is required.

#### Scenario: Each tenant failure mode is programmatically distinguishable

- GIVEN the three failure conditions defined in FR-002, FR-004, and FR-005
- WHEN each is triggered independently
- THEN a caller can `match` on `SecurityError::TenantMismatch { .. }`, `SecurityError::MissingContext`, or `SecurityError::CrossTenantDenied { .. }` respectively — no two conditions are indistinguishable

---

### Requirement: service-sdk Spec Contract Matches Enforced Behavior (FR-013)

Unchanged in intent since archival, and satisfied by this document itself: this spec
section (and "Explicit Context in Proxy Dispatch" / INV-003 above) describes the
fallible `enforce_tenant` check the code actually enforces, including the FR-006
cross-tenant consumption path that was still an open gap when CORE-008A originally
archived. No further requirement is added here.

---

### Requirement: Tenant Authority Is Immutable During Operation Execution (FR-014)

Once the canonical tenant has been established for an operation (per
FR-002/FR-003/FR-011), the tenant used for enforcement MUST remain stable for the
duration of that operation. `CanonicalTenant` has no setters, no public fields, and no
`&mut` API — it is immutable from the instant `TenantResolver::resolve` returns it
(there is no mutation point to close). `ServiceContext.resolved_tenant` is written
exactly once per operation, only by `set_resolved_tenant` (`pub(crate)`, sole caller
`enforce_tenant`); no downstream code — including a later mutation of the raw
`ctx.tenant_id` hint field on a cloned context — can alter the tenant an in-flight
operation enforces against.

#### Scenario: Downstream mutation attempts do not affect an operation already in progress

- GIVEN an operation whose `ctx.canonical_tenant()` has already been resolved
- WHEN downstream code attempts to alter `ctx.tenant_id` (the hint field) or clones `ctx`
- THEN all subsequent enforcement decisions for that operation observe the original `CanonicalTenant`, not the attempted alteration — there is no API to mutate `resolved_tenant` outside `crate::runtime`

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

**INV-003 — Tenant Enforcement Preserved**: for a `#[tenant_scoped]` operation, `enforce_tenant`
MUST be called with the same `ServiceContext` that was passed to the proxy method, and it is a
**fallible** check (CORE-008A AD-009, FR-009): on failure the operation body MUST NOT be
entered — the caller observes the enforcement error as the outcome of the call. No tenant check
may be skipped or reordered. (An operation with no `#[tenant_scoped]` marker keeps the
pre-existing best-effort, non-blocking call — D1's valid tenant-less execution mode — and is
unaffected by this invariant.)

**INV-004 — Spawned Task Ownership**: Any asynchronous task created through `tokio::spawn`
or equivalent MUST receive `ServiceContext` through ownership transfer, explicit parameter
passing, or cloning at the call site before the spawn boundary. No spawned task MAY perform
an ambient lookup to obtain a `ServiceContext` after crossing the spawn boundary.

---

## Declarative Authorization with `#[authorize]` Macro (CORE-015)

### Requirement: `#[authorize]` Syntax Contract

The macro `#[authorize]` accepts exactly two named arguments: `context = <ident>` and `permission = "<resource>:<action>"`.

**Acceptance criteria:**

- AC-1.1: `#[authorize(context = ctx, permission = "orders:read")]` on a service method inside `#[service]` compiles and generates an authorization guard.
- AC-1.2: The named argument `context` receives an identifier, not an expression or path.
- AC-1.3: The named argument `permission` receives a string literal, not a const reference, macro call, or any other expression form.

---

### Requirement: Named-Argument Form Is Required

**Acceptance criteria:**

- AC-2.1: `#[authorize(ctx, "orders:read")]` (positional) fails compilation with error E4 (`unknown argument`).
- AC-2.2: `#[authorize(context = ctx, perm = "orders:read")]` (unknown key name) fails compilation with error E4.
- AC-2.3: `#[authorize(context = ctx)]` (missing `permission`) fails compilation with error E4b.
- AC-2.4: `#[authorize(permission = "orders:read")]` (missing `context`) fails compilation with error E4b.

---

### Requirement: Compile-Time Structural Validation of Permission Literal

The permission literal must satisfy: exactly one `:`, non-empty string before `:` (resource), non-empty string after `:` (action). No semantic constraints are applied beyond this structure.

**Acceptance criteria:**

- AC-3.1: A permission literal with no `:` (e.g., `"ordersread"`) fails compilation with error E1.
- AC-3.2: A permission literal with more than one `:` (e.g., `"a:b:c"`) fails compilation with error E1b.
- AC-3.3: A permission literal with an empty resource (e.g., `":read"`) fails compilation with error E2.
- AC-3.4: A permission literal with an empty action (e.g., `"orders:"`) fails compilation with error E3.
- AC-3.5: A non-literal value for `permission` (e.g., a const reference `PERM_CONST`) fails compilation with the non-literal error.
- AC-3.6: A valid literal like `"orders:read"` does not trigger E2 (non-empty resource is correctly identified).

---

### Requirement: Guard Execution Order and Behavior

Authorization guard executes BEFORE the method body; exactly one `authorize_in_context` call per annotated method.

**Acceptance criteria:**

- AC-4.1: When the authorization provider denies the request, the service method body does not execute (no observable side effect from the body).
- AC-4.2: The generated proxy contains exactly one call to `authorize_in_context` per `#[authorize]`-annotated method.
- AC-4.3: The authorization guard appears as the first executable step in the generated proxy body, before `enforce_tenant`, interceptor `on_request`, and the inner method call.

---

### Requirement: Fail-Closed Policy When Security Is Enabled

Authorization is fail-closed when security is enabled — absent or unavailable providers must return an error.

| Security state | Guard behavior | Error returned |
|---|---|---|
| `ctx.security()` is `None` (security capability disabled) | Guard not emitted; call proceeds | — |
| Security enabled; `runtime.upgrade()` returns `None` (runtime dropped) | Fail closed | `SecurityError::ProviderError("authorization provider unavailable: runtime dropped")` |
| Security enabled; authorization resolution yields `CapabilityNotEnabled` | Fail closed | `SecurityError::CapabilityNotEnabled` |
| Security enabled; provider present; provider denies | Fail closed | `SecurityError::AuthorizationDenied { .. }` (propagated from provider) |
| Security enabled; provider present; provider allows | Guard passes; body executes | — |

**Acceptance criteria:**

- AC-5.1: When `ctx.security()` is `None`, the method body executes without any authorization check.
- AC-5.2: When the runtime `Weak` reference has been dropped and `ctx.security()` is `Some`, the method returns `Err(E::from(SecurityError::ProviderError(...)))`.
- AC-5.3: When authorization resolution yields `SecurityError::CapabilityNotEnabled`, the generated guard propagates that error and the method body does not execute.
- AC-5.4: When the provider returns `Deny`, the method returns `Err(E::from(SecurityError::AuthorizationDenied { .. }))` and the body does not execute.
- AC-5.5: When the provider returns `Allow`, the method body executes and returns its result.

---

### Requirement: Compile-Time `From<SecurityError>` Bound on Error Type

**Acceptance criteria:**

- AC-6.1: A method whose `Result<_, E>` has an error type `E` that does not implement `From<SecurityError>` fails compilation with error E_from.
- AC-6.2: The compile error is rustc's standard trait bound diagnostic, triggered by the `__assert_from_security_error::<E>()` helper; the span targets the error type with a message identifying the missing `impl From<SecurityError> for E`. No custom `compile_error!` is emitted.

---

### Requirement: `#[authorize]` Outside `#[service]` Emits Compile Error

**Acceptance criteria:**

- AC-7.1: `#[authorize]` applied to a free function (outside any `#[service]` impl block) fails compilation with error E5.
- AC-7.2: `#[authorize]` applied to a function inside a plain `impl` block (not `#[service]`) fails compilation with error E5.
- AC-7.3: When `#[authorize]` is used correctly inside `#[service]`, error E5 is never emitted.

---

### Requirement: Marker Execution Order Is Fixed and Lexical-Order-Independent

The pipeline order is fixed and independent of attribute lexical order:

```
1. authorize
2. [future pre-body marker]
3. enforce_tenant
4. chain.on_request
5. inner.method(args)
6. chain.on_response / on_error
7. [future post-body marker]
8. return result
```

**Acceptance criteria:**

- AC-8.1: A method annotated `#[audit] #[authorize(...)]` generates the same proxy body as `#[authorize(...)] #[audit]` — the order of authorization relative to other markers is determined by the pipeline, not by lexical attribute position.
- AC-8.2: The generated proxy always places the authorization guard at slot 1 (before `enforce_tenant`, before interceptors).

---

### Requirement: `ServiceContext` Remains a Pure DTO

**Acceptance criteria:**

- AC-9.1: No new methods, fields, or trait implementations are added to `ServiceContext` in this change.
- AC-9.2: `ServiceContext` does not expose a reference or accessor to any runtime provider.

**Scope note (PROD-003):** AC-9.1's "in this change" bounds the change that
introduced this requirement — it froze `ServiceContext`'s surface *for that
change*, it is not a permanent freeze on the type. AC-9's durable intent is
AC-9.2: `ServiceContext` stays a **pure data DTO** and MUST NOT expose a
runtime provider / behavior. A later change MAY add additive, **data-only**
fields — e.g. PROD-003's explicit `trace_context: Option<TraceContext>`
(trace identity carried by value, no provider reference, no ambient access) —
without violating AC-9.2. Such additions remain DTO-compatible.

---

### Requirement: `RuntimeInner::authorization_provider()` Accessor Added

**Acceptance criteria:**

- AC-10.1: `RuntimeInner` exposes `pub fn authorization_provider(&self) -> Option<Arc<dyn AuthorizationProvider>>`.
- AC-10.2: The method returns `None` when no security providers are configured.
- AC-10.3: The method returns `Some(Arc<dyn AuthorizationProvider>)` (an owned clone) when an authorization provider is configured.
- AC-10.4: The authentication provider remains inaccessible; only the authorization `Arc` is exposed.

**Accessibility contract**: This accessor is `pub` solely to satisfy Rust's visibility rules for code generated by proc-macros. It is not part of the application programming model; application code must not call it directly. Any future public accessor on `RuntimeInner` requires an explicit ADR.

---

### Non-Functional: No New Public API Beyond `RuntimeInner::authorization_provider()` and `#[authorize]`

- No new types, traits, or functions are added to any public crate surface beyond those two items.

---

### Non-Functional: Generated Internals Are Not Public API

The following generated identifiers are implementation details, not part of any stability contract:

| Identifier | Role |
|---|---|
| `__rt` | Temporary `Arc<RuntimeInner>` in the proxy body |
| `__provider` | Temporary `Arc<dyn AuthorizationProvider>` in the proxy body |
| `__assert_from_security_error` | Zero-size helper function enforcing the `From<SecurityError>` bound |

These names MUST NOT appear in hand-written application code. `cargo expand` output is a debugging aid, not a compatibility contract.

---

### Non-Functional: Allocation Overhead Is Accepted

Generated code constructs `Resource { kind: "...".to_string(), .. }` and `Action("...".to_string())` — two `String` allocations per authorized call. These allocations are intentional, reusing the stable `security-sdk` `Resource`/`Action` owned API. Allocation-free variants are deferred to a future `security-sdk` API change.

---

### Diagnostics Contract for `#[authorize]` Errors

All errors are span-targeted at the offending token.

| Code | Trigger | Required message |
|---|---|---|
| E1 | Permission literal has no `:` | `#[authorize] permission "foo" must have the form "resource:action"` |
| E1b | Permission literal has more than one `:` | `#[authorize] permission "a:b:c" must have exactly one ':' (form "resource:action")` |
| E2 | Empty resource (e.g., `":read"`) | `#[authorize] resource in ":read" must not be empty` |
| E3 | Empty action (e.g., `"orders:"`) | `#[authorize] action in "orders:" must not be empty` |
| E4 | Unknown named argument | `#[authorize] unknown argument 'foo'; expected 'context' and 'permission'` |
| E4b | Missing required argument | `#[authorize] missing required argument; both 'context' and 'permission' are required` |
| E5 | `#[authorize]` used outside `#[service]` | `#[authorize] can only be used on methods inside a #[service] trait` |
| E6 | `context = <ident>` names a param not present in the method signature | `#[authorize] context parameter 'ctx' not found in method signature` |
| E_from | Method error type lacks `From<SecurityError>` | rustc trait bound error at error type (e.g., `the trait bound \`OrderError: From<SecurityError>\` is not satisfied`); emitted by `__assert_from_security_error::<E>()` helper — no custom message |
| AD-4 (non-literal) | `permission` value is not a string literal | `#[authorize] permission must be a string literal known at compile time` |
| AD-4 (non-ident) | `context` value is not an identifier | `#[authorize] context must be a parameter name (identifier), not an expression` |

---

## Observability for Macro-Driven Security Enforcement (CORE-012A)

This section describes the observability instrumentation for `#[authorize]` and `#[tenant_scoped]` macro-driven enforcement denials.

### Requirement: Reachable Macro-Guard Denials Are Recorded

Each denied invocation of a `#[authorize]` and/or `#[tenant_scoped]` guarded operation MUST produce exactly one recorded `Observability` event for the denial that occurred. Because guard evaluation short-circuits on the first denial, at most one of `MissingContext`, `TenantMismatch`, or `AuthorizationDenied` MUST ever be recorded for a single denied call, regardless of how many guard attributes are present.

#### Scenario: A single-guard denial records one event
- GIVEN an operation guarded only by `#[authorize]`
- WHEN the invocation is denied with `AuthorizationDenied`
- THEN exactly one event is recorded, reporting `AuthorizationDenied`

#### Scenario: A denial with both attributes present still records exactly one event
- GIVEN an operation guarded by both `#[authorize]` and `#[tenant_scoped]`
- WHEN the invocation is denied because authorization fails
- THEN exactly one event is recorded (`AuthorizationDenied`), and no second tenant-related event is recorded for that call

#### Scenario: Allowed invocations record no denial event
- GIVEN an operation guarded by `#[authorize]` and `#[tenant_scoped]`
- WHEN the invocation passes both guards
- THEN no denial event is recorded, and existing denial semantics and guard order are unaffected

---

### Requirement: Minimum Recorded Event Contract

Every recorded denial event MUST contain at minimum: denial kind, service name, and operation name. Additional contextual fields (e.g. correlation id, actor id, tenant identifier, metadata) are optional; their absence MUST NOT fail this contract.

#### Scenario: A minimal event with only the three required fields satisfies the contract
- GIVEN a denied invocation is recorded with only denial kind, service name, and operation name populated
- WHEN the recorded event is checked against this contract
- THEN the event satisfies the contract

#### Scenario: A missing required field violates the contract
- GIVEN a recorded event for a denied invocation
- WHEN denial kind, service name, or operation name is absent
- THEN the event does not satisfy this contract

---

### Requirement: Recorded Denial Data Is Redacted

Recorded denial event data MUST NOT expose raw tenant identifiers or denial-reason strings in its recorded/`Display`-safe form, following the same `Display`/`Debug` split already established by the `SecurityError` convention (`security-sdk/src/error/mod.rs:47-75`). Full diagnostic detail MUST remain available only via `Debug`, never via the recorded or `Display`-safe form.

#### Scenario: Recorded event omits raw tenant id and denial reason
- GIVEN a `TenantMismatch` denial carrying a specific tenant id and mismatch reason
- WHEN the resulting event is observed in its recorded/`Display`-safe form
- THEN neither the raw tenant id nor the denial-reason string appears

#### Scenario: Full diagnostic detail remains available via the original error's Debug only
- GIVEN the same denied invocation, which independently produces and returns a `SecurityError::TenantMismatch` to the caller per the pre-existing AD-010 convention
- WHEN that returned error value is inspected via `Debug`
- THEN the raw tenant id and denial reason are present there, for internal diagnostics only — this change does not need to duplicate that detail into the recorded event's own representation to satisfy this requirement

---

### Requirement: Runtime Accepts an Observability Implementor, Default Behavior Unchanged

`RuntimeBuilder` MUST expose `with_observability(...)` allowing callers to supply an `Observability` implementor at build time. When it is not called, the runtime MUST keep no observability sink configured (`None`); denial recording MUST behave as a silent no-op, with return values, errors, guard ordering, and panic behavior identical to the runtime's behavior before this change.

#### Scenario: Supplying an Observability implementor is accepted at build time
- GIVEN a `RuntimeBuilder` configured with `.with_observability(some_implementor)`
- WHEN the runtime is built and a guarded operation is invoked
- THEN the build succeeds and denial recording uses the supplied implementor

#### Scenario: Omitting with_observability preserves today's behavior exactly
- GIVEN a `RuntimeBuilder` on which `.with_observability(...)` is never called
- WHEN the runtime is built and any guarded operation (allowed or denied) is invoked
- THEN behavior is identical to before this change — same return values, same errors, no new panics — with no sink configured, so denial recording is a silent no-op

---

### Requirement: CrossTenantDenied Remains Uninstrumented By Design

`CrossTenantDenied` MUST NOT be instrumented by this change. This is a deliberate deferral, not an oversight: no macro-reachable call path exists today that can produce a `CrossTenantDenied` outcome, so leaving it uninstrumented affects no caller-observable behavior and creates no regression when a future change adds such a path.

#### Scenario: No reachable path emits a CrossTenantDenied event today
- GIVEN the current set of macro-guarded operations reachable through `#[authorize]` and `#[tenant_scoped]`
- WHEN any of them is invoked, allowed or denied
- THEN no `CrossTenantDenied` event is ever produced, because no macro-reachable caller can trigger this outcome

#### Scenario: A future CrossTenantDenied caller does not conflict with this change
- GIVEN this change ships with `CrossTenantDenied` uninstrumented
- WHEN a future change introduces a macro-reachable path producing `CrossTenantDenied`
- THEN that future change may add instrumentation without contradicting or requiring rework of any requirement in this spec

---

## Service Registration — Struct-Macro Trait Link (CORE-028 Stage 2B)

### Requirement: Optional Struct-Macro Trait-Link Argument (`impl_of`)

The `#[service]` struct macro MUST accept an optional `impl_of` argument
naming the trait the struct implements, in the same explicit-argument style
the macro already uses elsewhere (`#[service(impl_of = Trait)]`, or
`#[service(impl_of = crate::module::Trait)]` for a path-qualified trait —
the generated Tag suffix applies only to the final path segment). When this
argument is present, the
macro MUST generate, at expansion time, a link from the struct to the
resolution Tag associated with the named trait, together with a concrete
coercion from an `Arc` of the struct to an `Arc<dyn Trait>`. This generated
link is what allows a single-type-parameter registration call to know, at
compile time, both which Tag to register under and how to produce the
trait-object coercion — information only macro-expansion-time code has,
because the caller cannot express "this struct implements whichever trait
underlies this Tag" as a Rust generic bound.

When the argument is absent, the macro's behavior on a struct MUST be
exactly what it is today: only the existing `Injectable`-related generation
occurs, with no trait link produced. Bare `#[service]` usage — including
testkit's — MUST compile and behave identically before and after this
change.

If the named trait argument does not name a trait the struct actually
implements, this MUST surface as a compile error at the macro-generated
code's location (an ordinary "trait not implemented"-shaped failure) — no
special macro diagnostic is required, but silently accepting a wrong or
unimplemented trait name, or deferring the mismatch to runtime, are not
acceptable outcomes.

#### Scenario: Bare `#[service]` struct usage is unaffected
- GIVEN a struct annotated `#[service]` with no trait-link argument, as
  written before this change
- WHEN the crate is compiled
- THEN it compiles and behaves exactly as before this change — no new
  required argument, no new generated trait link, no observable difference

#### Scenario: `#[service(impl_of = Trait)]` generates a usable trait link
- GIVEN a struct that implements `Trait`, annotated with the macro's optional
  trait-link argument naming `Trait`
- WHEN the crate is compiled
- THEN the macro generates a link from the struct to `Trait`'s resolution
  Tag and a coercion producing `Arc<dyn Trait>` from an `Arc` of the struct,
  and this link is what a single-type-parameter service registration call
  (see the companion `application-composition` delta) consumes to register
  and resolve the struct with no caller-supplied Tag or coercion closure

#### Scenario: A trait-link argument naming a trait the struct does not implement fails to compile
- GIVEN a struct annotated with the macro's trait-link argument naming a
  trait the struct does not implement
- WHEN the crate is compiled
- THEN compilation fails, identifying the trait the struct fails to satisfy
  — the mismatch is caught at compile time, not accepted and left to
  surface later as a runtime registration or resolution failure

### Non-Goals

- Coupling entity registration to this trait-link mechanism (`impl_of`) —
  entity registration follows the projection-registration pattern (a plain
  generic parameter naming the aggregate type), not the macro-generated
  trait-link pattern services use (CORE-028 Stage 2C).
- Framework-owned construction of the entity runtime (activation,
  passivation, config folding, or any change to `EntityRuntimeBuilder` or
  `EntityRegistry`) — CORE-028 Stage 2C's entity registration only registers
  and resolves a host-constructed runtime.
- Entity lifecycle ownership (spawn/stop) — entity actors are unaffected by
  this capability's registration or resolution requirements.
- A runtime or link-time registry (`inventory`, `linkme`, `ctor`, or
  equivalent) to discover macro-linked services — the link is a
  compile-time-only construct consumed directly by generated code and the
  registration call; no new dependency is introduced.
- Inferring the implemented trait from the struct's name (e.g. stripping an
  `Impl` suffix) — the trait is only ever named explicitly through the
  macro argument.
- Any change to trait-level or method-level `#[service]` macro behavior, or
  to the existing `Injectable` contract itself.

<!-- PROD-003: distributed tracing on ServiceContext (merged from change delta) -->

### Requirement: ServiceContext Carries an Explicit TraceContext Value

`ServiceContext` MUST carry an explicit `TraceContext` value (trace-id,
span-id, optional parent), settable via `with_trace_context(TraceContext)`
and readable via `trace_context()`. No ambient, thread-local, or task-local
storage MUST be used to carry it between construction and read. The
existing flat `trace_id` field MUST become a read-through mirror of
`trace_context().trace_id` (source-compatible, not an independent value),
and `TraceContext` MUST be authoritative over it BY CONSTRUCTION: the
`trace_id` field MUST be private, `with_trace_context` MUST set the mirror to
`trace_context().trace_id()`, and `with_trace_id` MUST write the legacy value
ONLY when no `TraceContext` is present (otherwise it is ignored — the
`TraceContext` wins). This makes `trace_id()` unable to desync from
`trace_context().trace_id()` regardless of builder call order. The existing
`correlation_id` field is a distinct business-causal concept, stays public,
and is NOT changed by this delta.

#### Scenario: Trace-context travels only via the explicit ServiceContext value
- GIVEN a call chain that passes `ServiceContext` explicitly across
  functions and `.await`/spawned-task boundaries
- WHEN the trace-context is needed at any point in that chain
- THEN it is obtained only by reading the passed `ServiceContext` value, with
  no ambient/thread-local/task-local lookup involved

#### Scenario: Flat trace_id mirrors trace_context().trace_id
- GIVEN a `ServiceContext` constructed via `with_trace_context(tc)`
- WHEN the flat `trace_id` field/accessor is read
- THEN its value equals `tc.trace_id`, with no independent trace-id storage

#### Scenario: TraceContext wins over with_trace_id regardless of call order
- GIVEN a `ServiceContext` built with both `with_trace_context(tc)` and
  `with_trace_id("x")` applied in either order
- WHEN `trace_id()` is read
- THEN it equals `tc.trace_id().to_hex()` (the `with_trace_id("x")` legacy
  write is ignored while a `TraceContext` is present), and only when no
  `TraceContext` is set does `with_trace_id("x")` make `trace_id()` return
  `"x"`

#### Scenario: correlation_id is unaffected by trace-context changes
- GIVEN a `ServiceContext` with both a `correlation_id` and a `trace_context`
  set
- WHEN the `trace_context` is read or replaced
- THEN `correlation_id` is unchanged, remaining the distinct business-causal
  identifier

### Requirement: Ambient Span/Context APIs Confined to the Infra OTLP Adapter

Any use of `Span::current()`, `Context::current()`, or equivalent
ambient-context APIs MUST be confined to the `infrastructure` crate's OTLP
adapter module. Service-author-facing code (services, interceptors,
handlers) MUST NOT call or rely on such ambient APIs to obtain or propagate
trace-context.

#### Scenario: Service code never touches ambient span/context APIs
- GIVEN service-author code outside the infrastructure OTLP adapter
- WHEN that code needs the current trace-context
- THEN it obtains it from the explicit `ServiceContext` value only, never
  from `Span::current()` or `Context::current()`

#### Scenario: Boundary lint fails if ambient APIs leak outside the adapter
- GIVEN a source-scan test (in the style of the existing
  `tenant_scoped_lint`) that scans all crates except the `infrastructure`
  OTLP adapter module
- WHEN it runs
- THEN it fails if `Context::current()` or `Span::current()` appears
  anywhere outside that module

### Requirement: TracingInterceptor Drives Span Lifecycle From ServiceContext

The built-in `TracingInterceptor` MUST, on `on_request`, call
`tracer.start_span(ctx.trace_context(), name, attrs)` (which returns nothing).
The span identity is `ctx.trace_context().span_id()`, re-derived from `ctx`.
On `on_response` it MUST call
`tracer.end_span(ctx.trace_context().span_id(), SpanOutcome::Ok)`. On
`on_error` it MUST call
`tracer.end_span(ctx.trace_context().span_id(), SpanOutcome::Error { status_message })`
with a redaction-safe `status_message`. Exactly one span MUST be owned per
request boundary — the interceptor MUST NOT call `ServiceContext::with_span`
(not present in v1) or invoke `TraceContext::child()`.

#### Scenario: Successful invocation starts and ends exactly one span
- GIVEN `TracingInterceptor` is installed and `on_request` calls `start_span`
  (which returns nothing) for the span identified by
  `S = ctx.trace_context().span_id()`
- WHEN `on_response` runs
- THEN it calls `end_span(S, Ok)` with `S` re-derived from
  `ctx.trace_context().span_id()`, closing exactly the span identified by `S`

#### Scenario: Failed invocation ends the span with a redaction-safe error message
- GIVEN `on_request` started a span with `SpanId` `S`
- WHEN the invocation fails and `on_error` runs
- THEN it calls `end_span(S, SpanOutcome::Error { status_message })` with a
  redaction-safe message, re-deriving `S` from `ctx` with no stored guard

#### Scenario: No with_span and no manual nested span in v1
- GIVEN `TracingInterceptor` is the only span owner in v1
- WHEN a request is handled
- THEN no `ServiceContext::with_span` call occurs, and `TraceContext::child()`
  is not invoked by any v1 code path

### Requirement: Trace-Context Originates At HTTP Ingress

Trace-context MUST be originated at HTTP ingress only, exactly once, at the
HTTP handler boundary: `TraceContext::from_inbound(traceparent)` MUST be
used when an inbound `traceparent` header is present, else
`TraceContext::root()`. The resulting `TraceContext` is then carried
explicitly via `ServiceContext` for the remainder of the call.
Message-consumer and actor/effect-runner trace-context origination are OUT
OF SCOPE for this delta.

#### Scenario: HTTP ingress with no traceparent uses root()
- GIVEN an inbound HTTP request with no `traceparent` header
- WHEN the HTTP handler constructs the `ServiceContext` for that request
- THEN `TraceContext::root()` is used, creating a new trace-id and root span

#### Scenario: HTTP ingress with a traceparent uses from_inbound()
- GIVEN an inbound HTTP request carrying a valid `traceparent` header for
  trace `T`, remote span `R`
- WHEN the HTTP handler constructs `ServiceContext` via
  `TraceContext::from_inbound(header)`
- THEN the resulting `TraceContext` has `trace_id` `T`, `parent_span_id`
  `Some(R)`, and a new local `span_id` distinct from `R`

## Out of Scope (Non-Goals for this Delta)

This delta does not add OTLP-exported metrics, OTLP-exported logs, or
tracing/propagation origination for actor/effect-runner execution or
message-consumer ingress (`persistent-entity`, `ego-scheduler`) — deferred,
no wire-header model exists for messaging. It does not add
`ServiceContext::with_span`; `TraceContext::child()` is retained only as a
future seam and is not exercised by any v1 requirement. It does not change
sampling (always-on, decided at the `Tracer` port level). It does not
change the `Observability` port or the CORE-012A macro-guard
denial-recording contract (`service-sdk/spec.md` — Observability for
Macro-Driven Security Enforcement section), which remain untouched. Spans
for macro-guard denials are not produced (guard denials short-circuit
before the interceptor chain); this is a documented v1 limitation.

## Idempotent Command Processing Support (PROD-012)

### Requirement: ServiceContext Exposes the Operation Key

`ServiceContext` MUST expose an accessor for the `OperationKey` established
at ingress for the current request, following the same explicit-ownership
model as its other accessors — no ambient lookup.

#### Scenario: Handler reads the operation key from context

- GIVEN a `ServiceContext` carrying an `OperationKey` established at ingress
- WHEN a handler calls the context's operation-key accessor
- THEN it receives the identical `OperationKey`, with no ambient or global
  lookup involved

### Requirement: RuntimeBuilder Registers the Reservation Store, Fail-Closed

`RuntimeBuilder` MUST support registering exactly one
`OperationReservationStore` implementation. When `IdempotencyEnforcementMode`
resolves to its enforcing (default) variant and no `OperationReservationStore`
is registered, `build()`/`try_build()` MUST fail rather than start a runtime
that cannot honor the mandatory-key guarantee.

#### Scenario: Startup fails closed when enforcement is on with no store registered

- GIVEN a `RuntimeBuilder` with the default (enforcing)
  `IdempotencyEnforcementMode` and no `OperationReservationStore` registered
- WHEN the runtime is built
- THEN build fails, naming the missing registration — no runtime starts

#### Scenario: Registered store enables a successful build under enforcement

- GIVEN a `RuntimeBuilder` with an `OperationReservationStore` registered and
  the default enforcing mode
- WHEN the runtime is built
- THEN build succeeds

### Requirement: RuntimeBuilder Registers a Single Injectable Clock

`RuntimeBuilder` MUST support registering a `Clock` (generalized out of the
existing auth `Clock`), defaulting to a system-clock implementation. The
registered `Clock` MUST be the sole time source injected into the
`OperationReservationStore`, which MUST NOT call `Utc::now()` directly.

This requirement deliberately says nothing about the effects subsystem. An
earlier draft also bound `EffectDedupStore`, on the mistaken premise that it
read the wall clock; its methods neither take nor read time, so there is
nothing there to inject.

#### Scenario: A custom Clock is the reservation store's only time source

- GIVEN a `RuntimeBuilder` registered with a deterministic test `Clock`
- WHEN the reservation store reads the current time
- THEN it observes the injected `Clock` value, with no direct `Utc::now()`
  call in its path

### Requirement: RuntimeBuilder Registers Enforcement Mode and Retention Policy

`RuntimeBuilder` MUST support configuring `IdempotencyEnforcementMode` and a
retention policy (TTL for reservations/stored responses, purge batch size).
Omitting configuration MUST resolve to the fail-closed enforcement default.

#### Scenario: Default configuration is fail-closed

- GIVEN a `RuntimeBuilder` with no explicit `IdempotencyEnforcementMode` call
- WHEN the runtime is built
- THEN the effective mode is the fail-closed mandatory-key variant

### Requirement: Purge-Worker Lifecycle Follows Existing Ordering

The reservation-purge background worker MUST start and stop under the same
lifecycle ordering contract (CORE-017) that governs other runtime-owned
background work. Shutdown MUST NOT release or abandon an in-progress
reservation's lease merely because the worker is stopping — an
in-progress lease is only ever resolved through its own expiry/takeover
path, never through a shutdown-triggered release.

#### Scenario: Shutdown does not release in-progress leases

- GIVEN a reservation `InProgress` with an active lease at the moment of
  runtime shutdown
- WHEN the purge worker and runtime shut down
- THEN the lease is left untouched by shutdown; it is only resolved later by
  expiry and takeover, never released as a side effect of shutdown
