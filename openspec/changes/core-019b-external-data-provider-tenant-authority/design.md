# Design: CORE-019B — External Data Provider Tenant Authority

## Technical Approach

The runtime provider-access chokepoint becomes the single authority for the
fetch tenant. Today the chokepoint is tenant-blind: `RuntimeDataProviderAccess`
holds only a registry and a config (`crates/runtime/src/providers/access.rs:279-285`)
and its `fetch` forwards the caller's `DataRequest` verbatim
(`access.rs:289-361`; `provider.fetch(request.clone())` at `:329`). The fetch
tenant therefore has whatever authority the caller chose to give it via
`DataRequest::new` / `for_tenant` / `with_tenant`
(`crates/persistent-entity/src/data_provider_access.rs:45-67`).

This design captures the **established** context tenant on an immutable
per-dispatch wrapper (ADR-3) and makes the shared chokepoint reconcile
`request.tenant` against it **inside** the observable fetch lifecycle (ADR-1), so
a fail-closed tenant decision still emits exactly one terminal
`data_provider_fetch` signal. The established tenant is the same one the actor
already treats as authoritative for effects: it is derived from
`self.entity_id.tenant_id` and validated via `ego_domain::TenantId::new`, failing
closed on an invalid identity (`crates/persistent-entity/src/actor.rs:326-339`).
`ServiceContext` already models the authoritative-vs-hint distinction this change
extends to the fetch path — `canonical_tenant()` (authoritative, resolver-produced)
versus `tenant_hint()` (non-authoritative ingress input)
(`crates/service-sdk/src/context/mod.rs:52-65,382-402`). `TenantId` itself is the
validated newtype `id_type!(TenantId, ...)` in
`crates/domain/src/context.rs:56`.

On the authorized (success) path the provider is invoked with an already-authorized
tenant through the unchanged #234 per-attempt timeout / bounded retry-backoff loop.
A fail-closed tenant decision short-circuits *before* entering that loop (zero
retries, zero provider calls) but still lands on the same shared terminal-signal
emitter, so #234's retry/timeout behavior is untouched while the fail-closed
outcome remains observable.

## Architecture Decisions

### ADR-1 (CORE DECISION): Injection vs Validation vs Both → **Both — inject-when-absent + fail-closed-validate-when-present**

**Choice**: the chokepoint reconciles `request.tenant` against the captured
authoritative context tenant `A` per this frozen **five-row matrix** (identical
in the proposal and the spec):

| Established context | `request.tenant` (caller) | Action | Tenant handed to provider |
|---|---|---|---|
| `Some(A)` | `None` | **Inject** `A` | `Some(A)` |
| `Some(A)` | `Some(A)` (matches) | Pass | `Some(A)` |
| `Some(A)` | `Some(B)`, `B != A` | **Fail closed** `TenantMismatch` | — (no fetch) |
| `None` (tenant-agnostic / single-tenant) | `None` | Pass (no-op) | `None` |
| `None` | `Some(C)` | **Fail closed** `TenantContextMissing` | — (no fetch) |

**Rejected**: (A) pure inject — always overwrite `request.tenant` with `A`,
ignoring caller input; (B) pure validate — require the caller to always supply
`request.tenant` and reject on mismatch or absence.

**Security rationale (fail-closed)**: a mismatch is the exact signature of a
spoofing attempt or a genuine cross-tenant bug. Pure inject would *silently*
overwrite `Some(B)` with `A` and proceed — the fetch succeeds against the right
tenant, but the wrong intent is swallowed, so an attack or bug leaves no signal
and no error. Failing closed surfaces it loudly. Pure inject is safe for
confidentiality but hostile to detection; this design keeps the confidentiality
(provider never sees `B`) *and* the detection (mismatch is an error).

The last row is the correction demanded by review: a caller that asserts a tenant
`C` when **no** authority is established cannot have that assertion validated
against anything, so accepting it would reintroduce exactly the caller-chooses-
the-tenant seam this change closes. It therefore **fails closed** with a
*distinct* error, `TenantContextMissing` — distinct from `TenantMismatch` because
there is no established tenant to mismatch *against*. Tenant-agnostic mode is
still perfectly preserved by the fourth row: `DataRequest::new(...)` produces
`tenant: None`, which passes as `None`; preserving that mode does **not** require
accepting an arbitrary caller-supplied tenant.

