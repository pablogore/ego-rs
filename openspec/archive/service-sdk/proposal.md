# Proposal: service-sdk

## Intent

Turn the structural scaffold on `feature/SPEC-008-service-sdk` into a behaviorally complete Service SDK: a type-keyed registry of live service implementations, a fully macro-generated `{TraitName}Ref` proxy, and a wired runtime that validates dependencies and constructs instances — so application services can declare contracts and invoke each other without touching transport, with interceptors and `ServiceContext` flowing transparently on every call.

## Problem

The current branch compiles and exposes the right vocabulary (`ServiceContext`, descriptors, interceptor chain, `ContractVersion`, `RuntimeBuilder`, `#[service]` on traits) but almost every behavioral requirement is a hollow stub. Concretely:

- **The registry stores descriptors, not implementations.** `ServiceRegistry` is `HashMap<String, ServiceDescriptor>` with no `register()`, no `resolve()`, no live `Arc<dyn Trait>` storage, and no enforcement of the `DuplicateService`/`DependencyCycle` variants that already exist in the error enum (FR-010–FR-013, FR-018).
- **There is no usable proxy.** `ServiceRef<T>` has the wrong shape — it is generic and unnamed, and its `invoke()` always returns `Err("not implemented")`. SPEC-008 requires a named `{TraitName}Ref` with typed methods (FR-015a–e). The proxy is the heart of the SDK and today it does nothing.
- **The runtime does not run.** `RuntimeBuilder::build()` returns `Ok(Runtime {})` against an empty struct — no factory collection, no dependency validation, no cycle detection, no instance construction (FR-014, FR-016, FR-018, FR-019, FR-020).
- **The lifecycle contract contradicts the spec.** The `Service` trait carries `initialize()`/`shutdown()` hooks, but FR-021a explicitly prohibits lifecycle hooks on application services.
- **Structural debt obscures the design.** Four parallel descriptor hierarchies, a duplicated `DomainError` trait, `serde` leaking onto the domain-layer `ServiceContext`, no `CancellationToken`, and interceptors coupled to the concrete `ServiceError` enum instead of a trait (FR-025d).
- **Tests validate shape, not behavior.** No test exercises register → resolve → invoke-via-proxy → interceptor fires → context propagates → error returns.

Net effect: the SDK looks complete in its type signatures but cannot host a single working service. Roughly 35–40% of SPEC-008 exists as structure; the load-bearing behavior is missing.

## Proposed Solution

Complete SPEC-008 end to end while consolidating the structural debt that hides the design. The change has five behavioral pillars and four cleanup tracks.

### Behavioral pillars

1. **Type-keyed live registry.** Replace `HashMap<String, ServiceDescriptor>` with type-keyed storage of live `Arc<dyn Trait>` implementations keyed by `(TypeId, ContractVersion)`. Implement `register()` with duplicate rejection, `resolve()` by type, and version-constraint resolution (semver range queries, not just exact equality).

2. **`{TraitName}Ref` proxy — 100% macro-generated.** Extend `#[service]` on traits to emit, in addition to the existing `ServiceContract` impl:
   - a concrete `{TraitName}Ref` struct wrapping `Arc<dyn TraitName>` plus its `InterceptorChain`;
   - an `impl TraitName for {TraitName}Ref` with one typed forwarding method per `#[operation]`;
   - transparent interceptor-chain execution (`on_request` → call → `on_response`/`on_error`) inside every forwarded method;
   - automatic `ServiceContext` scope propagation per invocation (FR-021o).
   No developer ever writes a proxy by hand. The generic, broken `ServiceRef<T>` is deleted.

3. **`#[service]` on structs + DI primitives.** Teach the macro to parse `ItemStruct`, detect dependency field types (`EntityRef<T>`, `ProjectionRef<P>`, `AdapterRef<A>`, config values), and emit DI metadata + a generated factory. Define `EntityRef<T>` by importing `entity_sdk::EntityRef` (canonical owner is CORE-006 entity-sdk — no local duplicate); define `ProjectionRef<P>` and `AdapterRef<A>` as injection primitives with runtime-scope context propagation.

