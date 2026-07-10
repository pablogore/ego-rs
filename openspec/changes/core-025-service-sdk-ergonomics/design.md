# Design: CORE-025 — Service SDK Developer Ergonomics

Wire the **already-built-but-unconnected** `ServiceRegistry` / `Resolvable` / `ResolvableContainer` machinery into the public `RuntimeBuilder` / `Runtime` surface by adding methods directly to those existing types — no new wrapper, no new builder, no parallel DI container. OQ-1 is resolved in favour of an **explicit-tag, trait-object registration** (`builder.with_service::<Tag>(Arc<dyn Trait>)`) paired with an **explicit-tag resolution** (`runtime.resolve::<Tag>()`), because that is the only shape that matches how `ServiceRegistry` is *already* keyed (`(TypeId<Tag>, ContractVersion)`) and what `Resolvable::create_proxy` *already* expects to downcast (`ResolvableContainer<dyn Trait>`). Everything else in the slice (fail-fast validation, richer errors, the TestKit helper) hangs off that decision.

This document is the HOW at architecture level. It resolves OQ-1, records the ADRs, and states exactly what the macro codegen must change. It does not enumerate tasks.

---

## Quick path (the target developer journey this design produces)

**Minimal / security / tenant service (impl constructable before `build()`):**

```rust
let rt = RuntimeBuilder::new()
    .with_service::<HelloServiceTag>(Arc::new(HelloServiceImpl) as Arc<dyn HelloService>)?
    .build();

let hello = rt.resolve::<HelloServiceTag>()?;      // -> HelloServiceRef, fully wired
let out = hello.greet(ServiceContext::new(), "world".into()).await?;
```

