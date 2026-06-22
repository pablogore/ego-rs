# Tasks: CORE-010A — Remove Ambient ServiceContext

**Change**: remove-ambient-service-context
**Delivery**: single-pr — all tasks in one PR
**TDD Mode**: STRICT — RED first (failing test commit), then GREEN (implementation commit)
**Test runner**: `cargo test --workspace`

---

## Phase 1 — Remove Ambient APIs from ServiceContext

> Sequential. Phase 2 depends on Phase 1 completing first.

### TASK-001 — RED: Write failing test asserting `current()` and `scope()` are gone

**Spec refs**: FR-001, FR-003, MR-001, MR-002, MR-003, AC-001, AC-002, AC-003, NFR-004

**Action**: In `crates/service-sdk/tests/context_scope.rs`, rewrite the four existing tests to use
explicit owned-value assertions instead of `scope()` / `current()`. The file currently calls
`context.scope(|| async { ... })` and `ServiceContext::current()` — replace every such call so
the file compiles only when those APIs no longer exist. Commit as RED (the file will NOT compile
until TASK-002 deletes the APIs).

Replacement shape for each test:
- Build a `ServiceContext` explicitly.
- Clone it and assert that field values on the clone equal the originals.
- The former "scope restores context" test becomes: assert that two separate owned values are
  independent (trivially true for value types — no ambient side effect to verify).

**Acceptance criterion (RED state)**: `cargo build -p ego-service-sdk --tests` fails with
`no method named scope` / `no method named current` on the rewritten file.

---

### TASK-002 — RED: Write failing test asserting propagation is explicit

**Spec refs**: FR-002, FR-005, MR-004, AC-004, AC-005

**Action**: Rewrite `crates/service-sdk/tests/context_propagation.rs` (currently one test using
`context.scope(|| async { ServiceContext::current() })`) to assert that a cloned `ServiceContext`
carries all fields through explicit passing:

```rust
#[tokio::test]
async fn test_service_context_explicit_propagation() {
    let ctx = ServiceContext::new()
        .with_tenant_id("tenant-123")
        .with_correlation_id("correlation-456")
        .with_trace_id("trace-789");

    // Explicit passing — no scope, no ambient read
    let ctx2 = ctx.clone();
    assert_eq!(ctx2.tenant_id(), Some("tenant-123"));
    assert_eq!(ctx2.correlation_id(), Some("correlation-456"));
    assert_eq!(ctx2.trace_id(), Some("trace-789"));
}

#[tokio::test]
async fn test_spawned_task_receives_context_explicitly() {
    let ctx = ServiceContext::new().with_tenant_id("spawn-tenant");
    let ctx_clone = ctx.clone();
    let result = tokio::spawn(async move {
        ctx_clone.tenant_id().map(|s| s.to_owned())
    })
    .await
    .unwrap();
    assert_eq!(result.as_deref(), Some("spawn-tenant"));
}
```

**Acceptance criterion (RED state)**: File compiles and tests pass in isolation, but
`context_scope.rs` (TASK-001) still drives the RED state for the ambient APIs.

---

### TASK-003 — RED: Rewrite cross-service test to use explicit clone/pass

**Spec refs**: FR-002, FR-004, MR-004, AC-009, INV-001

**Action**: Rewrite `crates/service-sdk/tests/context_cross_service.rs` (currently one test using
`context.scope(|| async { ServiceContext::current() })`) to simulate cross-service handoff via
explicit parameter passing:

```rust
#[tokio::test]
async fn test_context_cross_service_explicit() {
    let ctx = ServiceContext::new()
        .with_tenant_id("tenant-123")
        .with_correlation_id("correlation-456");

    // Simulate service boundary: clone and pass
    async fn service_b(ctx: ServiceContext) -> (Option<String>, Option<String>) {
        (
            ctx.tenant_id().map(|s| s.to_owned()),
            ctx.correlation_id().map(|s| s.to_owned()),
        )
    }

    let (tenant, correlation) = service_b(ctx.clone()).await;
    assert_eq!(tenant.as_deref(), Some("tenant-123"));
    assert_eq!(correlation.as_deref(), Some("correlation-456"));
}
```

