# Ergonomics Audit — Service SDK Developer Experience (CORE-025, proposed number)

**Scope of this document:** Phase 1 only — measurement, not solutioning. No API design decisions are made here. All claims below are cited against real file:line locations in the workspace as of 2026-07-10, verified by four independent research passes (minimal-service journey, DI + error journey, TestKit/security journey, comparables + docs).

---

## Executive Summary

The audit surfaced one finding that reframes the whole effort: **the SDK's documented service-registration path does not exist in production.** `ServiceRegistry`, `Resolvable`, and `Runtime::resolve()` are described in doc comments and `COOKBOOK.md` as the intended developer flow, but:

- `Runtime` has zero `resolve()` method (grepped `fn resolve\b` across the crate — nothing on `Runtime`).
- `RuntimeBuilder` has no `with_service`/`register_service` method — its internal `registry` field is permanently empty (`#[allow(dead_code)]`, comment: *"Populated by RuntimeBuilder (TASK-013); not yet read within this crate"*, `runtime/runtime_builder.rs:115-117`).
- `ServiceRegistry::register`/`resolve_raw` and `Resolvable::create_proxy` are called **only from their own unit tests** — zero other call sites anywhere in the workspace, including examples, TestKit, and the macro's own codegen tests.
- Every real test and example — `smoke.rs`, `proxy_codegen.rs`, `golden_codegen.rs`, `tenant_enforcement_contract.rs`, `authorization_integration.rs`, `tenant_scoped_codegen.rs`, and the example literally titled to show "the intended developer experience" — instead hand-rolls the same four-line sequence: build a `Runtime`, `Arc::downgrade(rt.inner())`, wrap zero-or-more interceptors in an `InterceptorChain`, and call the macro-generated `{Trait}Ref::new(inner, chain, weak)` directly.

This is not an isolated dead-code finding — it changes what "ergonomics improvement" means here. There is no working baseline to streamline; the actual baseline is: **construct everything by hand, because the intended shortcut was never wired up.** TestKit doesn't diverge from a production path for this piece because no production path exists to diverge from — confirmed independently by all four research passes and by TestKit's own doc comment: *"for forward compatibility with a future public `Runtime::resolve`"* (`crates/testkit/src/fixtures.rs:82-83`).

Separately, the DI path (`Injectable`/`AdapterRef`/`ConfigValue`) is real, macro-driven, and genuinely shared between production-style code and TestKit — but `RuntimeBuilder::build()` performs zero validation ("Always succeeds — security and the logger are both optional," doc comment, `runtime/builder.rs`), and the one failure signal (`RuntimeError::DependencyNotFound`) carries no type name, no service name, and has no `Display`/`Error` impl at all.

---

## A. Current Developer Journey

### A.1 — Minimal service, zero dependencies

**Define** (`crates/service-sdk-macros/src/lib.rs:59-75`, `expand_service_trait` 77-517):
```rust
#[service(version = "1.0.0")]
pub trait HelloService {
    #[operation]
    async fn greet(&self, ctx: ServiceContext, name: String) -> Result<String, ServiceError>;
}
```
`#[operation]` (`lib.rs:664-668`) is a marker consumed by `#[service]`. The macro generates, per trait: a zero-sized `HelloServiceTag`, a proxy `HelloServiceRef`, `impl ServiceContract for HelloServiceTag`, `impl Resolvable for HelloServiceTag` (lines 442-517).

**Implement** — one manual trait impl:
```rust
pub struct HelloServiceImpl;
#[async_trait]
impl HelloService for HelloServiceImpl {
    async fn greet(&self, _ctx: ServiceContext, name: String) -> Result<String, ServiceError> {
        Ok(format!("Hello, {name}"))
    }
}
```
`Service`/`ServiceFactory`/`LifecycleManaged` (`crates/service-sdk/src/implementation.rs:16-63`) are untouched by this path — see F-04.

**Build the runtime** (`runtime/builder.rs:46-59, 124-145`):
```rust
let rt = RuntimeBuilder::new().build();
```

