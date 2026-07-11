# Proposal: CORE-012A — Observability Integration (Security Enforcement Path)

Make macro-driven security denials observable through the existing `Observability` port. Evidence base: `explore.md` (this folder) — cited, not re-derived.

## Intent

Security denials are invisible today. The `#[authorize]` and `#[tenant_scoped]` guard blocks `?`-return **before** the interceptor chain runs, so `MissingContext`, `TenantMismatch`, and `AuthorizationDenied` never reach `InterceptorChain::on_error` (explore.md: `service-sdk-macros/src/lib.rs:264-401`, `chain.rs:112-124`). Compounding this, the `Observability` port is entirely unwired — no `RuntimeInner` field, no builder method, no `ServiceContext` accessor; `NoopObservability` has zero production call sites. An operator cannot answer "who was denied, when, and why" for any enforcement outcome.

## Scope

### In Scope
- Record the three **reachable** denial outcomes — `MissingContext`, `TenantMismatch`, `AuthorizationDenied` — through the existing `Observability` port at the macro guard sites.
- Approach 2 from explore.md: thin one-line call sites in both macro guard blocks into one shared, independently testable `RuntimeInner` helper (e.g. `record_security_denial`).
- Wire the port: `RuntimeInner` observability field + accessor (mirroring `authorization_provider()`), `RuntimeBuilder::with_observability(...)`, defaulting to no-recording behavior (behaviorally equivalent to `NoopObservability`; design.md AD-2 decides the concrete mechanism).
- Redaction per the **AD-010 `SecurityError` convention** (`security-sdk/src/error/mod.rs:47-75`): recorded event data carries the `Display`-safe form — no raw tenant ids or denial reason strings; full detail stays in `Debug` for internal diagnostics only. The competing `[REDACTED]`-placeholder convention (credential.rs/oidc_config.rs) is explicitly **rejected** for observability fields.

### Out of Scope (non-goals)
- **`CrossTenantDenied` instrumentation** — dead code today (`#[allow(dead_code)]`, test-only, no production caller; explore.md correction). Deferred until a real caller exists.
- **Interceptor-chain bypass fix** — separate structural gap. This change adds a direct instrumentation call in the guard blocks; it does NOT route denials through the (bypassed) chain.
- **Un-stubbing `TracingInterceptor`.**
- **OpenTelemetry export** — CORE-022.
- **Any real (non-Noop) `Observability` adapter** — this change wires call sites against the existing port only. Explicit call: a production adapter is a separate change; trivial in-scope inclusion is declined to keep this slice reviewable.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `service-sdk`: new requirements — (1) each reachable macro-guard denial produces exactly one recorded observability event; (2) recorded denial events are redacted per AD-010; (3) runtime accepts an `Observability` implementor at build time, defaulting to no-recording behavior with unchanged existing behavior. Existing guard-order and denial-semantics requirements are preserved unchanged.

## Observable Contract (what must be true)

- A denied invocation produces **exactly one** recorded event — the guards short-circuit (`authorize` runs first, `tenant` never runs after a denial), so double-recording when both attributes are present is prevented by construction and MUST be verified by test.
- Every recorded security-denial event MUST contain: denial kind, service name, operation name. Additional contextual fields (`correlation_id`, `actor_id`, `tenant`, metadata, etc.) are implementation-defined and specified by design — their absence does not violate this contract.
- Sensitive values (tenant ids, denial reasons) never appear in recorded event data (AD-010 `Display`/`Debug` split).
- Allowed invocations record nothing new; denial semantics, error values, and guard order are unchanged.
- With the default `NoopObservability`, behavior is byte-for-byte today's behavior.

## Affected Areas

| Area | Impact |
|---|---|
| `crates/service-sdk-macros/src/lib.rs:264-401` | Modified — one-line call in each guard block (shared template) |
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modified — observability field, accessor, denial-recording helper |
| `crates/service-sdk/src/runtime/builder.rs` | Modified — `with_observability(...)` |
| `crates/domain/src/observability.rs` | Unchanged — port sufficient as-is |
| `crates/infrastructure/src/observability.rs` | Unchanged — Noop remains sole implementor |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| No visible output without a real adapter — reviewers must not expect logs; this change wires call sites, Noop swallows events | High (certain) | Stated explicitly here; success criteria assert recording via a test double, not log inspection |
| Double-recording when both attributes present | Low | Prevented by guard short-circuit; regression test required |
| Sensitive data leaks into events | Low | AD-010 convention + explicit redaction scenario in spec |
| Macro codegen churn breaks golden/snapshot tests | Med | Call sites are one line in the shared template; goldens updated deliberately |

## Rollback

Additive surface. Revert the change commits; default-Noop wiring means no caller depends on emitted events yet. No data or contract migration.

## Dependencies

- explore.md (this folder) — complete.
- None external. CORE-022 (OpenTelemetry) builds on this later.

## Success Criteria

- [ ] Each of `MissingContext`, `TenantMismatch`, `AuthorizationDenied` from a macro guard produces exactly one recorded event, asserted via a test-double `Observability` implementor.
- [ ] Every recorded event contains denial kind, service name, and operation name (the minimum required contract), verified by test.
- [ ] A method with both `#[authorize]` and `#[tenant_scoped]` records exactly one event per denied call.
- [ ] Recorded event data contains no raw tenant ids or denial reason strings (AD-010 redaction verified by test).
- [ ] Runtime builds without `with_observability(...)` and behaves identically to today (Noop default).
- [ ] `CrossTenantDenied` remains uninstrumented, with the deferral documented.