**Acceptance criterion (RED state)**: Compiles and passes in isolation.

---

### TASK-004 — GREEN: Delete ambient APIs from `context/mod.rs`

**Spec refs**: MR-001, MR-002, MR-003, AC-001, AC-002, AC-003, AC-006, NFR-004

**Action**: In `crates/service-sdk/src/context/mod.rs`:
1. DELETE lines 8-11: the comment and `tokio::task_local! { static CURRENT_CONTEXT: ServiceContext; }` block.
2. DELETE lines 183-189: the doc comment block `/// Gets the current service context...` and the
   `pub fn current() -> Option<ServiceContext>` method (which calls `CURRENT_CONTEXT.try_with(...)`).
3. DELETE lines 191-206: the doc comment block `/// Creates a new scope...` and the
   `pub fn scope<F, Fut>(&self, f: F) -> ...` method.

Keep all other methods: `new()`, all `with_*` builders, `security()`, `is_cancelled()`,
`is_deadline_expired()`, `is_cross_tenant_allowed()`, and any remaining methods.

**Verification gate**:
```
cargo build -p ego-service-sdk 2>&1 | grep -c "no method named"
```
Should return non-zero (the old test files that still reference `scope`/`current` will fail).
After TASK-001, TASK-002, TASK-003 tests are in place and no production code calls these methods,
the workspace should build cleanly. Confirm with:
```
rg "ServiceContext::current|\.scope\(|CURRENT_CONTEXT" crates/ --type rust
```
Must return zero results.

**Acceptance criterion (GREEN state)**: `cargo build --workspace` exits 0 after this task AND
all three rewritten test files pass.

---

## Phase 2 — Rewrite Proxy Code-Generation Macro

> Sequential. Must follow Phase 1 (ambient APIs deleted before macro can be verified clean).

### TASK-005 — RED: Write a failing test for the new proxy method shape

**Spec refs**: FR-007, FR-008, AC-008, INV-002, INV-003, Scenario "Proxy generated method signature is explicit"

**Action**: In `crates/service-sdk/tests/proxy_codegen.rs`, add a new test
`context_propagates_via_explicit_param` that replaces the existing
`context_propagates_across_service_boundary` test (which currently uses
`ServiceContext::current()` inside the `PaymentService` impl and `ctx.scope(|| ...)`
at the call site):

New test shape:
```rust
#[tokio::test]
async fn context_propagates_via_explicit_param() {
    struct ContextCapturingService {
        captured_tenant: std::sync::Mutex<Option<String>>,
    }

    #[async_trait]
    impl PaymentService for ContextCapturingService {
        // After macro rewrite, PaymentService operations receive ctx explicitly.
        async fn charge(&self, ctx: ServiceContext, _amount: u64) -> Result<String, ServiceError> {
            *self.captured_tenant.lock().unwrap() = ctx.tenant_id.clone();
            Ok("charged".to_string())
        }
        async fn refund(&self, ctx: ServiceContext, _amount: u64) -> Result<String, ServiceError> {
            Ok("refunded".to_string())
        }
    }

    let capturing = Arc::new(ContextCapturingService {
        captured_tenant: std::sync::Mutex::new(None),
    });
    let inner: Arc<dyn PaymentService> = capturing.clone();
    let chain = Arc::new(InterceptorChain::new());
    let runtime_inner = Arc::new(ego_service_sdk::runtime::RuntimeInner::default());
    let runtime_weak = Arc::downgrade(&runtime_inner);
    let proxy = PaymentServiceRef::new(inner, chain, runtime_weak);

    let ctx = ServiceContext::new().with_tenant_id("tenant-abc");
    proxy.charge(ctx, 42).await.unwrap();

    let captured = capturing.captured_tenant.lock().unwrap().clone();
    assert_eq!(captured.as_deref(), Some("tenant-abc"));
}
```

