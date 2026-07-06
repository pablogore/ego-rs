# Design: CORE-018b — Restrict RuntimeInner Construction to RuntimeBuilder

## Technical Approach

This is a **visibility restriction plus call-site migration**, no behavioral
change to correctly-built runtimes. The goal: after this change, the only way
any code *outside* `service-sdk` can obtain a `RuntimeInner` is
`RuntimeBuilder::build()`, making the CORE-017 lifecycle guarantees (logger
wiring, ordered teardown, no rogue `security_providers`) structurally
unavoidable rather than conventional.

Three moving parts:

1. `RuntimeInner::new()` (`runtime_builder.rs:138`) drops from `pub` to
   `pub(crate)`.
2. `impl Default for RuntimeInner` (`runtime_builder.rs:251`) is **removed**
   (see the forced decision below — `pub(crate)` is not expressible for a
   trait impl).
3. Every construction site outside `RuntimeBuilder::build()` is migrated. A
   full workspace survey (not the two sites the proposal named) found the real
   set is larger — see the migration table.

`RuntimeBuilder::build()` already constructs `RuntimeInner` via
`new_with_logger()` (`pub(super)`), **not** via `new()` or `default()`. So
narrowing `new()` and removing `Default` does not touch the production
construction path at all — it only affects test call sites.

### Grounding finding (drives Decision 1)

`RuntimeInner` must stay `pub` — the `ego-service-sdk-macros` proc-macro crate
generates proxy code that names `std::sync::Weak<ego_service_sdk::runtime::RuntimeInner>`
and `&ego_service_sdk::runtime::RuntimeInner` (verified in
`crates/service-sdk-macros/src/lib.rs:357,364,381,455`). The macros only
*reference* the type; they never call `::new()` or `::default()`. So generated
code is unaffected by constructor visibility, but the type export cannot be
removed.

**Consequence**: because `RuntimeInner` stays `pub` and `Default` is a public
trait, `RuntimeInner::default()` is callable by any downstream crate as long as
the `impl Default` exists. Rust does **not** let a trait-impl method be scoped
to `pub(crate)` — impl method visibility follows the trait + type. The
proposal's "keep `Default` `pub(crate)` if internal code needs it" is therefore
**not expressible**. Removal is the only mechanism that closes external
`::default()` construction.

Production code never uses `Default`: `build()` uses `new_with_logger`; every
`::default()` call in the tree is in a test module (confirmed by workspace
grep). So removal costs nothing in production and only rewrites test fixtures.

## Architecture Decisions

### Decision 1: remove `impl Default for RuntimeInner` entirely (not `pub(crate)`)

| Option | Tradeoff | Decision |
|--------|----------|----------|
| Keep `Default`, scope it `pub(crate)` | **Impossible** — trait-impl method visibility follows the public `Default` trait + `pub RuntimeInner`; cannot be narrowed | Rejected (not expressible) |
| Keep `Default` `pub` | Leaves the exact external bypass this change exists to close | Rejected |
| Remove `impl Default`; internal tests use `new()` / a `#[cfg(test)] pub(crate)` helper | Closes external `::default()`; production unaffected (build() uses `new_with_logger`); only test churn | **Chosen** |

**Rationale**: A trait impl cannot be `pub(crate)`, so "restrict `Default`" and
"remove `Default`" collapse into the same action for external callers —
removal. `default()` is also, ironically, the *least* dangerous constructor
(it always sets `security_providers: None`, so it fails closed on authz); the
real TASK-014 hazard is `new()` with hand-crafted providers. Removing `Default`
and narrowing `new()` together close both the "empty/no-logger divergence" and
the "rogue providers" holes. To keep internal unit tests terse and preserve
their access to the private `resolved` table, add a `#[cfg(test)] pub(crate)`
inherent helper on `RuntimeInner` (an inherent method *can* be `pub(crate)`,
unlike a trait impl) that wraps `new(ServiceRegistry::new(),
Arc::new(InterceptorChain::new()), None)`. This does not widen the public API.