**"Register" and get a typed reference** — no working registration API; every real caller does this instead (`tests/proxy_codegen.rs:75-81`):
```rust
let inner: Arc<dyn HelloService> = Arc::new(HelloServiceImpl);
let chain = Arc::new(InterceptorChain::new());
let runtime_weak = Arc::downgrade(rt.inner());
let proxy = HelloServiceRef::new(inner, chain, runtime_weak);
```

**Invoke** (`lib.rs:382-401`):
```rust
let result = proxy.greet(ServiceContext::new(), "world".to_string()).await?;
```

Measured cost: 1-2 files, 1 manual trait impl, 2 macro attributes, ~20-25 lines of hand-written ceremony excluding business logic. See Section C for the full concept list.

### A.2 — Service with DI (adapter + typed config)

DI primitives (`crates/service-sdk/src/di/mod.rs:35-99`): `AdapterRef<A>`, `ConfigValue<T>` (thin `Arc`-wrapping newtypes), `DepKey` (`Entity | Projection | Adapter | Config`, each a `TypeId`), and:
```rust
pub trait Injectable: Send + Sync {
    fn dependencies() -> Vec<DepKey> where Self: Sized;
    fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> where Self: Sized;
}
```
`#[service]` on a **struct** (`expand_service_struct`, `lib.rs:519-561`; field classification `classify_field_init`/`classify_field_type`, 564-631) generates `Injectable` for you:
```rust
#[service]
struct MyService {
    db: AdapterRef<MyDbAdapter>,
    limit: ConfigValue<u32>,
}
// macro emits: impl Injectable for MyService { dependencies() / build() }
```
```rust
let rt = RuntimeBuilder::new()
    .with_adapter(Arc::new(MyDbAdapter::new()))
    .with_config(Arc::new(10u32))
    .build();
let svc = MyService::build(rt.inner())?;
```
**This `Injectable::build` call is the only DI entry point exercised anywhere in the workspace, and today it is invoked only from `crates/testkit/src/fixtures.rs`'s `ServiceTestFixture::service::<S>()` and from unit tests — never from a production/bootstrap code path.** `Injectable::dependencies()` — the full compile-time-known list of what a service needs — is likewise never consulted by `RuntimeBuilder::build()` or anything else in production; it exists only to be snapshot-tested (`golden_codegen.rs`, `proxy_codegen.rs`).

### A.3 — Security/tenant-scoped service

Real trait (`tests/tenant_enforcement_contract.rs:79-87`):
```rust
#[service(version = "1.0.0")]
pub trait TenantContractService {
    #[operation]
    #[tenant_scoped]
    async fn scoped_op(&self, ctx: ServiceContext) -> Result<Option<String>, TenantContractError>;

    #[operation]
    async fn unscoped_op(&self, ctx: ServiceContext) -> Result<bool, TenantContractError>;
}
```
`#[authorize]` variant (`tests/authorization_integration.rs:158-163`):
```rust
#[operation]
#[authorize(context = ctx, permission = "orders:read")]
async fn get_order(&self, ctx: ServiceContext, id: String) -> Result<String, AuthTestError>;
```
The generated proxy's forwarding method (`lib.rs:264-401`) runs, in order: `authorize_guard` (if `#[authorize]`) → `enforce_tenant_block` (if `#[tenant_scoped]`, via `runtime_builder.rs:255-264`'s `enforce_tenant`, which writes the resolved tenant into `ctx` through a crate-private setter — immutable for the rest of the operation) → the `InterceptorChain` (`on_request`/`on_response`/`on_error`) → the real inner call. Unmarked operations skip tenant enforcement entirely, by design (AD-007).