| Option | Tradeoff | Verdict |
|---|---|---|
| **Both** (inject-absent + validate-present, fail-closed on both mismatch and unauthorized assertion) | Provider never receives a caller-chosen tenant (confidentiality); every disagreeing or unauthorized caller value is surfaced as an explicit error (detection); the tenant-agnostic `None` path and existing callers keep working (compatibility). Mirrors the actor effect path's established-tenant-plus-fail-closed discipline (`actor.rs:326-339`). | **Chosen** |
| A. Pure inject (always override) | Simplest; guarantees the provider sees only `A`. But silently discards a disagreeing `Some(B)` — a spoof or a cross-tenant bug produces no error, defeating detection. Also makes `for_tenant`/`with_tenant` semantically dead (their argument is always ignored). | Rejected |
| B. Pure validate (require caller to supply, reject on mismatch/absence) | Strongest caller discipline, but breaks the tenant-agnostic `DataRequest::new` → `None` path and forces every existing caller to thread the tenant — a churny breaking change; and a caller that forgets either fails (breaks single-tenant) or must fall back (defeats enforcement). | Rejected |

**Placement — reconciliation runs INSIDE the observable fetch lifecycle.**
Reconciliation is NOT a pre-check that returns before observability. It runs
inside the same `fetch` lifecycle that emits the terminal `data_provider_fetch`
signal, so a fail-closed tenant decision (`TenantMismatch` / `TenantContextMissing`)
emits **exactly one** terminal signal, classified as a **distinct non-retryable
outcome**, with **zero** retries and **zero** provider invocations. A cross-tenant
attempt is therefore alertable through the existing #234 pipeline. The #234
retry/timeout loop itself is unchanged — a fail-closed decision short-circuits it
(never enters the loop, never calls a provider) but still lands on the shared
terminal-signal emitter. See ADR-1a and the Observability section.

### ADR-1a: Fail-closed outcomes are first-class `ProviderOutcome` classifications

`ProviderOutcome` gains two non-retryable values — `TenantMismatch` and
`TenantContextMissing` — alongside the existing `ProviderMissing` /
success/failure/timeout classifications. `is_retryable()` returns `false` for
both. On a fail-closed tenant decision the chokepoint calls the SAME `log_fetch`
terminal emitter (`access.rs:348-355`) with `attempts = 1`, `cache_hit = false`,
and the tenant outcome, then returns the corresponding `DataProviderError`. This
reuses the one terminal-signal path rather than adding a parallel one, keeping
the "exactly one `data_provider_fetch` per fetch" invariant intact.

### ADR-2 (DECISION): Compatibility / migration of `DataRequest` constructors → **Preserve constructors; reinterpret `tenant` from trusted authority to validated assertion**

**Choice**: keep `DataRequest::new`, `DataRequest::for_tenant`, and
`DataRequest::with_tenant` with their current signatures
(`data_provider_access.rs:45-67`); no source break for existing callers. What
changes is the *meaning* of `DataRequest.tenant`: it stops being "the
authoritative identity the caller is trusted to have threaded" and becomes "a
caller assertion the runtime validates against the established context". The
field doc (`data_provider_access.rs:31-38`) is updated to say so, and two
fail-closed variants — `DataProviderError::TenantMismatch` and
`DataProviderError::TenantContextMissing` — are added to the port error enum
(`data_provider_access.rs:122-157`).

**Rejected**: removing `for_tenant` / `with_tenant` (forces every current caller
to migrate and removes the ability to *assert* an expected tenant, which is a
useful defense-in-depth signal); making `tenant` private / builder-only (a
larger public break for no enforcement benefit, since the chokepoint enforces
regardless of how the field was set).

**Concrete migration impact**:

- Tenant-agnostic callers (`DataRequest::new`, tenant `None`) — **no change**,
  compile and behave unchanged (frozen decision 4).