### Decision 2: all external sites migrate mechanically to `RuntimeBuilder`; no external test-only helper needed

Every external construction site can be rewritten to `RuntimeBuilder` because
the builder produces an **equivalent** `RuntimeInner`:

- `RuntimeInner::default()` ≡ `RuntimeBuilder::new().build()` — both yield empty
  registry, empty interceptor chain, `security_providers: None`, empty
  `resolved`, `logger: None`, empty teardown. Byte-for-byte equivalent.
- `RuntimeInner::new(ServiceRegistry::new(), Arc::new(InterceptorChain::new()),
  Some((authn, authz)))` ≡ `RuntimeBuilder::new().with_security(authn,
  authz).build()`.

No external test needs a `RuntimeInner` shape the builder cannot produce: none
of them inject into the private `resolved` table (they can't — it's private),
and `security_providers` is only ever `Some((authn, authz))` or `None`, both of
which the builder covers. `Runtime::inner()` returns `&Arc<RuntimeInner>`,
which supplies the `Arc` (for `Arc::downgrade`) and `&RuntimeInner` (via deref)
that the migrated call sites need. **Therefore no `#[cfg(test)] pub(crate)`
helper is required for external sites** (the proposal's Risk-2 mitigation is
unnecessary here). Internal in-crate tests are the only ones that keep a
crate-local constructor, per Decision 1.

**Rationale**: The proposal named only `builder.rs` tests and
`authorization_integration.rs`. The real survey (see table) shows the builder's
constructor and its unit tests live in `runtime_builder.rs` (not `builder.rs`),
and there are three additional external files the proposal missed. All migrate
mechanically; none blocks the change.

### Decision 3: `pub(crate) fn new(...)`; `new_with_logger` stays `pub(super)`

**Choice**: `pub(crate) fn new(...)`. Confirmed safe:

- `RuntimeBuilder::build()` calls `new_with_logger` (`pub(super)`), **not**
  `new` — narrowing `new` cannot break `build()`.
- Workspace-wide grep for `RuntimeInner::(new|default)(` outside test/doc files
  returns only `runtime_builder.rs` (the definition + its own in-crate tests)
  and `authorization_integration.rs` (external test, being migrated).
  `crates/runtime`, `crates/runtime-tokio`, and `service-sdk-macros` reference
  the type (`Weak<RuntimeInner>`) but never construct it.
- After migration, `new`'s only callers are in-crate tests (same crate,
  different module `context/mod.rs` included), so `pub(crate)` is exactly
  right. `new` stays because one internal test genuinely needs the
  `security_providers`-carrying variant (`runtime_builder.rs:513`).

## Migration Map (the real, surveyed set)

| File | Crate boundary | Sites | Current | Migration |
|------|----------------|-------|---------|-----------|
| `src/runtime/runtime_builder.rs` (tests) | in-crate | ~13 `default()` + 1 `new(...)` | `RuntimeInner::default()` / `::new(_,_,Some(..))` | `RuntimeInner::new(reg, chain, None)` via a `#[cfg(test)] pub(crate)` helper; the `new(...)` site keeps its explicit providers. `pub(crate)` keeps these compiling; removal of `Default` forces the `default()` rewrites |
| `src/context/mod.rs` (tests) | in-crate | 2 `default()` | `RuntimeInner::default()` then `.issue_cross_tenant_permit()` | same in-crate helper / `new(reg, chain, None)` |
| `tests/authorization_integration.rs` | **external** | 1 `new(...)` (`make_runtime`, line 181) | `Arc::new(RuntimeInner::new(reg, chain, Some((authn,authz))))` | `RuntimeBuilder::new().with_security(authn, authz).build()`; return `(Runtime, Weak)` where weak = `Arc::downgrade(rt.inner())`. `t22`'s `drop(rt)` still drops the inner Arc → `Weak::upgrade` fails, identical semantics |
| `tests/proxy_codegen.rs` | **external** | 6 `default()` (72,138,163,203 → `Weak`; 264,282 → `&RuntimeInner`) | `Arc::new(RuntimeInner::default())` | `let rt = RuntimeBuilder::new().build();` then `Arc::downgrade(rt.inner())` or `rt.inner()` (deref → `&RuntimeInner` for `Injectable::build`) |
| `tests/compile_fail/issue_cross_tenant_permit_external.rs` (+`.stderr`) | **external, compile-fail** | 1 `default()` | `RuntimeInner::default()` then private `issue_cross_tenant_permit()` | Rewrite to `RuntimeBuilder::new().build().inner().issue_cross_tenant_permit()` and **regenerate the `.stderr`** — see Risks |