Construction (`tenant_enforcement_contract.rs:115-125`):
```rust
fn make_proxy(service: Arc<ContractService>, mode: TenantEnforcementMode) -> (Runtime, TenantContractServiceRef) {
    let inner: Arc<dyn TenantContractService> = service;
    let chain = Arc::new(InterceptorChain::new());
    let rt = RuntimeBuilder::new().with_tenant_enforcement_mode(mode).build();
    let runtime_weak = Arc::downgrade(rt.inner());
    let proxy = TenantContractServiceRef::new(inner, chain, runtime_weak);
    (rt, proxy)
}
```
Near-identical private `make_proxy` helpers are hand-rolled independently in `tenant_enforcement_contract.rs`, `tenant_scoped_codegen.rs`, `authorization_integration.rs`, and three call sites in `proxy_codegen.rs` — see F-06.

### A.4 — TestKit journey

`ServiceTestFixture`/`FixtureBuilder` (`crates/testkit/src/fixtures.rs`) genuinely routes the DI/`Injectable` path through the same production constructor: `FixtureBuilder::build()` calls `RuntimeInner::new_with_logger` (`runtime_builder.rs:164-184`, documented as "the sole constructor... closing the external bypass") — the exact same function `RuntimeBuilder::build()` calls. Verified line-by-line, not by doc comment alone.

For the trait-proxy/enforcement path (A.3), TestKit has **no equivalent at all** — `ServiceTestFixture::service::<S>()` only accepts `S: Injectable`, which the macro only generates for structs, never for the trait-level `{Trait}Ref`. Attempting to pass a `{Trait}Ref` there is a compile error, not a bypass. This is a coverage gap, not a divergence — there is no production path for TestKit to diverge from in the first place (see Executive Summary).

`TestContextBuilder`/`test_context()` (`testkit/src/context.rs`) builds the same real `ServiceContext` type used everywhere else — no divergence there.

`PairingAuthnStub` (`fixtures.rs:22-42`) is a real `AuthenticationProvider` impl required only to satisfy `RuntimeBuilder::with_security`'s all-or-nothing pairing; it's never actually invoked, by design (TestKit drives operations by calling the trait method directly, not through a credential/authenticate flow) — honestly documented in its own doc comment, not a hidden shortcut.

---

## B. Friction Points Found