- Callers that pass a tenant equal to their established context — **no change**;
  the value now round-trips through validation instead of being trusted, but the
  observable result is identical.
- Callers that pass a *different* tenant than their context — **behavior change**:
  previously the forged tenant reached the provider; now the fetch fails closed
  with `TenantMismatch`. This is the intended fix; any such caller was already
  violating the documented convention.
- Callers that assert a tenant with **no** established authority
  (`context=None`, `request=Some(C)`) — **behavior change**: previously `C`
  reached the provider; now the fetch fails closed with `TenantContextMissing`.
  Tenant-agnostic callers using `DataRequest::new` (tenant `None`) are unaffected.
- `DataProviderError::TenantMismatch` and `DataProviderError::TenantContextMissing`
  are additive. Exhaustive matches on `DataProviderError` in downstream code must
  add arms — a minor, mechanical, compile-time-caught change; documented in
  Migration below.

### ADR-3 (DECISION): How the authoritative tenant reaches the tenant-blind chokepoint → **An immutable PER-DISPATCH wrapper (never a singleton field)**

The port method `DataProviderAccess::fetch(&self, provider_id, request)`
(`data_provider_access.rs:191-195`) carries no context, and handlers hold
`Arc<dyn DataProviderAccess>` — so the chokepoint cannot read an ambient tenant
and must not add one to the handler-facing signature (that would let the handler
supply it, reintroducing the forgery seam).

**Choice (frozen)**: a dedicated immutable wrapper type,
`TenantScopedDataProviderAccess`, constructed **once per dispatch**. It holds an
`Arc` to the shared, tenant-blind `RuntimeDataProviderAccess` (registry + config,
one instance for the whole runtime) and captures the established tenant
`Option<TenantId>` for **that one invocation** from the entity/command context
(`self.entity_id.tenant_id` / the resolved `canonical_tenant`,
`actor.rs:106,231,327`). It implements `DataProviderAccess` by delegating to a
shared scoped-fetch method that receives the captured tenant, so reconciliation
and the single terminal observability signal both happen inside the one shared
chokepoint. The handler holds this wrapper as `Arc<dyn DataProviderAccess>` and
can neither read nor overwrite the captured tenant. This mirrors how the actor
binds the established tenant for effect acceptance rather than trusting the
handler to pass it.

**Rejected — a mutable/captured tenant field on the singleton
`RuntimeDataProviderAccess`**: the runtime holds ONE shared access instance
behind `Arc<dyn DataProviderAccess>`. Storing the "current" tenant on that shared
instance (a `with_authoritative_tenant()` setter, or a mutable field) would let
**concurrent dispatches for different tenants cross-contaminate** — dispatch for
`tenant-a` and dispatch for `tenant-b` race the same field, and one could observe
the other's tenant. That is a correctness AND a security defect, so the singleton
carries **no** tenant; the tenant lives only on the short-lived per-dispatch
wrapper. Freezing this closes the review gap where a `with_authoritative_tenant()`
constructor could exist for unit tests yet never be wired into production.