`resolve` internally does the four hand-rolled steps (clone the runtime's interceptor chain, `Arc::downgrade` the inner, call the macro-generated `create_proxy`, hand back the concrete `{Trait}Ref`). The proxy is **byte-for-byte the same** `{Trait}Ref` the hand path builds today — so the guard order (authorize → tenant → interceptors → body) is preserved because it is the *same generated proxy*, not a new code path.

**DI service (`Injectable` struct) — fail-fast on missing deps:**

```rust
let rt = RuntimeBuilder::new()
    .with_config(Arc::new(10u32))
    .with_injectable::<MyService>()        // records MyService::validate — a pure dependencies() presence check
    .try_build()?;                         // fails HERE naming the missing type + MyService — never constructs MyService
let svc = MyService::build(rt.inner())?;   // deps now guaranteed present; build() runs once, for real, here
```

`build()` stays infallible and unchanged (CORE-018b requirement honoured); `try_build()` is the new fail-fast terminal. Validation goes through a dedicated `Injectable::validate()` (AD-3 / OQ-2), **not** through calling-and-discarding `build()` — so `build()` runs exactly once, when the caller actually wants the instance.

---

## What each finding maps to

| Finding | Mechanism | Surface touched | Macro codegen change? |
|---|---|---|---|
| F-01 registration | `RuntimeBuilder::with_service::<Tag>(Arc<Tag::Service>) -> Result<Self, RegistryError>` | `runtime/builder.rs` | **Yes** — add `type Service = dyn Trait;` to the generated `Resolvable` impl |
| F-01 resolution | `Runtime::resolve::<Tag>() -> Result<Tag::Proxy, RuntimeError>` | `runtime/builder.rs` | No |
| F-02 fail-fast | `RuntimeBuilder::with_injectable::<S>()` + `RuntimeBuilder::try_build() -> Result<Runtime, RuntimeError>`, validating via a new defaulted `Injectable::validate()` (never `build()`) | `runtime/builder.rs`, `di/mod.rs` | **Yes — minimal:** `dependencies()`' `DepKey` gains a `&'static str` type name (macro `classify_field_type`, +1 arg × 3 sites). `validate()` is a **generic default** — zero per-service codegen. See AD-3 / OQ-2 |
| F-03 diagnostic error | `RuntimeError::DependencyNotFound { type_name, service_name }` + `Display` + `impl std::error::Error` | `runtime/runtime_builder.rs` | **Yes** — `create_proxy`'s downcast-failure arm re-points to `ServiceNotFound` |
| F-06/F-07 TestKit | `FixtureBuilder::with_service::<Tag>(..)` + `ServiceTestFixture::resolve::<Tag>()` pass-throughs | `crates/testkit/src/fixtures.rs` | No |
| F-09 example | New minimal end-to-end example using the above | `crates/service-sdk/examples/` | No |

---

## OQ-1 resolved — the four candidate shapes compared

All four target the same outcome: one registration call, one resolution call, reusing the existing registry/resolvable machinery. The registry is **immovably** keyed by `(TypeId<Tag>, ContractVersion)` and `create_proxy` **immovably** downcasts `Arc<dyn Any>` back to `Arc<ResolvableContainer<dyn Trait>>` (`runtime/resolvable.rs:26-56`, macro `lib.rs:474-483`). Those two facts, not aesthetics, decide the winner.

### The hard constraint that eliminates half the field

To store an entry the existing `create_proxy` can retrieve, registration **must** produce `Arc<ResolvableContainer<dyn Trait>>`. Building that requires an `Arc<dyn Trait>` in hand. Two consequences:

1. **You cannot infer the tag from a concrete impl** (`Arc<HelloServiceImpl>`). The `#[service]` macro runs on the *trait*, never on the user's impl struct, so there is no `Impl → Tag` type link, and the macro deliberately avoids a blanket `impl<T: Trait> ...` (orphan-rule note, `lib.rs:486`). Candidate `.register(Impl)` / `.with_service(Arc<Impl>)` is **not implementable on stable** without either a second macro on every impl or an unstable `Unsize` bound.
2. **You cannot coerce `Arc<S> → Arc<dyn Trait>` inside a function generic over the tag**, because the trait can only be named there as the associated type `Tag::Service`, and unsize-coercion to an associated-type projection is not automatic on stable. So the caller must coerce at their call site (they already write `let inner: Arc<dyn HelloService> = Arc::new(HelloServiceImpl)` today — no new burden).

| Shape | Concrete signature | Compatibility | Compilation impact | Error-message impact | Duplication risk | Verdict |
|---|---|---|---|---|---|---|
| **1. Improve existing APIs directly** (CHOSEN) | `RuntimeBuilder::with_service::<Tag>(self, Arc<Tag::Service>) -> Result<Self, RegistryError>`; `Runtime::resolve::<Tag>() -> Result<Tag::Proxy, RuntimeError>` | Fully additive; `{Trait}Ref::new` untouched. One additive assoc type on `Resolvable` (`type Service`) — additive to the generated impl | Adds `type Service = dyn Trait` to the generated `Resolvable` impl; resolution needs zero codegen change. Stable Rust throughout | Version derived from `ServiceContract::version()`; duplicate `(Tag, version)` rejected by the *existing* `register` (`RegistryError::DuplicateService`) | **None** — literally calls existing `registry.register`/`resolve_raw` + `create_proxy` | **Chosen.** Smallest honest surface, matches existing keying exactly, no new concept |
| **2. Thin facade** (new wrapper type over builder/runtime) | e.g. `ServiceHost::register::<Tag>(..)` wrapping `RuntimeBuilder` | Additive but introduces a *second* front door beside `RuntimeBuilder` | Same codegen need as #1, plus a whole new type + its lifecycle | Same as #1 | **High** — a wrapper that only forwards to `RuntimeBuilder` is exactly the "abstraction to save two lines" Principle #10 forbids; the audit found `RuntimeBuilder`'s *shape* is already good (Section D.2) | **Rejected.** No evidence in the audit demands a new type; it would duplicate the builder's role and split the canonical path in two |
| **3. Expand existing macros** (`#[service]` emits registration glue) | Macro generates e.g. `HelloServiceTag::register_on(builder, impl)` | Additive but grows generated surface and the snapshot contract for *every* service | Larger `golden_codegen`/`proxy_codegen` churn; every service now emits a registration helper whether used or not | Same as #1 | Medium — pushes registration logic into codegen that a single generic builder method covers once, centrally | **Rejected.** Violates "macros reduce repetition, never add surface for its own sake" (Principle #8); one generic method beats N generated helpers. (We still make **one minimal** codegen addition — `type Service` — but that is a type link, not registration logic) |
| **4. Additional builder** (dedicated service-registration builder) | `ServiceRegistryBuilder::new()....build_into(runtime_builder)` | Additive but a third builder alongside `RuntimeBuilder` + `FixtureBuilder` | Same codegen need as #1, plus a new builder type and the merge step into `RuntimeBuilder` | Same as #1 | High — the registry already lives *inside* the runtime the `RuntimeBuilder` produces; a separate builder re-creates a boundary that already exists | **Rejected.** Registration is one method on the builder we already have; a separate builder is ceremony with no payoff the audit identified |

**Sub-decisions inside the chosen shape (the rest of OQ-1):**

- **What gets registered:** an `Arc<dyn Trait>` (the trait object), wrapped internally into `ResolvableContainer<dyn Trait>` and stored as `Arc<dyn Any + Send + Sync>` via the existing `ServiceRegistry::register::<Tag>`. Nothing new is stored; this is exactly the container `create_proxy` already downcasts to.
- **What identifies it:** the explicit `Tag` turbofish. This is the honest match for a `TypeId<Tag>`-keyed registry and makes "wrong concrete type against this trait" a compile error, and "same trait twice" a returned `RegistryError::DuplicateService`.
- **What it returns:** `with_service` returns `Result<Self, RegistryError>` (duplicate detection is real and already implemented — surface it, do not swallow it). `resolve` returns `Result<Tag::Proxy, RuntimeError>` — the concrete `{Trait}Ref`.
- **Versioning:** registration derives the version from `<Tag as ServiceContract>::version()` (the macro-generated descriptor); resolution queries with `VersionConstraint::Exact(<Tag>::version())`. The ergonomic path is single-version by construction. Multi-version / semver-range registration stays available through the lower-level `ServiceRegistry::register` / `resolve_raw` (which already support ranges) — we do **not** build a multi-version ergonomic API nobody asked for.

### The one required codegen change for F-01

Add a single associated type to the generated `Resolvable` impl (`lib.rs` ~471):

```rust
impl ego_service_sdk::runtime::Resolvable for #tag_name {
    type Proxy = #ref_name;
    type Service = dyn #trait_name;          // <-- NEW: the Tag → Trait type link
    fn create_proxy(...) -> ... { /* unchanged */ }
}
```

and the trait definition (`runtime/resolvable.rs`):

```rust
pub trait Resolvable: ServiceContract {
    type Proxy: Send + Sync;
    type Service: ?Sized + Send + Sync + 'static;   // <-- NEW
    fn create_proxy(...) -> Result<Self::Proxy, RuntimeError>;
}
```

**Scope note on `type Service`:** this associated type exists solely so `with_service::<Tag>(Arc<Tag::Service>)` can name the trait object it accepts — a minimal type-level link between a `Tag` and the trait it fronts, nothing more. `Resolvable` is not becoming a general service descriptor: adding further associated metadata to this trait (`Name`, `Lifetime`, `Scope`, or similar) is explicitly out of scope for this reason, not merely unaddressed. If a future change needs to describe more about a service, that belongs on `ServiceContract` (which already owns descriptive metadata via `descriptor()`/`version()`) or a new trait — not accreted onto `Resolvable`, whose only job is producing a proxy from a registry entry.

Then `with_service` is stable and generic:

```rust
pub fn with_service<Tag>(mut self, svc: Arc<Tag::Service>) -> Result<Self, RegistryError>
where
    Tag: Resolvable + 'static,
{
    let raw: Arc<dyn Any + Send + Sync> = Arc::new(ResolvableContainer(svc));
    self.registry.register::<Tag>(<Tag as ServiceContract>::version(), raw)?;
    Ok(self)
}
```

`ResolvableContainer<dyn Trait>` is a concrete `Sized + 'static + Send + Sync` type, so it coerces to `Arc<dyn Any + Send + Sync>` on stable — exactly the value `create_proxy` downcasts back.

**Snapshot impact (flag for tasks phase):** `golden_codegen.rs` snapshots only `ServiceContract::descriptor()`, which is unchanged, so the golden snapshots stay green. `proxy_codegen.rs` is a compile+run test of generated code; adding `type Service` is additive and keeps it compiling. Confirm both remain green when the codegen lands; no snapshot *content* should change from the `type Service` addition alone.

---

## OQ-2 resolved — how `try_build()` validates dependencies: three options compared

**The objection this resolves (verified valid):** the earlier draft of AD-3 validated by calling `S::build()` and discarding the instance. Nothing in the `Injectable` contract requires `build()` to be side-effect-free, so a future `build()` that opens a socket, warms a cache, or subscribes to events would have that work silently triggered-and-discarded on **every** `try_build()`, with no compiler or contract signal. `build()` was thereby overloaded with two responsibilities — *construct* and (as a side effect of being called and thrown away) *validate* — not separated in the type system.

Grounding facts (verified in code, not assumed):

- `DepKey` is `Entity(TypeId) | Projection(TypeId) | Adapter(TypeId) | Config(TypeId)` — a **bare, opaque `TypeId` per variant** (`di/mod.rs:76-85`). `std::any::TypeId` has **no** reverse to a type name in std.
- The runtime's resolved tables are `HashMap<TypeId, Arc<dyn Any + Send + Sync>>` keyed by `TypeId` (`runtime_builder.rs:43-47`). Presence-by-`TypeId` (`contains_key`) is **semantically equivalent** to the `resolve_adapter::<A>()` path for validation: two distinct types can never share a `TypeId`, so the downcast in `resolve_*` never fails when the `get` succeeds (`runtime_builder.rs:79-93`).
- `dependencies()` is already macro-generated as `DepKey::Adapter(TypeId::of::<#inner_ty>())` (`service-sdk-macros/lib.rs:605-621`) — the concrete type is in hand at codegen.
- Dependencies are **strictly flat**: a service depends on N leaf adapters/configs/projections; leaves are plain `Arc<dyn Any>` values that are not `Injectable` and declare no dependencies. There are **no** service-depends-on-service edges and **no** transitive chains anywhere in the workspace.

| Option | Benefits | Cost / codegen | Compatibility | Compilation impact | Error message (F-03: name missing type + requesting service) | Duplicates an existing mechanism? | Verdict |
|---|---|---|---|---|---|---|---|
| **A — Trial construction** (call `S::build()`, discard) | Zero new validation code; reuses `build()` verbatim | None — no `DepKey` change, no macro change, no snapshot change | Additive | Nothing new to compile | **Free** — the generic `resolve_adapter::<A>()` yields `type_name::<A>()` on the way out (AD-4) | Reuses real resolution semantics (no parallel walk) | **Rejected.** Only option that leaves the objection unresolved: `build()` stays overloaded, and any future side-effecting `build()` is silently run-and-discarded on every `try_build()`, with no type-system separation of construct vs. validate |
| **B — Dedicated `Injectable::validate()`** (CHOSEN) | Construct and validate become **distinct methods** in the type system; validation is a pure, side-effect-free presence check that **never constructs** the service | A **generic default** `validate()` on `Injectable` (written once, zero per-service codegen) + a one-time `RuntimeInner::check_dependency(&DepKey)` presence helper. **To keep F-03 (naming) without constructing:** `DepKey` gains a `&'static str` type name (macro `classify_field_type`, +1 arg × 3 sites) — because a bare `TypeId` cannot be named | Additive at the public API; `DepKey`'s variant shape changes (public type, but `dependencies()` has zero real readers per audit) | `validate()` default compiles once; `DepKey` change is mechanical across 4 sites; **one golden snapshot regenerates** (see fallout) | **Preserved** — the enriched `DepKey` carries the missing type's name; `service_name` attached by the `try_build` validator, exactly as AD-4 specifies | New tiny `check_dependency` dispatch (per-kind `contains_key`); no new DI container | **Chosen.** Resolves the objection at the level it lives (the trait contract), keeps validation generic (Principle #8: one generic method beats N generated helpers), and — this is the load-bearing reason, not the self-describing-model framing an earlier draft leaned on — `try_build()`'s fail-fast error IS Scenario 3, this slice's star scenario. Shipping it with a service-name-only error while the resolution path (AD-4) gets a fully-named one would ship the flagship feature of this change with visibly worse diagnostics than the path nobody is asking to improve |
| **C — Dependency-graph validator** (`DependencyValidator` reasoning over transitive/cross-service edges) | Would future-proof nested/transitive DI | A whole new subsystem: graph model, edge discovery, cycle detection | Additive | Substantial new code | Could name types if the graph nodes carry them | **Solves a problem that does not exist** — the model is strictly flat today (no dep-on-dep edges, no service-as-dep) | **Rejected (YAGNI / Principle #10).** There is no graph to validate. Building one is inventing complexity the codebase has zero evidence of needing. **Future trigger, not now:** revisit only if `Injectable`s ever gain other `Injectable`s as dependencies (nested DI), which would introduce the first real edges |

**Sub-variants of Option B considered (and why B lands where it does):**

- **B1 — pure default, no `DepKey` change.** A generic `validate()` walking `dependencies()`' `TypeId`s and checking `contains_key`. Achieves the side-effect-free goal with *zero* codegen. **Reconsidered under independent review** (see below): `dependencies()` genuinely has zero non-test readers today, so enriching `DepKey` buys nothing *in the abstract* — the only place the added name is ever read is `try_build()`'s own error path. But that path is not a minor corner: it *is* Scenario 3, the scenario this slice exists to deliver. Under B1, `try_build()`'s fail-fast error would name the requesting service but not the missing type — while `resolve_adapter`/`resolve_config`'s resolution-path errors (AD-4) keep full type names via `type_name::<A>()`, unaffected either way. B1 would therefore ship two error qualities for the same `DependencyNotFound` variant depending on which path produced it, with the *worse* one on the feature this change is about. **Rejected on that concrete basis**, not on an abstract "self-describing model" argument — the earlier draft of this document offered that abstract framing and independent review correctly flagged it as unsupported by any real reader of `dependencies()`; the framing is corrected here, the decision is not.
- **B2 — default `validate()` + minimally enriched `DepKey` (CHOSEN flavor).** Adds the type name the macro already knows to `DepKey`, so the pure presence walk names the missing type *and* keeps `validate()` generic (one impl, no per-service codegen). Cost is fully mechanical (one macro arg × 3 codegen arms, 4 construction/match sites, one golden-snapshot regen — see AD-3 fallout) and buys parity between the fail-fast and resolution error paths.
- **B3 — macro-generated per-service `validate()`.** Generate a `validate()` body per service mirroring `build()` minus struct assembly; names types free via the existing `resolve_adapter::<A>()`; no `DepKey` change. **Rejected:** it regenerates, per service, information `dependencies()` already encodes — contradicting the very Principle #8 ("one generic method beats N generated helpers") this design used to reject macro-emitted glue in OQ-1 Option 3.

**Honest note on the tradeoff:** Option A is genuinely the smallest diff and gets F-03 for free. The objection is nonetheless real and the codebase's own principles (explicit contracts, generic-over-generated) favour separating validate from construct at the trait level. B2 pays a small, fully mechanical cost (one macro arg × 3 sites, a public `DepKey` shape change with no other readers, one golden-snapshot regen) so that `try_build()` — this slice's flagship fail-fast path — gets the same diagnostic quality as the resolution path already has. That is a proportionate, narrowly-justified trade, not gold-plating for its own sake.

---

## Data flow

```
                 with_service::<Tag>(Arc<dyn Trait>)                resolve::<Tag>()
                          |                                              |
   Arc<dyn Trait> --wrap--> ResolvableContainer<dyn Trait>        registry.resolve_raw::<Tag>(Exact(version))
                          |  as Arc<dyn Any>                            |  -> Result<Arc<dyn Any>, RegistryError>
                          v                                            v  (mapped: RegistryError::ServiceNotFound -> RuntimeError::ServiceNotFound)
   RuntimeBuilder.registry.register::<Tag>(version, raw)        Tag::create_proxy(raw, chain, weak)
                          |                                            |  downcast to ResolvableContainer<dyn Trait>
                          v                                            v
                RuntimeInner.registry (built)  <-- Arc::downgrade --  {Trait}Ref { inner, chain, runtime }
```

- `resolve_raw` returns `Result<_, RegistryError>`, not `RuntimeError` — `resolve::<Tag>()` must map `RegistryError::ServiceNotFound` (and any other `RegistryError` variant reached this way) to `RuntimeError::ServiceNotFound` before calling `create_proxy`. Trivial (the registry field is `pub(crate)`), but an explicit step tasks must implement, not just wire through.
- `chain` = the runtime's single `interceptor_chain` (`RuntimeInner.interceptor_chain`, `#[allow(dead_code)]` today — the attribute is already slightly stale since the manual `Debug` impl reads it (`runtime_builder.rs:143`); resolution is its first *functional* reader, closing the substantive dead-code concern even if the attribute itself predates this by one reader).
- `weak` = `Arc::downgrade(&self.inner)` — the deliberate anti-cycle handle (audit Q3). The proxy never keeps the runtime alive; identical to the hand path.

---

## ADRs

### AD-1 — Complete existing APIs in place; no new type, no new builder, no parallel container
**Decision:** Add `with_service` / `try_build` / `with_injectable` to `RuntimeBuilder` and `resolve` to `Runtime`; wrap the existing `ServiceRegistry` / `Resolvable` / `ResolvableContainer` / `Injectable` machinery.
**Rationale:** The audit's core reframe (Executive Summary; Q11) is that the mechanism was *built but never wired*. The fix is completion, not invention. `RuntimeBuilder`'s shape is already the codebase's endorsed idiom (Section D.2).
**Rejected:** thin facade (#2), macro-emitted glue (#3), separate builder (#4) — each adds surface the audit did not justify and risks a second canonical path (Principles #6, #8, #10).

### AD-2 — OQ-1: explicit-tag turbofish + trait-object argument; version derived from the descriptor
**Decision:** `with_service::<Tag>(Arc<Tag::Service>)`; `resolve::<Tag>() -> Tag::Proxy`; version comes from `ServiceContract::version()`.
**Rationale:** The registry is `(TypeId<Tag>, ContractVersion)`-keyed and `create_proxy` downcasts to `ResolvableContainer<dyn Trait>`. Tag-inference-from-impl is unimplementable on stable (no `Impl → Tag` link, macro runs on the trait, no blanket impl). Explicit tag makes mis-registration a compile error and duplicate-registration a returned `RegistryError`.
**Rejected:** `.with_service(Arc<Impl>)` / `.register(Impl)` (needs a second macro or unstable `Unsize`); explicit-version parameter (redundant with the macro descriptor for the single-version ergonomic path).
**Cost:** one additive associated type (`type Service`) on the generated `Resolvable` impl — a macro codegen change (see above).

### AD-3 — F-02 fail-fast by a dedicated `Injectable::validate()` presence check at `try_build()` (not by trial-constructing `build()`)
**Decision:** Add a **defaulted, generic** method to `Injectable`:
```rust
pub trait Injectable: Send + Sync {
    fn dependencies() -> Vec<DepKey> where Self: Sized;

    /// Presence-only dependency check. Constructs nothing. Default is fully
    /// generic over dependencies() — no per-service codegen.
    fn validate(rt: &RuntimeInner) -> Result<(), RuntimeError> where Self: Sized {
        for dep in Self::dependencies() {
            rt.check_dependency(&dep)?;   // per-kind contains_key; names the missing type from the DepKey
        }
        Ok(())
    }

    fn build(rt: &RuntimeInner) -> Result<Self, RuntimeError> where Self: Sized;   // unchanged
}
```
`with_injectable::<S: Injectable>()` records a monomorphic `fn(&RuntimeInner) -> Result<(), RuntimeError>` = `S::validate`. `try_build()` calls the existing infallible `build()`, then runs every recorded `validate` against `rt.inner()`, returning the first failure with the requesting service name attached (`service_name: Some(type_name::<S>())`). `build()` is untouched, and **`Injectable::build` is never invoked during validation.**

**Normative:** `with_injectable()` records validation metadata only — a `(name, fn)` pair pushed onto the builder's own bookkeeping. **It never changes the `Runtime` `build()` produces**, and it has no effect at all unless the caller later calls `try_build()` instead of `build()`. Reading the public API in isolation, `with_injectable(..).build()` and `with_injectable(..).try_build()` produce runtimes with identical inspectable state (same adapters, same config, same registry) — the only difference is whether `try_build()`'s extra validation pass ran before returning. This is stated explicitly so `with_injectable` is not mistaken for something that configures the `Runtime` itself, rather than something that configures `try_build()`'s behavior.

**Rationale (why B over A — see OQ-2 for the full three-way comparison):** validation and construction become **distinct methods in the type system**, so no `build()` side effect can be silently triggered-and-discarded by `try_build()`. `check_dependency(&DepKey)` does a per-kind `contains_key` on `RuntimeInner`'s resolved tables — semantically equivalent to the `resolve_*` path (a `TypeId` uniquely identifies a type, so `contains_key` ⇔ a successful `resolve_*` downcast) but **without constructing anything**. Scope is Adapter + Config + Projection presence (Projection tables are always empty from `with_registrations`, so a projection-dependent service correctly fails — matching what `build()` resolution would do); `DepKey::Entity` remains unvalidatable this slice (there is no entity table and no entity resolver — the *same* blind spot Option A's `build()` has, flagged below, not a regression introduced by B). **`check_dependency`'s `Entity` arm MUST return `Err` (treat as always-missing), not `Ok`** — the safe default given no entity table exists to check against; a service that (today, only in a test) declares an `Entity` dependency must not silently pass validation. No duplicate DI container (Principle #6).

**Naming the missing type without constructing (the one real cost):** a bare `TypeId` cannot be reversed to a name in std, so the presence walk cannot satisfy F-03 on its own. `DepKey` is therefore **minimally enriched** to carry the `&'static str` the macro already has in hand at codegen (`DepKey::Adapter(TypeId::of::<X>(), type_name::<X>())`). This makes the dependency model self-describing and keeps `validate()` generic (one impl, zero per-service codegen).

**Why not change `build()` to return `Result` / be called for validation:** `build()` is called across tests, examples, and the CORE-016 host bootstrap; changing its signature is a broad breaking change. Calling-and-discarding it (Option A) overloads it with validation and carries the side-effect footgun. The CORE-018b requirement "`RuntimeBuilder::build()` Behavior Is Unchanged" stays literally true; `build()` now runs exactly once, when the caller wants the instance.

**Fallout (flag for tasks — all mechanical):**
- `DepKey` variants gain a `&'static str` type-name field (public type shape change; `dependencies()` has zero non-test readers per the audit, so blast radius is small).
- Macro `classify_field_type` (`service-sdk-macros/lib.rs:605-621`): add `std::any::type_name::<#inner_ty>()` beside the existing `TypeId::of::<#inner_ty>()` at the three arms (Projection/Adapter/Config).
- Construction/match sites (independent review confirmed the line ranges below are right but the prose undercounted siblings — every variant at each cited site needs the same edit, not just the one named): `di/mod.rs`'s `di_primitives_are_recognizable` test constructs **all four** `DepKey` variants (`Entity`/`Projection`/`Adapter`/`Config`, ~lines 108-111) — all four need the name arg added, not just `Entity`; the **hand-rolled** `Injectable` in `testkit/src/fixtures.rs:205` (`DepKey::Config(TypeId::of::<u32>())` → add the name arg); `proxy_codegen.rs:254-255` has **two** `matches!` arms (`DepKey::Projection(_)` at 254, `DepKey::Adapter(_)` at 255) — **both** become `(_, _)`, not just the Adapter one.
- **Golden snapshot regenerates:** `golden_codegen::golden_struct_dependencies_mixed` snapshots `dependencies()` Debug output (`golden_codegen.rs:108-124`, `TypeId` already filtered). Adding the type name changes the snapshot content. `type_name` output is compiler-version-sensitive, so the regenerated snapshot should **normalize it** (extend the existing `insta` filter to reduce the full path to its trailing segment) to avoid cross-toolchain flakiness.
- F-08 (report *all* missing deps) is deferred, so first-failure semantics are correct for this slice.

### AD-4 — F-03: enrich `DependencyNotFound` with names + `Display` + `Error`
**Decision:**
```rust
pub enum RuntimeError {
    ServiceNotFound,
    DependencyNotFound { type_name: &'static str, service_name: Option<&'static str> },
}
impl std::fmt::Display for RuntimeError { /* names both */ }
impl std::error::Error for RuntimeError {}
```
Two producers populate this error, both with `type_name` as a `&'static str` (zero allocation):
- **Resolution-path failures** (`build()` at runtime, or a missing entry at resolve time): `resolve_adapter`/`resolve_config` populate `type_name` via `std::any::type_name::<A>()`, `service_name: None`.
- **Validation-path failures** (`try_build()` via AD-3's `check_dependency`): `type_name` comes from the enriched `DepKey` (AD-3), never from constructing the service.

In both cases, `try_build`'s validator rewrites `service_name` to `Some(type_name::<S>())` on the way out.
**Rationale:** directly answers F-03 (name the missing type *and* the requesting service). Note the `DepKey` type-name enrichment that feeds the validation path is owned by **AD-3**, not this ADR; the `RuntimeError` enrichment here is orthogonal and needed for resolution-path failures regardless.
**Fallout (flag for tasks):**
- `DependencyNotFound` moves from a unit variant to a struct variant → existing `matches!(e, Err(RuntimeError::DependencyNotFound))` sites (builder.rs tests, runtime_builder.rs tests, testkit fixtures.rs:255,331) must become `DependencyNotFound { .. }`. Mechanical. **Three construction sites, not two** — `resolve_adapter`/`resolve_config` (`runtime_builder.rs:79-93`) as already noted, plus `resolve_projection` (`runtime_builder.rs:76`), which also returns `.ok_or(RuntimeError::DependencyNotFound)` today and will fail to compile once the variant gains fields, even though projections are always empty for AD-3's validation purposes (the method itself still exists and must be updated).
- The macro's `create_proxy` currently maps a failed downcast to `RuntimeError::DependencyNotFound` (`lib.rs:481`). A failed downcast is a *resolution* failure, not a missing dependency — re-point it to `RuntimeError::ServiceNotFound`. This is a small codegen change; confirm `proxy_codegen.rs`/`golden_codegen.rs` stay green (the descriptor snapshot is unaffected).

### AD-5 — TestKit uses the same canonical path (F-06/F-07), thin pass-throughs
**Decision:** `FixtureBuilder` gains `with_service::<Tag>(Arc<dyn Trait>)` that forwards to `RuntimeBuilder::with_service` before it builds the fixture's real `Runtime`; `ServiceTestFixture` gains `resolve::<Tag>()` forwarding to `Runtime::resolve`. No parallel wiring, no bespoke proxy assembly.
**Rationale:** `FixtureBuilder::build` already constructs a real `Runtime` through the production constructor (`fixtures.rs:157-162`; audit A.4 verified line-by-line). Forwarding to the same `with_service`/`resolve` makes "same path in production and TestKit" literally true (Principle #7, the `FixtureBuilder` precedent, Section D.1). This retires the ≥4 hand-rolled `make_proxy` helpers (F-06) and gives the enforcement-wrapped-trait-proxy coverage TestKit lacks (F-07).
**Note:** the fixture registers *before* it builds — so it accumulates `with_service` calls on its internal `RuntimeBuilder`. `service::<S: Injectable>()` stays as-is (the DI-struct path is unchanged).

### AD-6 — `{Trait}Ref::new(inner, chain, weak)` stays a supported escape hatch, unconditionally
**Decision:** No deprecation, no `#[doc(hidden)]`, no removal. The macro keeps generating it verbatim.
**Rationale:** verbatim proposal Compatibility commitment. `resolve` is built *on top of* `create_proxy`, which is built on the same `{Trait}Ref` — they coexist by construction. A future "we have `resolve()` now" is explicitly *not* sufficient grounds to remove `::new()`.

### AD-7 — Architectural Limitation: the SDK cannot express "`Injectable` struct AND resolvable trait proxy" for the same service
**This is not a deferred task — it is a present-tense boundary of what CORE-025 makes expressible.** Read it as "the SDK cannot model this," not "we'll get to this next sprint." **This limitation emerges from the current runtime lifecycle (registration pre-`build()`, `Injectable` construction post-`build()`) — it is intentionally documented here as a fact about today's implementation, not asserted as a long-term architectural goal.** The correct reading is "the SDK today cannot do otherwise," not "the SDK is designed to work this way forever." A future change that revisits the runtime's construction lifecycle could close this gap without contradicting anything decided in CORE-025.

**The limitation:** a service can be modelled as an `Injectable` struct (DI: constructed *from* the built runtime via `Injectable::build(rt.inner())`) **or** as a resolvable trait proxy (registered *into* the builder as `Arc<dyn Trait>`, resolved via `resolve::<Tag>()`) — **but not both at once.** There is no combined form in this design. The two are structurally opposed:
- A resolvable proxy must be registered **before** `build()` (the `Arc<dyn Trait>` must be in hand to wrap into `ResolvableContainer` and store in the registry, which is immutable after `build()`, shared via `Arc`/`Weak`).
- An `Injectable` struct can only be constructed **after** `build()` (its `build()` needs the already-built `RuntimeInner` to resolve its dependencies).

So a service whose impl is an `Injectable` struct **cannot** be registered with `with_service`, and there is no SDK surface that both DI-constructs a service and exposes it as a resolvable, enforcement-wrapped trait proxy.

**Why it stays unresolved here (scope, not oversight):** the proposal explicitly excludes unifying these two paths. Closing the gap would require either post-build registration (`Mutex<ServiceRegistry>` inside `RuntimeInner` — new concurrency semantics, contradicts the immutable-runtime model) or an `Unsize`-based generic coercion (unstable Rust). Neither is justified by the audit, and today the two paths are genuinely disjoint in the real code: `Injectable` services are constructed and used directly; they are never wrapped into a trait proxy anywhere in the workspace.

**Consequence for this slice:** F-02 fail-fast (`with_injectable`/`try_build`) and F-01 proxy registration (`with_service`/`resolve`) are **separate calls**, each doing one clear thing. If a real service ever needs to be *both* — a DI-constructed struct that is also resolvable as a typed proxy — it hits this limitation and needs a dedicated follow-up design (post-build registration, or a construct-then-register bridge). Recorded as a limitation in Risks, not as a routine deferral.

---

## Constraint compliance checklist

- [x] `{Trait}Ref::new(inner, chain, weak)` keeps working unconditionally (AD-6).
- [x] Macro-generated contract change stated exactly: `type Service = dyn Trait` added to `Resolvable` impl (F-01); `create_proxy` downcast-failure re-points to `ServiceNotFound` (AD-4). Snapshot tests flagged (descriptor snapshot unaffected; compile tests stay green — confirm in tasks).
- [x] No ambient state / global locator / task-local — registry lives inside the runtime instance the caller holds; `ServiceContext` stays an explicit parameter (Principles #4, #5).
- [x] Fail-closed enforcement preserved exactly — `resolve` returns the *same* generated `{Trait}Ref` running the *same* guard order; no alternate code path (Scenario 5, AD-1/AD-2).
- [x] F-02 check location + target stated: in `try_build()`, against the built `RuntimeInner`'s adapter/config/projection tables, via a dedicated `Injectable::validate()` presence check — **never** by constructing the service (AD-3 / OQ-2).
- [x] No duplicate DI container — every method forwards to existing `ServiceRegistry`/`Resolvable`/`Injectable` (Principle #6).

---

## Risks / assumptions carried to spec & tasks

| Risk / assumption | Severity | Note |
|---|---|---|
| `DependencyNotFound` unit → struct variant breaks `matches!` sites across the crate + testkit | Med | Mechanical churn; enumerate the sites in tasks (builder.rs, runtime_builder.rs, testkit fixtures.rs). Three construction sites, not two — `resolve_projection` (`runtime_builder.rs:76`) also needs updating alongside `resolve_adapter`/`resolve_config`, even though it's unreachable for validation purposes today |
| `resolve::<Tag>()` needs an explicit `RegistryError -> RuntimeError` mapping step before `create_proxy` | Low | Trivial, but must be implemented explicitly in tasks — not automatic from `resolve_raw`'s return type |
| `create_proxy` codegen edit (error re-point) could shift a `proxy_codegen` expectation | Low | Descriptor snapshot unaffected; verify the compile+run tests in tasks |
| `type Service` associated type must not perturb `golden_codegen` output | Low | Snapshot is `descriptor()` only; expected green — confirm |
| `DepKey` gains a `&'static str` type-name field (public type shape change) + macro `classify_field_type` change | Med | Mechanical, 4 construction/match sites enumerated in AD-3 fallout (`di/mod.rs` test, `testkit fixtures.rs:205`, `proxy_codegen.rs:254-255`); `dependencies()` has zero non-test readers so blast radius is contained |
| `golden_codegen::golden_struct_dependencies_mixed` snapshot regenerates once `DepKey` carries the type name | Low-Med | `type_name` is compiler-version-sensitive — normalize it in the `insta` filter (trailing path segment) so the golden snapshot stays stable across toolchains. Regen is expected, not a surprise |
| `Injectable::validate()` correctly checks Adapter + Config + Projection presence; `DepKey::Entity` stays unvalidatable | Low | No entity table / resolver exists yet (Option A's `build()` has the identical blind spot). Revisit when entity-sdk (CORE-006) lands |
| **Architectural limitation (AD-7):** the SDK cannot model a service that is both an `Injectable` struct and a resolvable trait proxy | Med | Not a deferral — a present boundary of expressiveness. If a real service needs both, it requires a dedicated follow-up design (post-build registration / construct-then-register bridge); proposal scope excludes it here |
| Single-version ergonomic API only; multi-version stays on the low-level registry | Low | Deliberate (YAGNI); note in spec so it is not read as a gap |

---

## Next step

`sdd-tasks` (once `spec.md` is also ready) — the spec must encode: the new requirement for canonical registration/resolution, the new fail-fast dependency-validation requirement (distinct from the untouched CORE-018b `build()` requirement), the diagnosable-error requirement, and the TestKit same-path requirement, plus the five acceptance scenarios from the proposal.
