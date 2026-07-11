# Exploration: CORE-012A — Observability Integration (Security Enforcement Path)

## Objective (as stated)

Instrument observability for `#[authorize]` and `#[tenant_scoped]` macro-driven enforcement:

- Log `MissingContext`, `TenantMismatch`, `CrossTenantDenied`, `AuthorizationDenied` outcomes
- Redact sensitive data in those logs
- Integrate with the existing `Observability` port (no OpenTelemetry yet — that belongs to a separate future change, CORE-022)

## Current State

`#[authorize]`/`#[tenant_scoped]` expand inside `#[service]` trait methods (`crates/service-sdk-macros/src/lib.rs:264-401`). Generated method body order: `authorize_guard` (283-312) → `enforce_tenant_block` (351-358) → only then `chain_ref.on_request/on_response/on_error` (388-398).

**Confirmed, not stale**: both guard blocks `?`-return before the interceptor chain ever runs, so `MissingContext`, `TenantMismatch`, `AuthorizationDenied` never reach `InterceptorChain::on_error` (`crates/service-sdk/src/interceptor/chain.rs:112-124`) — that hook only sees errors from the actual business call.

**Correction to the 2026-07-09 audit**: `CrossTenantDenied` is not reachable via the macros at all today. It's only constructed in `RuntimeInner::issue_cross_tenant_permit` (`crates/service-sdk/src/runtime/runtime_builder.rs:315-344`), which is `pub(crate)`, `#[allow(dead_code)]`, and documented as "used only in tests ... this framework-stage codebase has no application services yet" (runtime_builder.rs:311-314). `TenantResolver::resolve` (`tenant.rs:172-237`) never returns it.

**TracingInterceptor still a commented-out stub** — `crates/service-sdk/src/interceptor/builtin/mod.rs` is unchanged (5 lines, all commented).

**Observability port is entirely unwired, not just Noop-only** (new finding beyond the prior audit): `RuntimeInner` (`runtime_builder.rs:120-140`) has no `observability` field — only `logger: Option<Arc<KITLogger>>`, a separate already-wired infra logger, distinct from the domain `Observability` trait. No `RuntimeInner::observability()` accessor exists (compare `authorization_provider()` at 262-264), no `RuntimeBuilder::with_observability(...)`, and `ServiceContext` has a `logger` field/accessor but no `observability` one. `NoopObservability` (`crates/infrastructure/src/observability.rs`) has zero production call sites — CORE-008A/B never touched this gap.

**Port shape** (`crates/domain/src/observability.rs:169-186`): `trace(SemanticEvent)`, `metric(name, value)`, `log(Level, message)`; `SemanticEvent` carries `event_name/correlation_id/actor_id/lifecycle_state/timestamp/metadata`.

**Redaction precedent exists, no dedicated module**:
1. `SecurityError` AD-010 convention (`crates/security-sdk/src/error/mod.rs:47-75`) — `Display` omits raw tenant ids/reasons, `Debug` retains them for diagnostics.
2. `[REDACTED]` Debug-placeholder convention for secrets (`crates/domain/src/auth/credential.rs:42-52`, `crates/security-jwt/src/oidc_config.rs:73`, `introspection.rs:40`) — opposite discipline (Debug itself redacted).

The design must pick/reconcile one of these for observability event fields.

`docs/architecture.md:89,118` — checked, generic module/dependency prose, not a stale observability claim.

## Affected Areas

- `crates/service-sdk-macros/src/lib.rs:264-401` — the two denial-emission sites (shared `forwarding_methods.push(quote!{...})` template — instrument once here, not per-macro).
- `crates/service-sdk/src/runtime/runtime_builder.rs:120-140,262-264` — needs `observability` field + accessor.
- `crates/service-sdk/src/runtime/builder.rs:40-232` — needs `with_observability(...)`.
- `crates/service-sdk/src/interceptor/chain.rs`, `.../builtin/mod.rs` — chain bypass is a separate, related gap (not this change's stated scope).
- `crates/domain/src/observability.rs` — port sufficient as-is.
- `crates/infrastructure/src/observability.rs` — `NoopObservability` unchanged; no real implementor exists or is in scope.
- `crates/security-sdk/src/error/mod.rs:47-75`, `crates/domain/src/auth/credential.rs`, `crates/security-jwt/src/{oidc_config,introspection}.rs` — reusable redaction precedents.

## Approaches

1. **Inline instrumentation in macro guard blocks** — call `__rt.observability()` directly inside `authorize_guard`/`enforce_tenant_block`. Pros: single shared macro path. Cons: hard to unit-test generated code directly; double-log risk if both attrs present. Effort: Medium.
2. **Shared `RuntimeInner` helper called from both guard sites** (e.g. `record_security_denial`) — macro calls stay one line each; logic lives in ordinary testable Rust. Pros: matches existing minimal-codegen style, independently unit-testable. Cons: still two call sites. Effort: Medium.
3. **Also fix the interceptor-chain bypass in this change** — Pros: closes the full structural gap. Cons: explicitly out of stated scope, conflates two gaps, no observable payoff without also un-stubbing `TracingInterceptor`. Effort: High — recommend a separate future change.

## Recommendation

Approach 2 — thin macro call sites into a shared, testable `RuntimeInner` helper; smallest new surface (one field + accessor + helper), consistent with existing style. Do not fold in the interceptor-chain fix (approach 3).

## Risks

- `CrossTenantDenied` has no production caller yet — instrumenting it now is speculative; proposal must decide scope.
- Two competing redaction conventions in the codebase — pick and justify one for observability fields.
- `NoopObservability` remains the only implementor — this change wires call sites but produces no visible logs without a real adapter; state this explicitly to reviewers.
- Double-invocation risk when both `#[authorize]` and `#[tenant_scoped]` are present on the same method.

## Ready for Proposal

Yes.