Also update the two existing interceptor order tests (`interceptors_fire_in_order_via_generated_ref`
and `interceptors_fire_on_success_via_generated_ref`) to pass an explicit `ctx: ServiceContext`
to `proxy.charge(ctx, 100)` and `proxy.refund(ctx, 50)`.

Update the trait declarations in the test file:
- `PaymentService`: add `ctx: ServiceContext` as first param to `charge` and `refund`
- `OrderService`: add `ctx: ServiceContext` as first param to `place_order`

Update all `impl` blocks to match.

**Acceptance criterion (RED state)**: `cargo build -p ego-service-sdk --tests` fails because
the macro has not yet been rewritten (generated `PaymentServiceRef` will not accept `ctx` param).

---

### TASK-006 — GREEN: Rewrite forwarding method generation in `service-sdk-macros/src/lib.rs`

**Spec refs**: FR-007, FR-008, AC-008, INV-002, INV-003, ADR-2 (design.md)

**Action**: Replace lines 117-151 in `crates/service-sdk-macros/src/lib.rs` (the `forwarding_methods.push(quote! { ... })` block) with the explicit-ctx shape from ADR-2.

Current (ambient) shape:
```rust
forwarding_methods.push(quote! {
    async fn #method_name(&self, #(#arg_names: #arg_types),*) #return_type {
        let ctx = ego_service_sdk::context::ServiceContext::current()
            .unwrap_or_default();
        if let Some(rt) = self.runtime.upgrade() {
            rt.enforce_tenant(&ctx);
        }
        let inner_ref = self.inner.clone();
        let chain_ref = self.chain.clone();
        let ctx_for_scope = ctx.clone();
        ctx_for_scope.scope(|| async move {
            let inner_ctx = ego_service_sdk::context::ServiceContext::current()
                .unwrap_or_default();
            let _ = chain_ref.on_request(&inner_ctx).await;
            match inner_ref.#method_name(#(#arg_names),*).await {
                Ok(v) => { chain_ref.on_response(&inner_ctx).await.ok(); Ok(v) }
                Err(e) => { chain_ref.on_error(&inner_ctx, &e as &dyn ...).await.ok(); Err(e) }
            }
        }).await
    }
});
```

New (explicit) shape — the macro must extract `ctx` from the method's own argument list
(the trait method signature already declares `ctx: ServiceContext` as the first typed arg
after `&self`), then pass `&ctx` to enforce/interceptors and `ctx.clone()` to the inner:

```rust
forwarding_methods.push(quote! {
    async fn #method_name(&self, #(#arg_names: #arg_types),*) #return_type {
        if let Some(rt) = self.runtime.upgrade() {
            rt.enforce_tenant(&ctx);
        }
        let inner_ref = self.inner.clone();
        let chain_ref = self.chain.clone();
        let _ = chain_ref.on_request(&ctx).await;
        let result = inner_ref.#method_name(#(#arg_names),*).await;
        match &result {
            Ok(_)  => { chain_ref.on_response(&ctx).await.ok(); }
            Err(e) => {
                chain_ref
                    .on_error(&ctx, e as &dyn ego_service_sdk::error::ServiceErrorTrait)
                    .await
                    .ok();
            }
        }
        result
    }
});
```

Notes:
- The `ctx` binding is available because the trait method signature (declared by the user)
  includes `ctx: ServiceContext` as the first typed parameter. The macro iterates
  `method.sig.inputs` and the generated forwarding method signature reproduces the same
  params, so `ctx` is in scope with no special extraction needed.
- Remove `unwrap_or_default()` entirely — no ambient fallback.
- Remove the `scope()` wrapper and the inner `CURRENT_CONTEXT` re-read.
- The inner call `inner_ref.#method_name(#(#arg_names),*)` forwards ALL args including `ctx`
  (since `ctx` is in `arg_names`), satisfying explicit handoff to the impl.

**Verification gate**: `rg "ServiceContext::current|\.scope\(|CURRENT_CONTEXT" crates/ --type rust`
must return zero results.

