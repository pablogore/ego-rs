# Design: CORE-012A — Observability Integration (Security Enforcement Path)

## Technical Approach

Implement explore.md **Approach 2**: both macro guard blocks emit one line each into a
single, ordinary-Rust, unit-testable `RuntimeInner::record_security_denial` helper. Wire
the domain `Observability` port into `RuntimeInner` as an optional field (mirroring
`authorization_provider()`), settable via `RuntimeBuilder::with_observability(..)`, default
`None`. Redaction reuses the AD-010 `Display`/`Debug` split. Satisfies spec requirements
1–5 with no new port, no adapter, no `CrossTenantDenied` path, no chain-bypass fix.

## Architecture Decisions

### AD-1 — Shared `RuntimeInner` helper, not inline codegen; input is a purpose-built `SecurityDenialKind`, not `&SecurityError`
**Choice:** `pub fn record_security_denial(&self, service: &'static str, operation: &'static str, kind: SecurityDenialKind)` on `RuntimeInner` (runtime_builder.rs), `#[doc(hidden)]` for macro-visibility (same contract as `authorization_provider`/`logger`). New type, defined alongside the helper:

```rust
#[derive(Debug, Clone, Copy)]
pub enum SecurityDenialKind {
    MissingContext,
    TenantMismatch,
    AuthorizationDenied,
}
```

Fieldless by design (see AD-3) — no `expected`/`actual`/`reason` payload.

**Addendum (code-review fix):** the `SecurityError → SecurityDenialKind` mapping used at both macro call sites is centralized as `SecurityDenialKind::from_security_error(err: &SecurityError) -> Option<Self>` on `runtime_builder.rs`, rather than duplicating an inline `match &e { .. }` inside each `quote!{}` block. Both call sites call `if let Some(kind) = SecurityDenialKind::from_security_error(&e) { ... }` — one shared, unit-testable classification instead of two hand-written copies inside proc-macro token trees.

**Rejected:**
- inline `observability().trace(..)` in codegen (Approach 1 — untestable generated code, double-log risk).
- `&SecurityError` as the parameter type (earlier draft of this AD) — `service-sdk` already depends on `ego-security-sdk` (Cargo.toml:16), so this is not a new crate-dependency edge, but `SecurityError` has 9 variants and only 3 are valid denial kinds for this spec; a helper typed `&SecurityError` compiles for `ProviderError`/`InvalidCredential`/etc. too, silently allowing calls that violate the spec's 3-kind contract.
- A field-carrying `SecurityDenialKind` (earlier draft of this AD, `TenantMismatch { expected, actual }` / `AuthorizationDenied { reason }`) — reviewed and rejected (see AD-3): nothing in the observability flow ever reads those fields, so cloning `String`s into them is cost with no payoff.
**Rationale:** logic lives in testable Rust; each call site stays one line; matches minimal-codegen style. `SecurityDenialKind` is exhaustively matched over exactly the 3 spec-scoped cases — the compiler rejects any input outside that set. The guard remains solely responsible for deciding *what* happened and *why* (and for constructing the `SecurityError` it independently returns via `?`, unchanged by this design); `RuntimeInner` remains solely responsible for *emitting observability* — it receives only the tag it needs, never inspects error internals, and never carries a copy of data it doesn't use.

### AD-2 — Default is `Option<Arc<dyn Observability>> = None`, not the infra `NoopObservability`
**Choice:** field `observability: Option<Arc<dyn Observability>>`, default `None`; helper is `if let Some(obs) = &self.observability { obs.trace(ev) }`. `with_observability(Arc<dyn Observability>)` sets `Some`.
**Rejected:** importing `ego_infrastructure::NoopObservability` as the concrete default.
**Rationale:** `ego-service-sdk` depends on `ego-domain` but **not** `ego-infrastructure` (Cargo.toml:17). Defaulting to the infra type would add a new service-sdk→infrastructure crate edge (a layering inversion) **that explore.md did not flag**. `None`⇒no-op is byte-for-byte identical to Noop discarding events, exactly mirrors the requested `authorization_provider()` Option pattern, keeps infra unchanged, and leaves `NoopObservability` the sole concrete implementor (callers may still pass it explicitly). Deviates from the proposal's literal "default to NoopObservability" wording; behaviorally equivalent.

### AD-3 — Redaction via `Display` label only; no shadow copy of raw detail
**Choice:** `impl std::fmt::Display for SecurityDenialKind` directly in runtime_builder.rs (no wrapper type). Emits **only the kind label** (`"MissingContext"`/`"TenantMismatch"`/`"AuthorizationDenied"`). `SecurityDenialKind` (AD-1) is fieldless, so there is no raw tenant id / denial reason anywhere in this type to redact or retain — the redaction guarantee reduces to "the label is all there is."

