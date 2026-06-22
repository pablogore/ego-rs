# Proposal: CORE-010A — Remove Ambient ServiceContext

## Intent

Remove all ambient `ServiceContext` access (`tokio::task_local! CURRENT_CONTEXT`, `ServiceContext::current()`, `ServiceContext::scope(...)`) and standardize on explicit propagation. Ambient state violates the explicit-dependency principle from CORE-009 and the project's "no task-local state" constraint, hides execution inputs, and does not auto-propagate across spawned tasks. After this change the runtime has a single context model: explicit ownership and parameter passing, with zero ambient execution state.

## Problem

`ServiceContext` can be obtained without being passed explicitly across function boundaries, introducing hidden dependencies, reduced execution transparency, non-obvious behavior, propagation bugs across task boundaries, and architectural inconsistency with explicit DI.

> **Premise correction (verified)**: The source brief states "only test-only usage." This is FALSE. The proxy code-generation macro (`crates/service-sdk-macros/src/lib.rs:119-149`) generates PRODUCTION code that calls `ServiceContext::current()` for `enforce_tenant()` and re-reads it inside `scope()` to feed the interceptor chain (`on_request`/`on_response`/`on_error`). This is a load-bearing tenant-enforcement path, not test-only cleanup.

## Scope

### In Scope
- Remove `CURRENT_CONTEXT` task-local, `ServiceContext::current()`, `ServiceContext::scope()` (`crates/service-sdk/src/context/mod.rs`).
- Rewrite the proxy code-gen macro to thread `ServiceContext` explicitly through generated forwarding methods (enforce_tenant + interceptor chain), preserving current behavior.
- Update examples (`order_service.rs`), `COOKBOOK.md`, and all tests (`context_scope.rs`, `context_propagation.rs`, `context_cross_service.rs`, `deadline_expiry.rs`, `proxy_codegen.rs`, `smoke.rs`) to explicit construction/propagation.

### Out of Scope
- `ServiceContext`, `SecurityContext`, `RuntimeBuilder` redesign; DI framework, telemetry, or actor-lifecycle changes.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `service-sdk`: ambient context access requirements removed; proxy/interceptor context acquisition becomes explicit (delta REMOVES task-local/`current()`/`scope()` requirements and MODIFIES proxy dispatch to receive context explicitly).
- `security-sdk`: aligns/strengthens existing "no task-local" invariant for the security field carried in `ServiceContext` (verify no requirement regression).

## Approach

Delete ambient APIs; change generated proxy methods to acquire `ServiceContext` from an explicit owned/parameter source and pass `&ctx` directly to `enforce_tenant` and the interceptor chain, removing the `scope()` wrap and inner re-read. Rewrite tests to build/pass context explicitly. Verify with `grep` that no task-local/`current()`/`scope()` remains.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/service-sdk/src/context/mod.rs` | Removed | task-local, `current()`, `scope()` |
| `crates/service-sdk-macros/src/lib.rs` | Modified | explicit ctx threading in proxy gen |
| `crates/service-sdk/tests/*`, `examples/order_service.rs`, `COOKBOOK.md` | Modified | explicit propagation |
| `openspec/specs/{service-sdk,security-sdk}/spec.md` | Modified | delta specs |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Behavioral regression in tenant enforcement / interceptor chain | High | Preserve order; integration tests assert tenant/trace propagation unchanged |
| Source brief understated scope (macro is production) | Confirmed | Proposal corrects premise; spec/design must cover macro rewrite |
| Spawned-task context loss | Med | Explicit capture/ownership per FR-005 |

## Rollback Plan

Pure refactor on a feature branch. Revert the branch/commits to restore task-local, `current()`, `scope()`, and prior macro output. No data, schema, or API-consumer migration involved. `cargo test --workspace` on `develop` confirms restored state.

## Dependencies

- CORE-009 (explicit-dependency constraint) — this change enforces it.

## Success Criteria

- [ ] No usage of `ServiceContext::current()` or `ServiceContext::scope(...)` in the workspace.
- [ ] No task-local `ServiceContext` implementation remains.
- [ ] Generated proxies propagate context explicitly; tenant + interceptor behavior unchanged.
- [ ] `cargo fmt`, `cargo clippy --all-targets --all-features`, `cargo test --workspace` pass.

## Proposal question round

Non-interactive run; assumptions needing user confirmation:
1. Macro rewrite (production code) is in scope — the brief's "test-only" claim is corrected here. Confirm.
2. Generated proxy methods should receive `ServiceContext` via an explicit parameter/owned source rather than reading ambient state — confirm the preferred threading shape before design.
3. `COOKBOOK.md` examples using `current()` must be rewritten — confirm docs are in scope for this change.
