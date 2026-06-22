# Design: CORE-010A — Remove Ambient ServiceContext

## Technical Approach

Adopt **Option A — Explicit Context Everywhere**. `ServiceContext` becomes an explicitly owned runtime dependency (same philosophy as actor refs, runtime handles, security propagation). Delete the three ambient APIs (`CURRENT_CONTEXT` task-local, `current()`, `scope()`) from `crates/service-sdk/src/context/mod.rs`, and rewrite the proxy code-gen macro (`crates/service-sdk-macros/src/lib.rs`) so generated forwarding methods take `ctx: ServiceContext` as their first parameter and pass `&ctx` explicitly to `enforce_tenant` and the interceptor chain. Tests, the example, and `COOKBOOK.md` move to explicit construction/passing.

**Invariant enforced**: Execution context MUST be visible in API boundaries.

> **Codebase correction (verified, must be honored)**: The launch note "Zero production callers — no proxy/macro rewrite needed" is FALSE. `lib.rs:119-149` generates PRODUCTION proxy code calling `ServiceContext::current()` + `.scope()` for tenant enforcement and the interceptor chain. The macro rewrite is mandatory and load-bearing, matching the proposal.

## Architecture Decisions

### ADR-1: Explicit context parameter vs ambient lookup

| Option | Tradeoff | Decision |
|--------|----------|----------|
| **A. Explicit `ctx: ServiceContext` param** | Visible in signatures; auto-propagates across `spawn`; no hidden state | **CHOSEN** |
| `ServiceContext::current()` | Hidden input; breaks on task boundaries | Rejected |
| `task_local!` / `thread_local!` CURRENT_CONTEXT | Ambient; lost across spawned tasks; violates CORE-009 | Rejected |
| `OnceCell` / `LazyLock<ServiceContext>` | Single global ctx; wrong per-invocation semantics | Rejected |
| Global Context Registry | Ambient lookup by key; same hidden-dependency flaw | Rejected |
| Proxy/runtime/interceptor-owned hidden ctx | Re-creates ambient state behind an abstraction | Rejected |
| Context Provider w/ ambient lookup | Indirection over the same anti-pattern | Rejected |

**Rationale**: Only an explicit parameter makes the execution input first-class, survives `tokio::spawn` via capture in `async move`, and aligns with the project's explicit-DI constraint (CORE-009) and the security-sdk "no task-local" invariant (`security-sdk/spec.md:308`).

### ADR-2: Generated proxy method shape

**Choice**: `ctx` is the first parameter of every generated forwarding method; pass `ctx.clone()` to the inner impl, `&ctx` to enforcement/interceptors.

```rust
async fn create_order(&self, ctx: ServiceContext, request: CreateOrderRequest)
    -> Result<CreateOrderResponse>
{
    if let Some(rt) = self.runtime.upgrade() { rt.enforce_tenant(&ctx); }
    self.chain.on_request(&ctx).await?;
    let result = self.inner.create_order(ctx.clone(), request).await;
    match &result {
        Ok(_)  => self.chain.on_response(&ctx).await?,
        Err(e) => self.chain.on_error(&ctx, e as &dyn ServiceErrorTrait).await?,
    }
    result
}
```

Since the macro already iterates `method.sig.inputs`, the `#[operation]` trait method signatures themselves must declare `ctx: ServiceContext` first; the macro forwards it like any other arg. No new ambient acquisition step remains. `enforce_tenant` keeps its current `&ServiceContext -> ()` signature (no `?`).

## Data Flow