| ID | Severity | Category | Evidence | Impact | Root cause | Possible direction |
|---|---|---|---|---|---|---|
| F-01 | **CRITICAL** | Discoverability, Consistency | `Runtime` has no `resolve()` (grep-confirmed); `RuntimeBuilder` has no service-registration method; `ServiceRegistry::register`/`Resolvable::create_proxy` called only from their own unit tests (`registry/registry.rs`, `runtime/resolvable.rs`) | A new developer reading the public API (`ServiceRegistry`, `Resolvable`, doc comment at `resolvable.rs:4,42` showing `runtime.resolve::<OrderServiceTag>()`) will try an API that doesn't compile. Every real example instead hand-rolls proxy construction, which isn't documented anywhere as the "real" path. | The registry/resolve/proxy machinery was built (macro generates a full `Resolvable` impl per service) but never connected to `RuntimeBuilder`/`Runtime`'s public surface — an incomplete integration, not a missing feature. | Wire the existing `ServiceRegistry`/`Resolvable` infrastructure into `RuntimeBuilder` and `Runtime` through a canonical public registration/resolution API. No new conceptual layer needed — see Section D. **Method names, signatures, and exactly what gets registered are explicitly NOT decided here — see "Open Questions Carried to Design" below.** |
| F-02 | **CRITICAL** | Error messages, Typing | `RuntimeBuilder::build()` doc comment: "Always succeeds — security and the logger are both optional." Zero validation against any registered service's `Injectable::dependencies()`. | A missing adapter/config is invisible until `Injectable::build()` is actually called — which today only happens in test fixtures, meaning in a hypothetical production bootstrap this would surface only on first real invocation, not at startup. | `dependencies()` is computed at macro-expansion time (compile-time-complete) but nothing in `RuntimeBuilder`/`Runtime` ever reads it. | Add a fail-fast check — once services are actually registered (depends on F-01) — that walks each registered service's `dependencies()` against what's been provided to the builder, before `build()` returns. |
| F-03 | HIGH | Error messages | `RuntimeError::DependencyNotFound` (`runtime_builder.rs:398-403`) is a bare unit variant, `#[derive(Debug, Clone, PartialEq)]` only — no `Display`, no `std::error::Error` impl anywhere in the crate (grep-confirmed). Debug output is literally the string `DependencyNotFound`. | A developer who forgets to register an adapter gets an error that names neither the missing type nor the requesting service — they must bisect their own registration calls to find it. | The error type was scoped to distinguish `ServiceNotFound` vs `DependencyNotFound` as *categories*, not to carry diagnostic payload. | Add fields (`type_name: &'static str`, `service_name: &'static str` or similar) and a `Display` impl. Low-risk, additive. |
| F-04 | HIGH | Consistency, Boilerplate | `ServiceFactory` (`implementation.rs:56-63`) has zero `impl ServiceFactory for` anywhere in the repo (grep-confirmed); the macro never generates or calls it. `COOKBOOK.md` references a `TestServiceFactory` in `crates/service-sdk/src/testing.rs` — that file does not exist. | Three DI/construction mechanisms exist side by side (`Injectable`, `ServiceFactory`, `Resolvable`+`ServiceRegistry`) and only one (`Injectable`) is real. A developer reading `implementation.rs` reasonably assumes `ServiceFactory` is a live extension point. | Appears to be a superseded design left in place after `Injectable` became the actual mechanism — same "built but never adopted" pattern found and resolved for `ExecutionContext`/`ExecutionEnvelope` in CORE-008B. | Confirm zero callers (already done here), then delete — same playbook as CORE-008B, pending its own proposal-review decision. |
| F-05 | HIGH | Documentation | `COOKBOOK.md` Service SDK section (lines 230-377) has ≥6 confirmed-stale claims: (1) shows a blanket `impl<T: MyService> ServiceContract for T` — real macro generates a non-blanket `{Trait}Tag` + `{Trait}Ref`; (2) shows `Service` with `initialize()`/`shutdown()` — those live on a separate `LifecycleManaged` trait by design (regression-tested: `service_trait_has_no_lifecycle_hooks`); (3) describes the registry as "name → descriptor" — actually keyed by `(TypeId of Tag, ContractVersion)`; (4) Testing Guide snippet imports `ego_service_sdk::testing::{TestService, TestServiceFactory, TestInterceptor}` — module doesn't exist, won't compile; (5) File Nav Map lists `src/builder.rs` (`ServiceBuilder`) and `src/reference.rs` (`ServiceReference`) — neither file exists; (6) claims `ServiceError` has 9 variants — actually 10. | Anyone using COOKBOOK.md as a reference will copy a non-compiling snippet or look for files that don't exist. | Docs drifted as the SDK evolved past what COOKBOOK.md described; not caught because nothing validates the doc against source (no doctested code blocks for this section). | Rewrite once the ergonomics slice lands (Section B findings will otherwise go stale again immediately) — sequence this after, not before, the implementation change. |
| F-06 | MEDIUM | Boilerplate, Testing | Near-identical private `make_proxy`-style helpers hand-rolled independently in `tenant_enforcement_contract.rs`, `tenant_scoped_codegen.rs`, `authorization_integration.rs`, and 3 call sites in `proxy_codegen.rs` — all constructing `{Trait}Ref::new(Arc<dyn Trait>, Arc<InterceptorChain>, Weak<RuntimeInner>)` with the identical signature. | Same 4-line ceremony duplicated ≥4 times with no shared helper; a future signature change to `{Trait}Ref::new` must be hunted down in every copy. | No shared "build me a wired proxy for tests" utility exists — likely because there's no production equivalent to extract one from (see F-01). | A thin, generic TestKit helper (`fixture.proxy::<TraitTag>(inner)` or similar) once the underlying construction pattern is stable — natural byproduct of resolving F-01. |
| F-07 | MEDIUM | Testing, Production/TestKit parity | `ServiceTestFixture::service::<S>()` only accepts `S: Injectable` (struct-only); there is no TestKit-side helper for constructing an enforcement-wrapped trait proxy (`{Trait}Ref`). | Every tenant/authz test must hand-roll proxy wiring (same root cause as F-06), and TestKit's crate-level "same-contract" doc claim reads broader than its actual coverage for this specific case. | Consequence of F-01 — there is no production `resolve()` for TestKit to mirror. | Resolve together with F-01/F-06. |
| F-08 | MEDIUM | Error messages | Generated `Injectable::build()` chains resolvers with `?` in field-declaration order (`expand_service_struct`), so a struct with 2+ missing dependencies only ever reports the first one hit. | Fix-rebuild-fix-rebuild cycle instead of seeing every missing dependency at once. | Straightforward `?`-chaining codegen; no aggregation attempted. | Collect all `DepKey` failures before returning, if F-02/F-03 are addressed (natural to batch alongside a `Display` improvement). |
| F-09 | LOW | Documentation, Discoverability | `examples/reference-app` only demonstrates `AppConfig`→`RuntimeBuilder` (CORE-016), never touches `#[service]`/registration/invocation. `crates/service-sdk/examples/order_service.rs` is explicitly headed "illustrative... shows the manual equivalent of what the macros generate" — doesn't use the macros at all. | The repo's only "reference" and "example" service code doesn't show the actual macro-driven happy path end-to-end. | Examples were written to demonstrate specific mechanisms (config wiring, manual desugaring) rather than the full developer journey. | A new minimal end-to-end example, once the slice in Phase 2 stabilizes what "the happy path" actually is. |
| F-10 | INFO | Discoverability | File naming: `runtime/builder.rs` contains `RuntimeBuilder`; `runtime/runtime_builder.rs` contains `RuntimeInner` and `enforce_tenant`/`issue_cross_tenant_permit` — the name suggests the reverse. | Minor navigation friction reading the crate for the first time. | Historical naming, predates current module boundaries. | Cosmetic; not worth its own slice, mention only. |
| F-11 | INFO | Typing | No compile-time detection of a missing dependency is possible in principle (a `RuntimeBuilder` instance's registered adapters/configs are a runtime value, not visible to a proc-macro at expansion time) — but nothing attempts even a fail-fast *bootstrap* check either (see F-02). | Sets an expectation ceiling: "compile-time" is out of reach for this specific failure mode; "fail at `build()`, not at first call" is the realistic ambition. | N/A — documenting a constraint, not a defect. | Frame Phase 2 goals around bootstrap-time validation, not compile-time. |

---

## C. Complexity Budget (measured, current state)

| Metric | Minimal service (no deps) | Service with DI (adapter + config) | Security/tenant-scoped service |
|---|---|---|---|
| Files needed | 1-2 | 1-2 (+ wherever adapters/config types live) | 1-2 (+ shared `make_proxy`-style helper hand-rolled per file today, F-06) |
| Manual trait impls | 1 (`{Trait}` for the impl struct) | 0 (macro generates `Injectable` for DI structs) | 1 (same as minimal — enforcement is macro-attribute-driven, not hand-implemented) |
| Registration steps that actually work today | **0** — no service-registration method exists on `RuntimeBuilder` (F-01) | 2 (`with_adapter`, `with_config` on `RuntimeBuilder`) | Same as minimal for the proxy; `with_tenant_enforcement_mode` / `with_security` for the runtime |
| Manual proxy-construction steps (today's actual substitute for registration) | 4 (`RuntimeBuilder::build()`, `Arc::downgrade`, `InterceptorChain::new()`, `{Trait}Ref::new(...)`) | same 4, plus resolving the struct via `Injectable::build` | same 4, unchanged by security/tenant attributes |
| Explicit types a developer must name | `Arc<dyn Trait>`, `Arc<InterceptorChain>`, `Weak<RuntimeInner>`, the generated `{Trait}Ref` | + `AdapterRef<A>`, `ConfigValue<C>` | + `TenantEnforcementMode` (if non-default) |
| Boilerplate LOC (hand-written, excl. business logic) | ~20-25 | ~25-30 | ~25-30 |
| Prerequisite concepts before first successful invocation | `{Trait}Tag`/`{Trait}Ref` naming convention; `ServiceContext` explicit-propagation model; `async_trait`; literal `Result<_, E>` (no aliases, macro-enforced); `Arc<dyn Trait>`; constructing an empty `InterceptorChain` even with nothing to intercept; `Weak<RuntimeInner>` and why it's `Weak` not `Arc`; **the fact that `ServiceRegistry`/`Resolvable` are documented but non-functional** | + `AdapterRef`/`ConfigValue` newtypes; `DepKey`; that `RuntimeBuilder::build()` won't tell you if you forgot one | + `TenantEnforcementMode` variants; the guard-ordering model (authorize → tenant → interceptors → body) |
| Production vs. TestKit divergence | For DI/`Injectable`: **none** (verified line-by-line — same `RuntimeInner::new_with_logger` constructor) | Same as above | For the trait-proxy/enforcement path: **not "divergent" — absent on both sides.** TestKit has no coverage because production has no working `resolve()` to mirror either. |

No target numbers are proposed here — per the audit's own constraint, targets belong to Phase 2, after this baseline.

---

## D. Internal Comparables (existing ergonomic patterns to emulate)

1. **`ServiceTestFixture`/`FixtureBuilder`** (`crates/testkit/src/fixtures.rs`) — the strongest model in the codebase. `::new()` needs zero configuration and is immediately usable; `.builder()` lets a caller override exactly one knob while everything else keeps a sane default; `.service::<S: Injectable>()` drives the *same* production `Injectable::build` path — proven by a hand-rolled-vs-macro comparison test, not just claimed. This is what "hard to misuse, easy to get right" looks like here.
2. **`RuntimeBuilder`** (`crates/service-sdk/src/runtime/builder.rs`) — a standard consuming builder: `::new()` needs no arguments, every `.with_*()` is optional, `.build()` is documented to always succeed for the pieces it currently validates. Good shape; the ergonomics gap is what it's missing (F-01/F-02), not its existing shape.
3. **`SdkAttr` enum** (`crates/service-sdk-macros/src/lib.rs:6-29`) — adding a new macro attribute requires exactly one new enum variant; detection and stripping both derive automatically from it. A clean example of low-ceremony extensibility inside the macro crate itself.

No external framework comparison was needed to identify the highest-leverage fix — the intended shape (registry → resolve → typed proxy) already exists as a design, and the comparable patterns above (especially `FixtureBuilder`) show the idiom this codebase already uses for "zero-config default, one-knob override, real underlying path" — the fix is completion, not invention.

---

## Audit questions — direct answers

1. **Canonical path today?** None that works as documented. The intended path (`ServiceRegistry`/`Resolvable`/`Runtime::resolve`) is unwired; the actual path everywhere is hand-rolled `{Trait}Ref::new(inner, chain, weak)`.
2. **Multiple overlapping paths?** Not duplicate *working* paths — one non-working documented path (registry/resolve) coexists with one working undocumented path (manual `Ref::new`). `builder.rs`/`runtime_builder.rs` naming is confusing but not duplicated API (F-10). Macro attributes have no ambiguous/redundant combinations (fail loudly by design, AD-007).
3. **Necessary vs. accidental boilerplate?** Necessary: `Weak<RuntimeInner>` (deliberate anti-cycle design — proxy must not keep the runtime alive). Accidental: the 4-step `RuntimeBuilder::build()` → `Arc::downgrade` → `InterceptorChain::new()` → `Ref::new()` sequence, hand-duplicated across ≥4 files with an identical constructor signature every time (F-06).
4. **`ServiceFactory` vs `Injectable`?** Not complementary — disjoint. `ServiceFactory` has zero implementations anywhere in the repo; only `Injectable` is real and macro-driven (F-04).
5. **Could registration derive from what the macro already knows?** Yes — `Injectable::dependencies()` is compile-time-complete today but never consulted by `RuntimeBuilder`/`Runtime` (F-02). The information needed for both registration validation and richer errors already exists; nothing reads it.
6. **Do production and TestKit build the same service the same way?** For the DI/`Injectable` struct path: yes, verified identical (same constructor, line-by-line). For the trait-proxy/enforcement path: there is no distinct production path to compare against — both are equally hand-rolled (F-07).
7. **When are missing deps detected?** Never at `build()` time (documented "always succeeds"); only when `Injectable::build()` is actually called — which today happens only in test fixtures, not in any production bootstrap.
8. **Do errors name the missing type and service?** No — `RuntimeError::DependencyNotFound` has no `Display`, no fields, Debug output is the bare variant name (F-03).
9. **Is `ServiceRef<T>` obtained naturally?** There is no generic `ServiceRef<T>` — each trait gets its own concrete `{Trait}Ref`, constructed by manually assembling 3 explicit arguments and calling `::new()`. Nothing is inferred or looked up by type (consequence of F-01).
10. **Is macro-generated code diagnosable?** Moderately — `expand_service_trait` is one straight-line ~440-line function using `quote!` close to final output, and `golden_codegen.rs` pins exact output via snapshot tests. No `cargo expand` artifact is checked in for a developer who doesn't want to read macro internals.
11. **What existing public API can simplify without a new conceptual layer?** Wire the already-fully-implemented `ServiceRegistry`/`Resolvable`/macro-generated `Resolvable` impls into `RuntimeBuilder`/`Runtime`'s public surface through a canonical registration/resolution API (F-01). This is completion of an existing, designed mechanism — not a new abstraction. The exact method name(s) and signature(s) are a Design-phase decision (see Open Questions below), not an Explore finding.
12. **Minimum slice with perceptible improvement?** Completing the registry→resolve→proxy wiring (F-01) plus giving `RuntimeError` a real `Display`/service-name payload (F-02/F-03) — these two alone would replace today's hand-rolled 4-step ceremony with a single registration call and a single resolution call, and turn a silent/late failure into an immediate, named one. F-04/F-05/F-06/F-07/F-08 become natural, low-risk follow-ons once F-01 gives them something real to attach to.

---

## Open Questions Carried to Design (explicitly NOT decided here)

**OQ-1 — What exactly does the registration API register, and what does it return?**

The audit confirms the *gap* (F-01) but deliberately does not decide the *shape*. Candidates observed as plausible during research, none chosen:

- `.with_service(Arc<MyServiceImpl>)` — infers the tag/trait from the concrete type.
- `.with_service::<MyTag>(Arc<dyn MyTrait>)` — explicit tag, matches how `Resolvable`/`ServiceRegistry` are already keyed internally (`(TypeId of Tag, ContractVersion)`).
- `.register(MyServiceImpl)` — a different verb, no tag parameter, relies entirely on type inference.
- `.register::<dyn MyTrait>(...)` — explicit trait-object-keyed registration.

This is not just a naming question — the choice affects:
- **Type safety**: whether a caller can accidentally register the wrong concrete type against a trait, or register the same trait twice.
- **Inference**: whether the caller must spell out a generic parameter or the compiler can infer it from the argument.
- **Versioning**: `ServiceContract`/`ContractVersion` already exist — does registration take a version explicitly, or derive it from the macro-generated descriptor?
- **`ServiceRegistry`'s existing contract**: `register`/`resolve_raw` already have a real signature (`registry/registry.rs:78-121`) keyed by `(TypeId, ContractVersion)` — any new public-facing method wraps this, and must not contradict or duplicate what `ServiceRegistry` itself already guarantees.

Design must compare at least the options above (and any others it finds) with explicit tradeoffs before a method name or signature is chosen. Explore/this document takes no position.

---

## Sub-agent research artifacts (full detail)

- `sdd/service-sdk-ergonomics/audit-journey1-minimal` — minimal service walkthrough
- `sdd/service-sdk-ergonomics/audit-journey2-di-errors` — DI + missing-dependency error path
- `sdd/service-sdk-ergonomics/audit-journey3-testkit-security` — TestKit vs. production, security/tenant journey
- `sdd/service-sdk-ergonomics/audit-comparables-docs` — internal comparables, examples, COOKBOOK.md drift