4. **Runtime wiring.** Implement `RuntimeBuilder::build()`: collect factories from `with_entity`/`with_projection`/`with_service`/`with_service_bundle`, merge bundles, validate that every declared dependency is satisfiable, detect circular dependencies via topological sort (Kahn's algorithm), construct instances in dependency order, and return a live `Runtime` that can hand out resolved `{TraitName}Ref` proxies.

5. **Lifecycle contract split.** Remove `initialize()`/`shutdown()` from the application `Service` trait (FR-021a) and introduce a separate `LifecycleManaged` trait that only runtime-managed components (entities, projections, adapters) implement. The runtime drives lifecycle on `LifecycleManaged` components only.

### Cleanup tracks (done alongside the pillars, not after)

6. **Descriptor consolidation.** Designate `contract/descriptor.rs` as the single canonical descriptor set; delete the overlapping definitions in `contract/contract.rs`, `service/service.rs`, `operation/operation.rs`, and `version/version.rs`. Add the missing descriptor fields: `OperationDescriptor` gains idempotency + read-only/mutating flags (FR-022b); `FieldDescriptor` gains required/optional designation (FR-022c). Re-export `ContractDescriptor`/`FieldDescriptor` from `lib.rs`.

7. **`ServiceContext` hardening.** Remove `Serialize`/`Deserialize` from `ServiceContext` — serialization is a transport-adapter concern, not a domain concern. Add `cancellation_token: Option<tokio_util::sync::CancellationToken>` for push-style cancellation (FR-021w/x) alongside the existing deadline polling; add `tokio-util` to `Cargo.toml`.

8. **Error-trait compliance (FR-025d).** Define a `ServiceErrorTrait` (object-safe) that interceptors program against, instead of receiving the concrete `&ServiceError`. Delete the duplicated `DomainError` trait (keep one definition).

9. **End-to-end behavioral tests.** Add integration tests that exercise the full path: `register` → `resolve` (with version constraint) → invoke via generated `{TraitName}Ref` → interceptor fires → `ServiceContext` propagates across a service-to-service call → domain error returns through the trait. Plus cycle-detection, duplicate-rejection, and cross-tenant-enforcement tests.

### What stays

`ServiceContext`'s task-local `current()`/`scope()` machinery, the `Interceptor`/`InterceptorChain` design, `ContractVersion` semver, the transport-free contract guarantee, and the existing `#[service]`-on-trait descriptor generation all stay and are built upon.

## Scope

### In scope

- Type-keyed `ServiceRegistry` with live `Arc<dyn Trait>` storage, `register()`, `resolve()`, duplicate rejection, and version-constraint (semver range) resolution.
- `#[service]`-on-trait proxy codegen: `{TraitName}Ref` struct, `impl TraitName for {TraitName}Ref`, interceptor chain execution, per-call `ServiceContext` scope propagation.
- `#[service]`-on-struct support: field-type detection and DI metadata + factory generation.
- DI primitives: import `entity_sdk::EntityRef`; define `ProjectionRef<P>`, `AdapterRef<A>`, and config-value injection.
- `RuntimeBuilder::build()` wiring: factory collection, bundle merging, dependency validation, cycle detection (Kahn), ordered instance construction, live `Runtime`.
- `LifecycleManaged` trait; removal of `initialize()`/`shutdown()` from the application `Service` trait.
- Descriptor consolidation to one canonical set; missing field/flag additions; `lib.rs` re-exports.
- `ServiceContext`: remove `serde`, add `CancellationToken`; add `tokio-util` dependency.
- `ServiceErrorTrait` for interceptors (FR-025d); delete duplicated `DomainError`.
- Cross-tenant runtime enforcement (reject when `allow_cross_tenant` is false and tenant mismatches).
- End-to-end and edge-case integration tests (strict TDD: tests first).

### Out of scope (explicit non-goals)

- Field injection on arbitrary structs beyond the declared DI primitives (no general autowiring / `#[inject]` annotation).
- An advanced/general-purpose DI container.
- Reflection-like or runtime type-name string resolution.
- Entity graph injection (resolving an entity's transitive relationships).
- Complex lifecycle hooks beyond the simple `LifecycleManaged` start/stop contract.
- Transport adapters themselves (HTTP/gRPC/messaging) — only the descriptor-consumption integration *point* is in scope, not a concrete adapter.
- Defining `EntityRef<T>` here — it is owned by CORE-006 entity-sdk and only imported.

## Key Design Decisions

1. **Registry is type-keyed, not string-keyed.** Keying on `(TypeId, ContractVersion)` gives compile-relevant identity and removes stringly-typed lookups. Rationale: FR-001/FR-010–FR-013 demand type-safe resolution; string keys cannot express version-range queries safely.

2. **`{TraitName}Ref` is 100% macro-generated; `ServiceRef<T>` is deleted.** A named per-service proxy with typed methods is the only shape that satisfies FR-015a–e and keeps callers transport-free. Rationale: the generic `ServiceRef<T>` cannot expose typed operations and its erased `invoke(&str, &[u8])` reintroduces a transport-like seam. Generating the proxy guarantees interceptors and context propagation can never be forgotten by a developer.

3. **`EntityRef<T>` is imported from entity-sdk, never duplicated.** Canonical ownership lives in CORE-006. Rationale: a duplicate type would fork the entity contract and break interop. If CORE-006 is not yet available, take a temporary dependency on the correct crate rather than defining a local copy.

4. **Lifecycle is split: application services have none.** `initialize()`/`shutdown()` move off `Service` onto a `LifecycleManaged` trait for runtime-managed components only. Rationale: FR-021a prohibits lifecycle hooks on application services; mixing them invites stateful services and ordering bugs.

5. **Cycle detection via Kahn's topological sort at build time.** Fail fast before any instance is constructed. Rationale: FR-018/FR-020 require pre-start validation; topological sort gives a deterministic, explainable failure naming the cycle.

6. **One canonical descriptor set; delete the other three hierarchies.** Rationale: four overlapping definitions guarantee drift and make FR-022 compliance unverifiable. Consolidation is a prerequisite, not a follow-up.

7. **`ServiceContext` stays in the domain layer with no `serde`; `CancellationToken` added.** Serialization belongs to transport adapters; cancellation needs push-style propagation, not just deadline polling. Rationale: FR-003 (zero transport coupling) and FR-021w/x.

8. **Interceptors program against `ServiceErrorTrait`, not `ServiceError`.** Rationale: FR-025d — interceptors must operate on the trait so they remain decoupled from any concrete error enum, and domain errors flow through unchanged.

9. **Cross-tenant enforcement is the Runtime's sole responsibility.** The `ServiceRuntime` is the single enforcement point for tenant isolation. Generated `{TraitName}Ref` proxies MAY perform defensive pre-call validation (tenant_id present, context valid) but are never the sole barrier. No service invocation path may bypass runtime tenant validation. Rationale: the runtime is the only component that simultaneously knows the current tenant scope, registry, entity ownership, and isolation boundaries — the proxy can be bypassed via `registry.get()` or direct resolution. Centralizing enforcement also provides a single audit point across all invocation paths: generated proxies, internal calls, tests, and future transport adapters.

## Affected Components

- `crates/service-sdk/Cargo.toml` — add `tokio-util`; add/confirm `entity-sdk` (CORE-006) dependency.
- `crates/service-sdk/src/registry/registry.rs` — full redesign (type-keyed live storage, `register`/`resolve`, version constraints).
- `crates/service-sdk/src/runtime/runtime_builder.rs` — implement `build()` (validation, cycle detection, construction); real `Runtime` state.
- `crates/service-sdk/src/reference.rs` — delete `ServiceRef<T>`; the generated `{TraitName}Ref` replaces it (and any shared `ServiceReference` plumbing the proxy needs).
- `crates/service-sdk/src/context/mod.rs` — remove `serde`; add `cancellation_token`; cross-tenant enforcement helper.
- `crates/service-sdk/src/implementation.rs` — remove `initialize`/`shutdown` from `Service`; add `LifecycleManaged` trait.
- `crates/service-sdk/src/contract/` — consolidate to `descriptor.rs`; delete `contract.rs`, `service/service.rs`, `operation/operation.rs`, `version/version.rs`; add idempotency/read-only flags and required/optional field designation.
- `crates/service-sdk/src/error/` — define `ServiceErrorTrait`; delete duplicated `DomainError` (keep one of `domain_error.rs` / `category.rs`).
- `crates/service-sdk/src/interceptor/` — interceptors receive `&dyn ServiceErrorTrait`.
- `crates/service-sdk/src/lib.rs` — re-export `ContractDescriptor`/`FieldDescriptor`; update module tree for deletions; add DI primitive re-exports.
- New DI primitives module — `ProjectionRef<P>`, `AdapterRef<A>`, config injection.
- `crates/service-sdk-macros/src/lib.rs` — extend `#[service]` for proxy generation (trait side) and struct side (DI metadata + factory).
- `crates/service-sdk/tests/` — new end-to-end and edge-case integration tests.
- `crates/service-sdk/examples/order_service.rs` — update to use real macros/proxy once generated.

## Risks

- **CORE-006 entity-sdk availability.** If `EntityRef<T>` is not yet published in entity-sdk, the temporary-dependency fallback must point at the correct crate and be reversible. Risk: accidental local duplicate that later forks the contract. Mitigation: import-only policy, called out in code review.
- **Proxy codegen complexity.** Generating `impl TraitName for {TraitName}Ref` with `async_trait` methods, generic args, and interceptor wrapping is the hardest part of the macro. Risk: trait-object dispatch + `async_trait` interaction edge cases. Mitigation: TDD with macro expansion tests (`trybuild`/expansion snapshots) before wiring runtime.
- **Breaking changes.** Removing `serde` from `ServiceContext`, deleting `ServiceRef<T>`, and removing `initialize`/`shutdown` are source-breaking. Risk: downstream crates in the workspace break. Mitigation: workspace-wide `cargo test --workspace` is the gate; fix call sites as part of this change.
- **Descriptor consolidation churn.** Deleting three hierarchies touches many re-exports. Risk: wide compile breakage mid-change. Mitigation: do consolidation early and keep the workspace green incrementally.
- **95% coverage threshold under strict TDD.** Macro-generated code and error branches are hard to cover. Risk: `cargo tarpaulin --fail-under 95` blocks. Mitigation: design generated code to be exercised by behavioral tests; cover error/cycle/duplicate/cross-tenant paths explicitly.
- **Cross-tenant enforcement semantics.** The exact rejection rule (where it fires, what error) must match the spec precisely. ✅ Resolved: enforcement is in the Runtime (sole authority); proxy performs defensive validation only (see Key Design Decision #9).

## Success Criteria

- A service contract declared with `#[service]` on a trait produces a working `{TraitName}Ref` whose typed methods forward to the live implementation, run the interceptor chain, and propagate `ServiceContext` — with zero hand-written proxy code.
- `ServiceRegistry::register()` rejects duplicates; `resolve()` returns the correct implementation for an exact version and for a satisfied semver range, and errors for an unsatisfied one.
- `RuntimeBuilder::build()` validates dependencies, rejects a circular-dependency graph with a named cycle, constructs instances in order, and returns a `Runtime` that yields resolved proxies.
- `#[service]` on a struct compiles, detects `EntityRef`/`ProjectionRef`/`AdapterRef`/config fields, and produces a usable factory.
- The application `Service` trait has no `initialize`/`shutdown`; `LifecycleManaged` exists and is driven only for runtime-managed components.
- Exactly one descriptor hierarchy remains; `EntityRef<T>` is imported, not defined; `ServiceContext` has no `serde` and has a `CancellationToken`; interceptors take `&dyn ServiceErrorTrait`.
- An end-to-end integration test passes the full path register → resolve → invoke-via-proxy → interceptor → context propagation → domain error.
- `cargo test --workspace` is green and `cargo tarpaulin --workspace --fail-under 95` passes.

## Dependencies

- **CORE-006 entity-sdk** — canonical owner of `EntityRef<T>`. Required import; temporary correct-crate dependency allowed if not yet available, never a local duplicate.
- **`tokio-util`** — new crate dependency for `CancellationToken`.
- **`async-trait`** — already present; relied on by generated proxy methods.
- Existing internal pieces reused: `ServiceContext` task-local machinery, `Interceptor`/`InterceptorChain`, `ContractVersion`.
