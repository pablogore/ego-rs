# Delta for service-sdk

Scope: CORE-025 F-01, F-02, F-03, F-09. Wires the existing `ServiceRegistry`/`Resolvable`/`Injectable` machinery into `RuntimeBuilder`/`Runtime`'s public surface. Additive only — no existing requirement's text is altered.

## ADDED Requirements

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