**Acceptance criterion (GREEN state)**: `cargo test --workspace` exits 0; all proxy_codegen
tests pass including the new `context_propagates_via_explicit_param`.

---

## Phase 3 — Update Remaining Tests and Docs

> TASK-007 through TASK-011 can run in parallel with each other once Phase 2 is GREEN.

### TASK-007 — Update `deadline_expiry.rs` (remove `scope()`/`current()`)

**Spec refs**: MR-004, AC-004

**Action**: In `crates/service-sdk/tests/deadline_expiry.rs`, replace the test block that calls:
```rust
let captured_context = context.scope(|| async { ServiceContext::current() }).await;
assert!(captured_context.is_some());
let captured = captured_context.unwrap();
assert!(captured.deadline.is_some());
```
with direct field assertion on the owned value:
```rust
assert!(context.deadline.is_some());
assert!(!context.is_deadline_expired());
```
Keep the deadline-expiry assertion already present above it.

**Acceptance criterion**: `cargo test -p ego-service-sdk deadline` exits 0.

---

### TASK-008 — Update `smoke.rs` (remove `test_context_scope`)

**Spec refs**: MR-004, AC-004

**Action**: In `crates/service-sdk/tests/smoke.rs`, rewrite `test_context_scope` (lines 195-204)
to test that an owned/cloned `ServiceContext` carries `tenant_id` explicitly:

```rust
#[tokio::test]
async fn test_context_explicit_carry() {
    let ctx = ServiceContext::new().with_tenant_id("scoped-tenant");
    let ctx2 = ctx.clone();
    assert_eq!(ctx2.tenant_id(), Some("scoped-tenant"));
    // No scope() call; no current() call.
}
```

Also remove the `assert!(ServiceContext::current().is_none())` line that followed the old test.

**Acceptance criterion**: `cargo test -p ego-service-sdk smoke` exits 0.

---

### TASK-009 — Update `golden_codegen.rs` and snapshots (new `ctx` param in traits)

**Spec refs**: AC-008, NFR-003

**Action**: The golden tests declare `GoldenOrderService` and `GoldenPaymentService` using
`#[operation]`. After the macro rewrite, the generated `ServiceDescriptor` will include
`ctx: ServiceContext` as an input type for each operation. Update:
1. Add `ctx: ServiceContext` as the first parameter to `place_order`, `charge`, and `refund` in
   the two golden trait declarations inside `crates/service-sdk/tests/golden_codegen.rs`.
2. Delete the existing `insta` snapshot files under
   `crates/service-sdk/tests/snapshots/` that correspond to `trait_descriptor_order_service`
   and `trait_descriptor_payment_service`, so insta regenerates them on next run.
3. Run `cargo test -p ego-service-sdk golden_ -- --force-update-snapshots` (or
   `INSTA_UPDATE=always cargo test -p ego-service-sdk golden_`) to regenerate snapshots.
4. Review that the regenerated snapshots include `"ServiceContext"` in the `input` field of
   each operation descriptor.

**Acceptance criterion**: `cargo test -p ego-service-sdk golden_codegen` exits 0 with current
snapshots committed.

---

### TASK-010 — Update `interceptor_invocation.rs` (no change needed — verify clean)

**Spec refs**: AC-010, INV-002

**Action**: Inspect `crates/service-sdk/tests/interceptor_invocation.rs`. This file constructs
`ServiceContext::new()` and calls `chain.on_request(&context)` directly — it does NOT use
`scope()` or `current()`. Verify it compiles and passes without modification after Phase 1.

If `cargo test -p ego-service-sdk interceptor_invocation` exits 0 with no changes, mark done.
If any compile error appears, fix the minimal delta needed.

**Acceptance criterion**: `cargo test -p ego-service-sdk interceptor_invocation` exits 0.

---

### TASK-011 — Update `security_integration.rs` (no change needed — verify clean)

**Spec refs**: AC-004, AC-009

