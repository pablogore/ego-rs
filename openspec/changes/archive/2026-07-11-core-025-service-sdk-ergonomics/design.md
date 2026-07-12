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

## Constraint compliance checklist

- [x] `{Trait}Ref::new(inner, chain, weak)` keeps working unconditionally (AD-6).
- [x] Macro-generated contract change stated exactly: `type Service = dyn Trait` added to `Resolvable` impl (F-01); `create_proxy` downcast-failure re-points to `ServiceNotFound` (AD-4). Snapshot tests flagged (descriptor snapshot unaffected; compile tests stay green — confirm in tasks).
- [x] No ambient state / global locator / task-local — registry lives inside the runtime instance the caller holds; `ServiceContext` stays an explicit parameter (Principles #4, #5).
- [x] Fail-closed enforcement preserved exactly — `resolve` returns the *same* generated `{Trait}Ref` running the *same* guard order; no alternate code path (Scenario 5, AD-1/AD-2).
- [x] F-02 check location + target stated: in `try_build()`, against the built `RuntimeInner`'s adapter/config/projection tables, via a dedicated `Injectable::validate()` presence check — **never** by constructing the service (AD-3 / OQ-2).
- [x] No duplicate DI container — every method forwards to existing `ServiceRegistry`/`Resolvable`/`Injectable` (Principle #6).