```
caller ── owns ctx ──> ProxyRef::method(ctx, args)
                           │  &ctx
                           ├──> runtime.enforce_tenant(&ctx)
                           ├──> chain.on_request(&ctx)
                           ├──> inner.method(ctx.clone(), args)   // explicit handoff
                           └──> chain.on_response/on_error(&ctx)
```
Across `tokio::spawn`: caller captures `let ctx = ctx.clone();` inside `async move { ... }`. No task-local; propagation is the caller's explicit responsibility.

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/service-sdk/src/context/mod.rs` | Modify | Delete `task_local!` block (9-11), `current()` (187-189), `scope()` (199-206), and the doc line referencing `current()` (192). Keep struct + builders + getters. |
| `crates/service-sdk-macros/src/lib.rs` | Modify | Replace ambient acquisition (119-149) with explicit `ctx`-param forwarding per ADR-2. |
| `crates/service-sdk/tests/context_scope.rs` | Modify | Replace scope/current with explicit field assertions on owned/cloned `ServiceContext` (see Testing Strategy). |
| `crates/service-sdk/tests/context_propagation.rs` | Modify | Build ctx explicitly, pass through; assert preserved fields. |
| `crates/service-sdk/tests/context_cross_service.rs` | Modify | Same; assert tenant survives explicit handoff. |
| `crates/service-sdk/tests/deadline_expiry.rs` | Modify | Drop `scope()`; assert on owned ctx. |
| `crates/service-sdk/tests/proxy_codegen.rs` | Modify | Pass `ctx` arg to `charge/refund`; capture via param not `current()`. |
| `crates/service-sdk/tests/smoke.rs` | Modify | Rewrite `test_context_scope` to explicit clone/pass. |
| `crates/service-sdk/examples/order_service.rs` | Modify | Explicit ctx threading. |
| `COOKBOOK.md` | Modify | Replace `scope()`/`current()` examples + the mermaid diagram (275-277, 539-540). |
| `openspec/specs/{service-sdk,security-sdk}/spec.md` | Modify | Delta specs (sdd-spec phase). |

## Interfaces / Contracts

- `#[operation]` trait methods MUST declare `ctx: ServiceContext` as the first param after `&self`. This is the contract change — callers now supply context explicitly.
- Interceptor trait unchanged: `on_request/on_response/on_error(&ServiceContext, ...)`.
- `RuntimeInner::enforce_tenant(&ServiceContext)` unchanged.

## Replacement pattern for former `scope()`/`current()` callers

- **Was** `ctx.scope(|| async { ServiceContext::current()... }).await` → **Now** pass `ctx` (or `ctx.clone()`) directly into the call; read fields off the owned value.
- **Spawned tasks**: `let ctx = ctx.clone(); tokio::spawn(async move { svc.op(ctx, args).await });` — explicit capture replaces ambient inheritance (addresses FR-005 spawned-task loss).
- **Tests verifying nesting**: assert field preservation across explicit clone/pass, not ambient set/restore. Former "restores outer after inner scope" becomes "outer `ctx` value is unaffected by inner `ctx`" — trivially true with owned values.

## Tokio dependency question (investigated)

`Cargo.toml` uses `tokio = { features = ["full"] }` and `tokio-util` (`CancellationToken` on `ServiceContext`). `task_local!` is part of tokio core, not a separate feature. Removing it does **not** shrink the dependency: tokio is still required for the async runtime, `#[tokio::test]`, and `tokio-util::CancellationToken`. **Decision**: keep `tokio` as-is; no `Cargo.toml` change. (Optional later hygiene: narrow `"full"` to needed features — out of scope here.)

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit | Field preservation through clone/pass | Build ctx, clone, assert fields equal |
| Integration | Tenant + interceptor order unchanged | proxy_codegen spy asserts `on_request -> on_response/on_error`; tenant captured via param |
| Regression | No ambient API remains | `grep -rn "current()\|scope(\|task_local" crates/.../src` returns no service-context hits |

## Migration / Rollout

No data/schema/consumer migration. Pure refactor on a feature branch; revert restores prior macro output and ambient APIs. `cargo fmt`, `cargo clippy --all-targets --all-features`, `cargo test --workspace` gate the change.

## ADR-2: Concrete Context Over Abstract Context

**Decision**: Service operations SHALL receive `ServiceContext` directly. A2 (`ExecutionContext` trait) is rejected.

**Rationale**: Only one execution context implementation exists. No demonstrated production variability. Follows the same pattern as `DomainExecutionContext` (concrete type, no trait indirection). Preserves static type safety and avoids speculative abstractions.

**Rejected alternative**: `async fn create_order(&self, ctx: &dyn ExecutionContext, ...)` — trait-object indirection with no current benefit.

**Boundary rule**:
- `ServiceContext` MAY appear in: Service Layer, Proxy Layer, Runtime Layer, Scheduler Layer, Projection Layer, Integration Layer
- `ServiceContext` MUST NOT appear in: Aggregates, Entities, Value Objects, Domain Events, Domain Services, Domain Model abstractions
- When domain layer needs context data → translate to domain-specific types before crossing the boundary

**Future evolution**: if multiple execution context implementations emerge in production, a dedicated abstraction may be introduced as a separate change. Until then, `ServiceContext` is canonical.

## Open Questions

- None. All architectural decisions finalized: Option A approved, A1 approved, macro rewrite confirmed mandatory.