**Action**: Inspect `crates/service-sdk/tests/security_integration.rs`. This file uses only
`ServiceContext::new()` and builder methods — no `scope()` / `current()`. Verify it compiles
and passes without modification after Phase 1.

If `cargo test -p ego-service-sdk security_integration` exits 0 with no changes, mark done.

**Acceptance criterion**: `cargo test -p ego-service-sdk security_integration` exits 0.

---

### TASK-012 — Rewrite `examples/order_service.rs` (remove `scope()`/`current()`)

**Spec refs**: MR-004, AC-006, INV-001

**Action**: In `crates/service-sdk/examples/order_service.rs`:
1. Remove the `context.scope(|| async { ... })` wrapper around the service call (lines 132-146).
2. Remove the `ServiceContext::current()` call inside the closure (line 135-137).
3. Replace with a direct call: `service.create_order(cmd).await`.
4. Remove the `let outside_ctx = ServiceContext::current(); assert!(outside_ctx.is_none());`
   block (lines 161-163), as `current()` no longer exists.
5. Remove the `test_context_scoping` test in the `#[cfg(test)]` module (lines 185-193), which
   uses `context.scope(|| async { ServiceContext::current() })`.
6. Keep `test_create_order_success` intact.

**Acceptance criterion**: `cargo build --example order_service -p ego-service-sdk` exits 0.

---

### TASK-013 — Update `COOKBOOK.md` (remove `scope()`/`current()` examples)

**Spec refs**: NFR-003, AC-011

**Action**: In `/Users/pablogore/workspace/pablogore/ego-rs/COOKBOOK.md`:
1. Replace the mermaid diagram at lines 272-278 that shows `ctx.scope(|| async { ... })` and
   `ServiceContext::current()` with an explicit-passing diagram:
   ```mermaid
   flowchart LR
       A["Request arrives"] --> B["Build ServiceContext\nwith_tenant_id()\nwith_correlation_id()"]
       B --> C["Pass ctx to service method\nsvc.operation(ctx, args)"]
       C --> D["Handler receives ctx\nas owned parameter"]
       D --> E["Clone for sub-calls\nctx.clone()"]
   ```
2. Replace the code snippet at lines 537-542:
   - Old: `ctx.scope(|| async { ServiceContext::current() }).await`
   - New:
     ```rust
     // Build and pass explicitly — no scope, no ambient read
     let ctx = ServiceContext::new().with_tenant_id("my-tenant");
     let ctx2 = ctx.clone();
     assert_eq!(ctx2.tenant_id(), Some("my-tenant"));
     ```

**Acceptance criterion**: `rg "ServiceContext::current|\.scope\(" COOKBOOK.md` returns zero results.

---

## Phase 4 — Final Verification Gates

> Sequential. Runs after all previous phases are complete.

### TASK-014 — Run ambient-API grep (AC-007)

**Spec refs**: AC-001, AC-002, AC-003, AC-007, NFR-004

**Action**: Run the following command and confirm zero results:
```
rg "ServiceContext::current|ServiceContext::scope|CURRENT_CONTEXT" crates/ --type rust
```

**Acceptance criterion**: Command exits with no matching lines. If any match is found, trace
it back to the responsible task and fix before proceeding.

---

### TASK-015 — Run full workspace test suite (AC-004, AC-006)

**Spec refs**: AC-004, AC-006, AC-008, AC-009, AC-010, NFR-001

**Action**:
```
cargo fmt --check && \
cargo clippy --all-targets --all-features -- -D warnings && \
cargo test --workspace
```

All three commands must exit 0.

**Acceptance criterion**: Zero test failures, zero clippy warnings promoted to errors, zero
formatting diffs.

---

### TASK-016 — Code review: no new sync primitives (NFR-002)

**Spec refs**: NFR-002

**Action**: Review the diff of `crates/service-sdk/` between `develop` and the feature branch.
Confirm that no new `Mutex`, `RwLock`, `Arc<Mutex<...>>`, `Semaphore`, or `Condvar` was
introduced in source files (test fixtures may use `std::sync::Mutex` for spy counters —
that is acceptable as it pre-exists).