**Also rejected**: adding a `tenant` parameter to the public `fetch` signature
(handler-supplied ⇒ same forgery seam); a thread-local / task-local ambient
tenant (implicit, hard to audit, and the codebase deliberately avoids ambient
tenant state per `data_provider_access.rs:36-37`: "There is no ambient/global
tenant state").

Because a per-dispatch wrapper can be constructed in a unit test without going
through real dispatch, ADR-3 is only fully satisfied by an **end-to-end
integration test** that drives a real actor→handler dispatch under a known
tenant and asserts the provider receives it (Testing Strategy, and TASK covering
real-dispatch wiring). Unit tests on hand-built wrappers are necessary but not
sufficient.

## Data Flow

    Per dispatch (actor knows entity_id.tenant_id / canonical_tenant):
      TenantScopedDataProviderAccess { inner: Arc<RuntimeDataProviderAccess>, authoritative_tenant: Option<TenantId> }   ← captured for THIS invocation only

    handler ──fetch(provider_id, DataRequest{ key, payload, tenant: caller })──▶ TenantScopedDataProviderAccess::fetch
          │  delegates ▶ inner.fetch_scoped(authoritative_tenant, provider_id, request)   [the ONE shared observable chokepoint]
          │
          │  ── started = now; key = request.key ──                       [inside the observable lifecycle]
          ├─ reconcile(authoritative_tenant, request.tenant):
          │     Some(A) + None        ⇒ inject   → request.tenant = Some(A)          → continue
          │     Some(A) + Some(A)     ⇒ pass                                          → continue
          │     Some(A) + Some(B!=A)  ⇒ FAIL CLOSED → log_fetch(outcome=TenantMismatch, attempts=1, cache_hit=false); return Err(TenantMismatch)
          │     None    + None        ⇒ pass (tenant-agnostic)                        → continue
          │     None    + Some(C)     ⇒ FAIL CLOSED → log_fetch(outcome=TenantContextMissing, attempts=1, cache_hit=false); return Err(TenantContextMissing)
          │                                          (fail-closed ⇒ ONE terminal signal, ZERO retries, ZERO provider calls)
          │
          └─ authorized request ──▶ [ #234 retry/timeout loop: provider.fetch(request.clone()) → log_fetch(terminal) ]  (provider sees only the authorized tenant)

### Sequence: tenant reconciliation at fetch

    Handler   TenantScopedDataProviderAccess(A)   RuntimeDataProviderAccess(shared)   Provider
      │─fetch(id, req{tenant:None})─▶│─fetch_scoped(A, ..)─▶│
      │                                                     ├ reconcile(A,None) ⇒ inject A
      │                                                     │──── provider.fetch(req{tenant:A}) ────▶│  (retry/timeout loop, #234)
      │                                                     │◀────────── DataResponse ──────────────┤
      │◀──────────────────── DataResponse ──────────────────┤
      │
      │─fetch(id, req{tenant:B})─────▶│─fetch_scoped(A, ..)─▶│   (B != A: forged / cross-tenant)
      │                                                     ├ reconcile(A,Some(B)) ⇒ MISMATCH
      │                                                     ├ log_fetch(outcome=TenantMismatch, attempts=1)   [ONE terminal signal]
      │◀──── Err(DataProviderError::TenantMismatch) ─────────┤   (fail closed; provider never invoked; zero retries)

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/runtime/src/providers/access.rs` | Modify (FUTURE) | Add `fetch_scoped(authoritative_tenant, provider_id, request)` that reconciles INSIDE the observable lifecycle (inject / pass / fail-closed per ADR-1); on fail-closed emit ONE `log_fetch` terminal signal with the tenant outcome, `attempts=1`, and return the error — never entering the retry loop or calling a provider; forward only the authorized tenant. `RuntimeDataProviderAccess` keeps NO tenant field (ADR-3). |
| `crates/runtime/src/providers/access.rs` (`TenantScopedDataProviderAccess`) | Create (FUTURE) | Immutable per-dispatch wrapper: `{ inner: Arc<RuntimeDataProviderAccess>, authoritative_tenant: Option<TenantId> }` implementing `DataProviderAccess::fetch` by delegating to `inner.fetch_scoped(...)` (ADR-3) |
| `crates/runtime/src/providers/access.rs` (`ProviderOutcome`) | Modify (FUTURE) | Add non-retryable `TenantMismatch` and `TenantContextMissing` outcome classifications; `is_retryable()` returns `false` for both (ADR-1a) |
| `crates/persistent-entity/src/data_provider_access.rs` | Modify (FUTURE) | Reclassify `DataRequest.tenant` doc to "validated assertion"; add `DataProviderError::TenantMismatch` and `DataProviderError::TenantContextMissing` variants (hand-written `Debug`, consistent with the redaction style already in the file); keep `new`/`for_tenant`/`with_tenant` |
| Actor→handler dispatch (`crates/persistent-entity/src/actor.rs` / runtime builder) | Modify (FUTURE) | Per dispatch, construct a `TenantScopedDataProviderAccess` capturing `entity_id.tenant_id` / `canonical_tenant` and hand THAT to the handler as `Arc<dyn DataProviderAccess>` (ADR-3) |
| `crates/runtime/src/providers/access.rs` (tests) | Modify (FUTURE) | `reconcile`/`fetch_scoped` unit + integration: inject, match, both fail-closed cases (one terminal signal, zero retries, zero provider calls), cross-tenant forgery, tenant-agnostic pass-through |
| `crates/persistent-entity/src/data_provider_access.rs` (tests) | Modify (FUTURE) | Constructor / `None`-path compatibility; `TenantMismatch`/`TenantContextMissing` redaction in `Debug`/`Display`; both non-retryable |
| End-to-end dispatch test (`crates/persistent-entity` or integration harness) | Create (FUTURE) | A handler executed under a real dispatch for `tenant-a` issuing `request.tenant=None` ⇒ the provider receives `tenant-a` (ADR-3 real-wiring proof) |

## Interfaces / Contracts

```rust
// crates/persistent-entity/src/data_provider_access.rs — two additive error variants
pub enum DataProviderError {
    // ... existing variants (Transient, Fatal, Timeout, NotFound, ProviderMissing) ...

    /// The caller-supplied `DataRequest.tenant` disagreed with the tenant
    /// established for the current command/entity context. Fail-closed: the
    /// provider is never invoked. Synthesized by the runtime chokepoint, never
    /// by a provider. Non-retryable.
    #[error("data provider request tenant does not match the established context tenant")]
    TenantMismatch,

    /// The caller asserted a `DataRequest.tenant` but no tenant is established
    /// for the current context, so the assertion cannot be validated against any
    /// authority. Distinct from `TenantMismatch` — there is nothing to mismatch
    /// against. Fail-closed; the provider is never invoked. Non-retryable.
    #[error("data provider request asserted a tenant with no established context tenant to validate it")]
    TenantContextMissing,
}

// DataRequest constructors are UNCHANGED (source-compatible). Only the semantics
// of `tenant` change: it is now a caller *assertion*, validated by the runtime
// chokepoint against the established context — not a trusted authority.
```

```rust
// crates/runtime/src/providers/access.rs
// The shared chokepoint holds NO tenant (ADR-3): concurrent dispatches must not
// share tenant state on the one Arc<RuntimeDataProviderAccess> instance.
pub struct RuntimeDataProviderAccess {
    registry: ExternalDataProviderRegistry,
    config: ProviderAccessConfig,
}

/// Immutable per-dispatch wrapper. Constructed once per actor→handler dispatch,
/// capturing the established tenant for THAT invocation; shares the registry/
/// config via `Arc`. The handler holds this as `Arc<dyn DataProviderAccess>` and
/// cannot read or overwrite `authoritative_tenant`.
pub struct TenantScopedDataProviderAccess {
    inner: Arc<RuntimeDataProviderAccess>,
    authoritative_tenant: Option<TenantId>,
}

#[async_trait]
impl DataProviderAccess for TenantScopedDataProviderAccess {
    async fn fetch(&self, provider_id: &str, request: DataRequest)
        -> Result<DataResponse, DataProviderError>
    {
        self.inner
            .fetch_scoped(self.authoritative_tenant.as_ref(), provider_id, request)
            .await
    }
}

impl RuntimeDataProviderAccess {
    /// Reconcile the request tenant against `authoritative` per ADR-1's five-row
    /// matrix. Pure — no I/O. Fail-closed arms return the classified error; the
    /// caller (`fetch_scoped`) emits the single terminal signal.
    fn reconcile_tenant(
        authoritative: Option<&TenantId>,
        request: &mut DataRequest,
    ) -> Result<(), DataProviderError> {
        match (authoritative, &request.tenant) {
            (Some(a), None) => { request.tenant = Some(a.clone()); Ok(()) }      // inject
            (Some(a), Some(b)) if a == b => Ok(()),                             // match
            (Some(_), Some(_)) => Err(DataProviderError::TenantMismatch),       // fail closed
            (None, None) => Ok(()),                                             // tenant-agnostic
            (None, Some(_)) => Err(DataProviderError::TenantContextMissing),    // fail closed
        }
    }

    /// The one observable chokepoint. Reconciles INSIDE the fetch lifecycle so a
    /// fail-closed decision emits exactly one terminal `data_provider_fetch`
    /// signal (attempts=1, non-retryable outcome), zero retries, zero provider
    /// calls, then returns the error. On success it runs the unchanged #234
    /// retry/timeout loop with the authorized request.
    async fn fetch_scoped(
        &self,
        authoritative: Option<&TenantId>,
        provider_id: &str,
        mut request: DataRequest,
    ) -> Result<DataResponse, DataProviderError> {
        let started = Instant::now();
        if let Err(e) = Self::reconcile_tenant(authoritative, &mut request) {
            let outcome = ProviderOutcome::from_tenant_error(&e); // TenantMismatch / TenantContextMissing
            log_fetch(provider_id, &request.key, started.elapsed(), false, outcome, 1);
            return Err(e); // never enters the retry loop, never calls a provider
        }
        // ... unchanged #234 registry lookup + retry/timeout loop with `request` ...
    }
}
```

## Error Model

Two additive, fail-closed, **non-retryable** variants:
`DataProviderError::TenantMismatch` and `DataProviderError::TenantContextMissing`.
Both carry **no** tenant identifiers — a fail-closed tenant decision is
queryable/alertable on its own without rendering any tenant, and omitting them
keeps the variants allocation-free and consistent with `Timeout`'s "carries no
free text" style (`data_provider_access.rs:133-142`). Both are `is_retryable() ==
false` (a tenant decision is never resolved by retrying). They are synthesized by
`fetch_scoped` inside the observable lifecycle; on either, the provider is
**never** invoked and the retry loop is never entered. Their hand-written `Debug`
follows the file's existing redaction convention.

## Observability

Reconciliation runs INSIDE the fetch lifecycle, so a fail-closed tenant decision
emits **exactly one** terminal `data_provider_fetch` signal — NOT zero. Two new
`ProviderOutcome` classifications, `TenantMismatch` and `TenantContextMissing`,
join the existing outcome set; both are non-retryable, so the emitted signal
carries `attempts = 1` and no retry signal is emitted. This is the
security-appropriate choice: cross-tenant attempts are alertable through the
existing #234 pipeline. Per the bounded-cardinality rule, no raw tenant id is
added as a metric label — tenant values never become labels; the decision is
observable via the outcome classification only. No new pipeline, no
provider-authored text, and the "exactly one `data_provider_fetch` per fetch"
invariant is preserved (the fail-closed path emits one and only one).

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | `reconcile_tenant` five-row matrix: inject (`Some(A)`,`None`); pass (`Some(A)`,`Some(A)`); `TenantMismatch` (`Some(A)`,`Some(B)`); tenant-agnostic pass (`None`,`None`); `TenantContextMissing` (`None`,`Some(C)`) | pure runtime unit tests |
| Unit | `DataProviderError::TenantMismatch` AND `TenantContextMissing` `Debug`/`Display` leak no tenant id; BOTH are non-retryable (`is_retryable() == false`) | persistent-entity + runtime unit tests |
| Integration | Positive — captured authority `A`, caller `None` ⇒ provider records exactly `A`; caller `A` ⇒ provider records `A` | `#[tokio::test]` with a tenant-recording provider double (shape of `TenantRecordingProvider`, `data_provider_access.rs:284-331`) |
| Integration | Negative (mismatch) — captured authority `A`, caller `Some(B)` ⇒ `Err(TenantMismatch)`, provider **never** called | `#[tokio::test]` |
| Integration | Negative (context-missing) — captured authority `None`, caller `Some(C)` ⇒ `Err(TenantContextMissing)`, provider **never** called | `#[tokio::test]` |
| Integration | Observable fail-closed — for BOTH a mismatch and a context-missing case: exactly ONE terminal `data_provider_fetch` outcome emitted, ZERO retries, ZERO provider invocations (recording double sees zero fetches) | `#[tokio::test]` asserting the terminal outcome value + attempts=1 + recorder empty |
| Integration (CROSS-TENANT) | Forged cross-tenant read — captured authority `tenant-a`, caller forges `tenant-b` via `for_tenant`/`with_tenant` ⇒ fetch fails closed; provider never receives `tenant-b`; no data for `tenant-b` is returned | `#[tokio::test]`, dedicated cross-tenant negative |
| Integration | Compatibility — captured authority `None`, `DataRequest::new` path ⇒ unchanged pass-through; existing tenant-agnostic tests pass unmodified | `#[tokio::test]` |
| End-to-end (ADR-3) | Real actor→handler dispatch under `tenant-a`; handler issues `fetch` with `request.tenant=None` ⇒ provider receives `tenant-a`. Proves the per-dispatch wrapper is actually wired into production dispatch, not only hand-built in tests | `#[tokio::test]` driving real dispatch |
| Regression | #234 timeout/retry/observability behavior unchanged — the retry/timeout loop is entered only on the authorized (success) path | existing #234 tests pass unmodified |

## Threat Matrix

| Threat | Vector | Mitigation | Test |
|--------|--------|-----------|------|
| **Tenant spoofing** | Handler calls `DataRequest::with_tenant(other)` / `for_tenant(.., other)` (`data_provider_access.rs:54-67`, `with_tenant` "Overwrites any tenant already set") to assert a tenant it is not scoped to | Chokepoint reconciles against the captured authoritative tenant; a disagreeing value fails closed with `TenantMismatch`, an unauthorized assertion with `TenantContextMissing` (ADR-1); the captured tenant is unreachable by the handler (ADR-3) | Negative (both) + cross-tenant integration tests |
| **Cross-tenant read** | Forged `DataRequest.tenant = tenant-b` while established context is `tenant-a`, aiming to make the provider fetch `tenant-b`'s data | Provider is invoked only with the reconciled authoritative tenant; mismatch fails closed *before* any provider call, so `tenant-b` never reaches the provider and no `tenant-b` data is returned | Cross-tenant integration test (provider double asserts it never sees `tenant-b`) |
| **Concurrent dispatch cross-contamination** | Two simultaneous dispatches for `tenant-a` and `tenant-b` share the one `Arc<RuntimeDataProviderAccess>`; a tenant stored on that singleton could leak across | The shared chokepoint holds NO tenant; each dispatch carries its tenant on its own immutable `TenantScopedDataProviderAccess` (ADR-3) — no shared mutable tenant state to race | Covered structurally by ADR-3 (no tenant field on the singleton) |
| **Silent widening** | Caller supplies a broader / different tenant hoping the runtime silently overrides and proceeds, masking the attempt | A disagreeing/unauthorized value is an explicit error, not a silent override (frozen decision 2), and emits an alertable terminal outcome; detection is preserved alongside confidentiality | Negative + observable-fail-closed tests assert `Err` + one emitted outcome, not a silent success |
| **Injection bypass** | Caller relies on ambient/global tenant state to sidestep the request field | No ambient tenant state exists (`data_provider_access.rs:36-37`); the only authority is the per-dispatch captured tenant | Covered structurally by ADR-3 (no ambient source) |

Out of matrix scope: explicitly *granted* cross-tenant access
(`allow_cross_tenant`, issue #73) — that is authorized widening, a separate
concern from forgery, and is not addressed here.

## Migration / Rollout / Compatibility

Source-compatible for tenant-agnostic callers (`DataRequest::new`, tenant `None`)
and matching-tenant callers (ADR-2). The additive breaks are
`DataProviderError::TenantMismatch` and `DataProviderError::TenantContextMissing`,
which downstream exhaustive matches must handle — mechanical and
compile-time-caught. Behavior changes for callers that previously forged a
different tenant (now `TenantMismatch`) or asserted a tenant with no established
authority (now `TenantContextMissing`) — both the intended fix. The
`DataRequest::new` → `None` tenant-agnostic path is unchanged. Rollback = drop the
`TenantScopedDataProviderAccess` wrapper + `fetch_scoped`/`reconcile_tenant` + the
two outcome classifications and error variants, restoring the pass-through
`fetch`; tenant-agnostic behavior is identical either way. No schema or storage
migration.

## Open Questions

None blocking. ADR-3 is frozen (immutable per-dispatch wrapper; the singleton
holds no tenant), so there is no remaining apply-time fork on where the tenant
lives.