Full diagnostic detail (raw `expected`/`actual`/`reason`) is **not duplicated into this path at all**. It remains available exactly where it already lived before this change: in the `SecurityError` value the guard independently constructs and returns to the caller via `?`, whose own `Debug` impl (pre-existing, AD-010, CORE-008A) already retains it. Spec requirement 3's "Debug retains raw identifiers" scenario is satisfied by that pre-existing value, not by anything this change introduces (see the revised spec scenario).
**Rejected:**
- using `SecurityError`'s own `Display` for the recorded label — `AuthorizationDenied`'s is `"authorization denied: {reason}"` and **leaks the reason** (error/mod.rs:24); `TenantMismatch`/`CrossTenantDenied` are already redacted but the type is non-uniform. Hence the hand-written label-only `Display` above.
- a field-carrying `SecurityDenialKind` whose `Debug` duplicates `expected`/`actual`/`reason` (earlier draft) — reviewed: nothing in the observability flow reads that `Debug` output (`SecurityDenialKind`'s `Display`, the only thing embedded in the event, never interpolates fields regardless of whether they exist), and the original `SecurityError` already provides the same diagnostic guarantee independently. Cloning `String`s solely to duplicate an already-available capability is cost without payoff.
- a separate `RecordedDenial<'a>(&'a SecurityDenialKind)` newtype wrapping `SecurityDenialKind` to carry the `Display` impl (earlier draft, implemented then removed during code review) — since the kind is fieldless there is nothing left for a wrapper to redact; `impl Display for SecurityDenialKind` directly does the same job with one fewer type and no dedicated wrapper test.
**Rationale:** the fieldless kind means correctness here can't regress by construction (no field to accidentally leak into `Display`), and this change avoids introducing a second, redundant carrier of sensitive data — or a redundant wrapper type — it doesn't need.

**Event name stability (non-normative note):** `event_name` is fixed at `"security.denial"` for every current and future denial kind — new denial kinds (e.g. a future `CrossTenantDenied` instrumentation) MUST differentiate solely via the `denial_kind` metadata field, never by forking into per-kind event names (`security.tenant_denial`, `security.authorization_denial`, etc.), which would silently break event-name-keyed aggregation/dashboards.

## Data Flow

    #[authorize] guard ──(denied: MissingContext | AuthorizationDenied)──┐
                                                                          ├─► __rt.record_security_denial(TRAIT, METHOD, kind)
    #[tenant_scoped] guard ──(denied: TenantMismatch)─────────────────────┘        │
      (only reached if authorize passed → at most one call)                        ▼
                                              kind.to_string() [Display] → metadata["denial_kind"]
                                              SemanticEvent{ event_name:"security.denial", metadata } → observability?.trace()

## Event construction (minimum contract → `SemanticEvent`)

Helper builds `SemanticEvent::new("security.denial", "", "", "Denied", "", metadata)` where
`metadata = { "denial_kind": kind.to_string(), "service": service, "operation": operation }`.
The 3 required fields (denial_kind, service, operation) live in `metadata`; `event_name` is the
stable non-empty label the fail-closed constructor requires. `correlation_id`/`actor_id`/`timestamp`
stay empty and `tenant` is absent — all optional per spec (a real clock/correlation source is out of scope).

## Double-attribute short-circuit (spec req 1)

Guaranteed by existing codegen order, not new logic: the forwarding method emits
`#authorize_guard` then `#enforce_tenant_block` (lib.rs:384–385); every denial path in each block
`?`-returns. If authorize denies, the method returns before the tenant block runs (one call). If
authorize passes, it recorded nothing and only the tenant block can record (one call). Cited, not asserted.

## Call-site changes (lib.rs:264–401)

| Site | Denial | Change |
|---|---|---|
| authorize `ctx.security()` missing (~285) | `MissingContext` | `.ok_or_else(..)?` → `match`; on `None`, `if let Some(__rt)=upgrade() { __rt.record_security_denial(stringify!(#trait_name), stringify!(#method_name), SecurityDenialKind::MissingContext) }` then return |
| `authorize_in_context(..)` fail (~312) | `AuthorizationDenied` only (verified: `authorize_in_context` returns `Result<(), SecurityError>` — `e` may also be `CapabilityNotEnabled`/`ProviderError`, which are infra failures, not denials) | `map_err(\|e\| { if let Some(kind) = SecurityDenialKind::from_security_error(&e) { __rt.record_security_denial(TRAIT, METHOD, kind); } <#err_ty>::from(e) })?` (reuse already-upgraded `__rt`; the centralized `from_security_error` mapping — code-review fix — returns `None` for the infra-failure arms per the exclusion below, so nothing is recorded for those) |
| `enforce_tenant(..)` fail (~357-358) | `TenantMismatch` **or** `MissingContext` (verified: `enforce_tenant` → `tenant_resolver.resolve(..)` returns `Err(SecurityError::MissingContext)` for an unresolvable/unauthenticated context, `Err(SecurityError::TenantMismatch{expected,actual})` for a hard mismatch — both spec-in-scope kinds surface from this one `?` site) | `map_err(\|e\| { if let Some(kind) = SecurityDenialKind::from_security_error(&e) { __tenant_rt.record_security_denial(TRAIT, METHOD, kind); } <#err_ty>::from(e) })?` — same centralized mapping as the row above, shared by both call sites instead of two independent inline `match` blocks |

**Note — a second, distinct `MissingContext` site exists and is deliberately excluded**: `self.runtime.upgrade().ok_or_else(MissingContext)` at lib.rs:351-355 (both guard blocks have this pattern) fires when the weak `Runtime` handle itself is dropped — an infra/lifecycle failure reusing the `MissingContext` variant for convenience, not an actual "no security context was supplied" denial. Treat this the same as `ProviderError`/`CapabilityNotEnabled` below: excluded from instrumentation. Only the `ctx.security().ok_or_else(MissingContext)` site (line 285, row 1 above) and the `enforce_tenant` `map_err` site (row 3 above) represent the spec's genuine `MissingContext` denial.

`ServiceContext` needs **no** observability accessor — the helper reads `observability` via the
already-upgraded `RuntimeInner` (`__rt`/`__tenant_rt`). The explore.md logger/observability accessor
gap stays out of scope. `ProviderError`/`CapabilityNotEnabled`/dropped-runtime paths are infra failures, not the three
spec denial kinds — left uninstrumented. `CrossTenantDenied` unreachable — untouched (spec req 5).

## File Changes

| File | Action | Description |
|---|---|---|
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modify | `observability` field, `SecurityDenialKind` enum + `Display` impl, `SecurityDenialKind::from_security_error` mapping, `record_security_denial` helper + tests |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | `observability` field + `with_observability(..)`; pass into `new_with_logger` |
| `crates/service-sdk-macros/src/lib.rs` | Modify | recording calls at 2 guard call sites (one matches 2 outcomes) in guard error paths |

## Interfaces / Contracts

```rust
// RuntimeInner
#[derive(Debug, Clone, Copy)]
pub enum SecurityDenialKind {
    MissingContext,
    TenantMismatch,
    AuthorizationDenied,
}
impl SecurityDenialKind {
    pub fn from_security_error(err: &SecurityError) -> Option<Self>;
}
pub fn record_security_denial(&self, service: &'static str, operation: &'static str, kind: SecurityDenialKind);
// RuntimeBuilder
pub fn with_observability(self, obs: Arc<dyn ego_domain::Observability>) -> Self;
```

## Testing Strategy

| Layer | What | Approach |
|---|---|---|
| Unit | `SecurityDenialKind` `Display` redaction (req 3, event-side) | `Display` yields only the kind label for each of the 3 variants, never any field-derived text (there are no fields to leak) |
| Unit | `SecurityError`'s existing `Debug` (req 3, diagnostic-side) | pre-existing AD-010 test coverage already asserts raw `tenant_id`/`reason` appear in `Debug` — no new test needed, cited as already-satisfied |
| Unit | helper (reqs 1,2) | call directly with each `SecurityDenialKind` variant; assert one event, 3 fields present |
| Integration | guard wiring (reqs 1,5) | `#[service]` trait w/ both attrs + `RecordingObservability` test double; assert exactly one event per denied call, allowed=none |
| Integration | Noop default (req 4) | build without `with_observability`; assert identical error/return, no panic |

**Test double:** none exists in the codebase — add a small `RecordingObservability { events: Mutex<Vec<SemanticEvent>> }` in service-sdk test scope (dev-only). **Snapshot churn:** the 3 guard error paths change shape, so `proxy_codegen`/golden output for authorize/tenant-guarded methods regenerates deliberately; `golden_codegen` (descriptor-only) is unaffected. Keep each recording call a single appended closure to minimise diff.

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, executable-file classification, or process-integration boundary.

## Migration / Rollout

No migration. Additive; default `None` ⇒ today's behavior. Revert commits to roll back.

## Open Questions

- [x] AD-2 deviates from the proposal's literal "default to NoopObservability" (behaviorally equivalent, avoids a new infra dependency edge). **Resolved — user confirmed**: accept `Option<Arc<dyn Observability>> = None` over importing the infra type, to keep the service-sdk→infrastructure layering boundary intact.