```
git diff develop -- crates/service-sdk/src/ | rg "Mutex|RwLock|Semaphore|Condvar"
```

**Acceptance criterion**: Zero new synchronization primitives in `crates/service-sdk/src/`.

---

### TASK-017 — Code review: boundary rule (ADR-2, INV-001)

**Spec refs**: FR-001, AC-011, ADR-2 boundary rule

**Action**: Confirm `ServiceContext` does not appear in aggregates, entities, value objects,
domain events, or domain services. Scan the domain layer:
```
rg "ServiceContext" crates/ --type rust -l | rg "aggregate|entity|domain|event|value_object"
```

**Acceptance criterion**: Zero files in domain-layer paths contain `ServiceContext`.

---

## Task Dependency Summary

```
TASK-001 (RED: context_scope rewrite)       ─┐
TASK-002 (RED: context_propagation rewrite)  ├─> TASK-004 (GREEN: delete APIs) ─> TASK-005 (RED: proxy test)
TASK-003 (RED: cross_service rewrite)       ─┘                                         │
                                                                                        v
                                                                               TASK-006 (GREEN: macro rewrite)
                                                                                        │
                                              ┌─────────────────────────────────────────┤
                                              v                                          v
                                    TASK-007 .. TASK-013 (parallel)            TASK-014 .. TASK-017 (sequential, after all)
```

**Parallel groups**:
- TASK-001, TASK-002, TASK-003: can be written in parallel (independent files)
- TASK-007 through TASK-013: can be applied in parallel once TASK-006 is GREEN

**Sequential gates**:
- TASK-004 must follow TASK-001/002/003 (RED tests must be in place first)
- TASK-005 must follow TASK-004 (macro RED test requires APIs already deleted)
- TASK-006 must follow TASK-005 (GREEN implementation against RED test)
- TASK-014 through TASK-017 must follow all prior tasks

---

## Checklist

### Phase 1 — Remove Ambient APIs

- [x] TASK-001 RED: Rewrite `context_scope.rs` (explicit field assertions, no `scope()`/`current()`)
- [x] TASK-002 RED: Rewrite `context_propagation.rs` (explicit passing + spawned task)
- [x] TASK-003 RED: Rewrite `context_cross_service.rs` (explicit clone/pass across boundary)
- [x] TASK-004 GREEN: Delete `CURRENT_CONTEXT`, `current()`, `scope()` from `context/mod.rs`

### Phase 2 — Rewrite Proxy Macro

- [x] TASK-005 RED: Add explicit-ctx proxy test to `proxy_codegen.rs`; update trait/impl signatures
- [x] TASK-006 GREEN: Rewrite forwarding method generation in `service-sdk-macros/src/lib.rs`

### Phase 3 — Update Remaining Tests and Docs (parallel)

- [x] TASK-007: Update `deadline_expiry.rs` (remove `scope()`/`current()`)
- [x] TASK-008: Update `smoke.rs` (remove `test_context_scope`, rewrite as explicit carry)
- [x] TASK-009: Update `golden_codegen.rs` + regenerate insta snapshots
- [x] TASK-010: Verify `interceptor_invocation.rs` clean (no changes expected)
- [x] TASK-011: Verify `security_integration.rs` clean (no changes expected)
- [x] TASK-012: Rewrite `examples/order_service.rs` (remove `scope()`/`current()`)
- [x] TASK-013: Update `COOKBOOK.md` (replace mermaid + code snippet)

### Phase 4 — Verification Gates

- [x] TASK-014: `rg "ServiceContext::current|ServiceContext::scope|CURRENT_CONTEXT" crates/` returns zero
- [x] TASK-015: `cargo fmt --check && cargo clippy --all-targets --all-features && cargo test --workspace` all exit 0
- [x] TASK-016: Code review — no new sync primitives in `crates/service-sdk/src/`
- [x] TASK-017: Code review — `ServiceContext` absent from domain-layer files