## Interfaces / Contracts

```rust
// runtime_builder.rs — narrowed constructor, Default removed.
impl RuntimeInner {
    pub(crate) fn new(                       // was: pub
        registry: ServiceRegistry,
        interceptor_chain: Arc<InterceptorChain>,
        security_providers: Option<(Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>)>,
    ) -> Self { /* unchanged body */ }

    pub(super) fn new_with_logger(/* unchanged */) -> Self { /* unchanged — build()'s path */ }

    // Optional terse test fixture. Inherent method CAN be pub(crate) (a trait impl cannot).
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self::new(ServiceRegistry::new(), Arc::new(InterceptorChain::new()), None)
    }
}

// REMOVED entirely:
// impl Default for RuntimeInner { fn default() -> Self { ... } }
```

`RuntimeInner` stays `pub` (macro-facing). `mod.rs`'s
`pub use runtime_builder::{RuntimeError, RuntimeInner};` is unchanged — the type
is still exported; only its constructors are no longer public.

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Compile | `RuntimeInner::default()` and `RuntimeInner::new(..)` are not callable externally | The existing trybuild compile-fail suite; regenerate `issue_cross_tenant_permit_external.stderr` (now the construction line, reached via `RuntimeBuilder`, still gates `issue_cross_tenant_permit`) |
| Unit (in-crate) | All migrated `runtime_builder.rs` / `context/mod.rs` tests still pass with `new(..)` / `for_test()` | `cargo test -p ego-service-sdk` |
| Integration (external) | `authorization_integration.rs` T-18..T-24 keep identical semantics through `RuntimeBuilder`; `t22` drop-path still yields `Weak::upgrade == None` | run the integration test file unchanged in intent |
| Integration (external) | `proxy_codegen.rs` proxy/interceptor/injectable tests unaffected by builder-produced runtime | run the file |
| Build | Whole workspace compiles — the compiler is the enforcement: any missed external site fails to build | `cargo build --workspace` |

## Migration / Rollout

Single commit. No data, config, or public-API-consumer migration — the public
API *loses* two constructors that no downstream production code uses (only
tests). `RuntimeBuilder::build()` is unchanged. `new_with_logger` unchanged.
Rollback = revert the commit.

## Open Questions — for the Tasks phase

- [ ] **Prefer `for_test()` helper vs. inline `new(reg, chain, None)`** at the
  ~15 in-crate test sites. The helper is cleaner for the no-arg cases but some
  `runtime_builder.rs` tests also mutate the private `resolved` table
  afterward (`let mut rt = ...; rt.resolved.projections.insert(..)`), which
  both forms support since they return an owned in-crate `Self`. Tasks phase
  picks one for consistency.

## Future Considerations

Not pending decisions — deferred work, noted so they aren't read as guarantees
of this change.

- **TASK-014 runtime authorization check.** Once
  `issue_cross_tenant_permit` performs a real `AuthorizationProvider` check and
  changes to a fallible signature, the value of gating `new()` grows (a rogue
  in-crate `new(.., Some(custom_providers))` is the last remaining internal
  bypass). This change removes the *external* bypass; the internal one is
  covered by `pub(crate)` + the fact that only sanctioned in-crate code and
  tests can reach it.
- **Issue #120 (builder DI write-side, `.with_adapter()`/`.with_config()`).**
  Depends on this landing first: it relies on `RuntimeBuilder::build()` being
  the single construction path so DI registration has exactly one funnel.
