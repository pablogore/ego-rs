# Proposal: CORE-019B — External Data Provider Tenant Authority

## Intent

CORE-019A shipped a tenant field on the provider fetch request —
`DataRequest.tenant: Option<TenantId>`
(`crates/persistent-entity/src/data_provider_access.rs:38`) — but its value is
entirely **caller-populated** through `DataRequest::new` / `for_tenant` /
`with_tenant` (`:45-67`), and the runtime chokepoint
`RuntimeDataProviderAccess::fetch` passes the request through **untouched**
(`crates/runtime/src/providers/access.rs:289-361`, `provider.fetch(request.clone())`
at `:329`). Nothing reads, sets, or validates `request.tenant` against the
tenant already established for the current command/entity. The doc on the field
says "the caller must pass the same authoritative identity the rest of the
command context already carries" (`data_provider_access.rs:33-37`) — but that is
**convention, not enforcement**. A handler can call
`DataRequest::for_tenant(key, payload, other_tenant)` (or
`.with_tenant(other_tenant)`, which "Overwrites any tenant already set",
`:64-67`) and the runtime will faithfully forward the forged tenant to the
provider.

The canonical spec already forbids the **provider** from substituting or minting
a tenant ("Tenant Isolation For Tenant-Scoped Fetches",
`openspec/specs/external-data-providers/spec.md:196-208`) — but that requirement
is satisfied *trivially*, because the provider only ever reads the tenant it is
handed. The real, un-specified gap is on the **runtime side**: the runtime never
binds the fetch tenant to the established context, so a tenant-scoped fetch's
authority rests on the caller behaving. This change closes that gap: the
authoritative tenant for a tenant-scoped fetch MUST derive from the established
command/entity context, and the runtime MUST enforce it — injecting it when the
caller supplied none and failing closed when a caller-supplied tenant disagrees.

The framework already applies exactly this authoritative-vs-hint discipline
elsewhere: the actor's effect-acceptance path derives the tenant from the
established `entity_id.tenant_id`, validates it, and fails closed on an invalid
identity (`crates/persistent-entity/src/actor.rs:326-339`), and `ServiceContext`
distinguishes an authoritative `canonical_tenant()` from a non-authoritative
`tenant_hint()` (`crates/service-sdk/src/context/mod.rs:52-65,396`). The provider
fetch path is the one authority-carrying seam that still trusts caller input.

## Scope

### In Scope

- The **authoritative source** of the tenant for a tenant-scoped provider fetch:
  the established command/entity context, never unvalidated caller input.
- The **relationship** between the fetch tenant and the command/request context
  that already carries the established tenant identity.
- Prevention of **tenant spoofing** via a forged `DataRequest.tenant`.
- Both **injection** (context tenant supplied when the caller left it absent) and
  **validation** (a caller-supplied tenant checked against the context).
- **Fail-closed** behavior when a caller-supplied tenant does not match the
  established context.
- **Compatibility / migration** of the existing `DataRequest::new` /
  `for_tenant` / `with_tenant` constructors and their callers.
- Positive, negative, and **cross-tenant** tests.

### Out of Scope (Non-Goals / Follow-ups)

- Per-attempt **timeout**, bounded **retry/backoff**, and **fetch observability**
  signals — already shipped by issue #234 and specified by the existing
  "Timeout/Retry Observability" and "Fetch Observability Signals" requirements.
  Referenced here only as PRESERVED context; not re-specified and not re-worked.
- Cross-tenant **grant/escalation** semantics (`allow_cross_tenant`,
  `CrossTenantGrant`, issue #73) — an explicitly-authorized cross-tenant read is
  a separate concern; this change closes the *forgery* path, not the *granted*
  path.
- Provider-internal tenant handling beyond receiving the authorized value; the
  provider side is already constrained by the existing spec.
- Any transport, HTTP, or ingress tenant-resolution mechanism.

## Frozen Decisions (decided constraints, not open questions)

1. **Authoritative source is the context, never caller input.** The tenant a
   tenant-scoped fetch is authorized against MUST be the tenant already
   established for the current command/entity (the actor's `entity_id.tenant_id`
   / the resolved `canonical_tenant`), not a value the handler is free to choose
   at the fetch call site.
2. **Fail closed on mismatch.** A caller-supplied tenant that disagrees with the
   established context MUST cause the fetch to fail with an explicit, observable
   error — never a silent override that could mask a spoofing attempt, and never
   a widened read.
3. **The provider receives only the authorized tenant.** After enforcement, the
   provider MUST receive exactly the context-authoritative tenant (or the
   tenant-agnostic `None`), with no path to widen or substitute it — preserving
   the existing "Tenant Isolation For Tenant-Scoped Fetches" guarantee, now
   backed by runtime enforcement rather than caller convention. A caller MUST
   NOT be able to choose the authorizing tenant at the fetch call site.
4. **The tenant-agnostic path stays.** `DataRequest::new` (tenant `None`) remains
   the single-tenant / not-tenant-scoped case and MUST keep working unchanged;
   enforcement is a no-op when there is no established tenant to enforce.
   Preserving tenant-agnostic mode does NOT require accepting an arbitrary
   caller-supplied tenant: a caller that asserts a tenant when no authority is
   established (`context=None`, `request=Some(C)`) MUST fail closed, because
   there is nothing to validate that assertion against.
5. **Two distinct fail-closed cases.** A caller-supplied tenant that disagrees
   with an established authority is a `TenantMismatch`; a caller-supplied tenant
   asserted with no established authority is a `TenantContextMissing`. Both fail
   closed; they are distinct because one has an authority to disagree with and
   the other has none.
6. **Fail-closed tenant decisions are observable.** A fail-closed tenant decision
   MUST emit exactly one terminal `data_provider_fetch` signal classified as a
   distinct non-retryable outcome, with zero retries and zero provider calls, so
   cross-tenant attempts are alertable through the existing (#234) pipeline.

## Design decision resolved by ADR-1

The core API decision is **resolved**, not open: the runtime does **both** —
injects the established context tenant when the caller supplied none, and
fail-closed-validates a caller-supplied tenant against the context. Design ADR-1
freezes the full five-row reconciliation matrix (including the two distinct
fail-closed cases above) with the Verdict, tradeoffs, and fail-closed security
rationale. The mechanism by which the established tenant reaches the fetch
chokepoint is frozen by ADR-3 (an immutable per-dispatch wrapper).

## Capabilities

### New Capabilities

None. This change adds runtime-side enforcement behavior to an existing
capability.

### Modified Capabilities

- `external-data-providers`: adds a runtime-side authoritative-tenant obligation
  to the provider fetch path — the established context is the tenant authority;
  the runtime injects it when absent and fail-closed-validates a caller-supplied
  tenant against it; a forged `DataRequest.tenant` can neither widen nor
  cross-tenant-read. The existing provider-side "Tenant Isolation" requirement is
  restated to name the runtime as the enforcing authority.

## Approach

Make the runtime provider-access chokepoint the single authority for the fetch
tenant. An immutable per-dispatch wrapper captures the established context tenant
(from the actor's `entity_id.tenant_id` / the resolved `canonical_tenant`) for
one invocation and shares the registry/config via `Arc`, so the chokepoint has an
authoritative value the handler cannot forge and concurrent dispatches never
cross-contaminate. Inside the observable fetch lifecycle the chokepoint
reconciles `request.tenant` against that authoritative tenant per the frozen
five-row matrix (ADR-1): absent caller value ⇒ inject the context tenant;
matching caller value ⇒ pass; disagreeing caller value ⇒ fail closed
`TenantMismatch`; caller value asserted with no established authority ⇒ fail
closed `TenantContextMissing`; both `None` ⇒ tenant-agnostic pass-through. Each
fail-closed decision emits one terminal `data_provider_fetch` signal (a distinct
non-retryable outcome) with zero retries and zero provider calls. The provider is
invoked only with the reconciled, context-authoritative tenant. Existing
`DataRequest` constructors are kept for source compatibility; their `tenant`
value is reinterpreted from a *trusted authority* to a *caller assertion that is
validated*, and the tenant-agnostic `None` path is preserved. The #234
timeout/retry/observability loop itself is untouched.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/runtime/src/providers/access.rs` | Modified (FUTURE) | Reconcile the request tenant against the authoritative tenant INSIDE the observable fetch lifecycle; inject when absent, fail closed (`TenantMismatch` / `TenantContextMissing`) with one terminal signal, zero retries, zero provider calls; forward only the authorized tenant; new per-dispatch wrapper type |
| `crates/persistent-entity/src/data_provider_access.rs` | Modified (FUTURE) | `DataRequest.tenant` semantics reclassified to a validated assertion; two fail-closed error variants added (`TenantMismatch`, `TenantContextMissing`); constructors preserved |
| Actor→handler dispatch (`crates/persistent-entity/src/actor.rs` / runtime builder) | Modified (FUTURE) | Build a per-dispatch tenant-scoped wrapper that shares the registry/config `Arc` and captures the established `entity_id.tenant_id` for that one invocation; hand it to the handler |
| Fetch observability (`ProviderOutcome`) | Modified (FUTURE) | Two new non-retryable outcome classifications for tenant-mismatch and tenant-context-missing |
| Tests (runtime + persistent-entity) | New (FUTURE) | Positive (inject / match), negative (both fail-closed cases), non-retryable proof, single-terminal-signal proof, cross-tenant forgery, and an end-to-end real-dispatch binding test |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Silent override masks a spoof attempt | Med | Fail closed on mismatch (frozen decision 2); never override silently |
| Breaking the tenant-agnostic `None` path / existing callers | Med | Enforcement is a no-op with no established tenant AND no caller-asserted tenant; `DataRequest::new`/constructors preserved (ADR-2) |
| New error variants break exhaustive matches on `DataProviderError` | Low | Two additive variants; document as a minor compatibility note; compile-time-caught |
| A singleton access facade cross-contaminates concurrent dispatches | High | ADR-3 freezes an immutable per-dispatch wrapper (never a mutable/captured singleton field); the tenant is captured per invocation |
| Fail-closed tenant decisions escape the observability pipeline | Med | Reconciliation runs INSIDE the observable fetch lifecycle; each fail-closed decision emits one terminal `data_provider_fetch` signal (frozen decision 6) |
| Re-touching #234 timeout/retry/observability by accident | Low | Reconciliation reuses the existing terminal-signal path with zero retries and zero provider calls; the retry/timeout loop itself is unchanged |

## Rollback Plan

The enforcement is a check inserted at the fetch chokepoint plus a bound
authoritative tenant and one additive error variant. Rollback = remove the
reconciliation check and the bound tenant, restoring the pass-through fetch;
`DataRequest` constructors and the `None` path are unchanged either way, so
revert is behavior-neutral for tenant-agnostic callers. No schema or migration
impact.

## Dependencies

- Builds on CORE-019A (`DataRequest`, `DataProviderAccess`,
  `RuntimeDataProviderAccess`) — archived at
  `openspec/changes/archive/2026-07-16-core-019a-external-data-providers`.
- PRESERVES issue #234 (provider hardening: per-attempt timeout, bounded retry,
  fetch/retry observability). Referenced, not modified.
- Related but out of scope: issue #73 (`allow_cross_tenant` self-escalation);
  the established-tenant plumbing in `crates/service-sdk/src/runtime/tenant.rs`
  and `crates/persistent-entity/src/actor.rs`.
- No dedicated open issue tracks this runtime-side enforcement gap.

## Success Criteria

- [ ] The authoritative tenant for a tenant-scoped fetch derives from the
  established command/entity context, not from unvalidated caller input.
- [ ] A caller-supplied tenant that matches the context passes; an absent one is
  injected from the context.
- [ ] A caller-supplied tenant that disagrees with the context fails closed with
  `TenantMismatch`; a caller-supplied tenant asserted with no established
  authority fails closed with `TenantContextMissing`. Neither reaches the
  provider.
- [ ] Each fail-closed tenant decision emits exactly one terminal
  `data_provider_fetch` signal, classified non-retryable, with zero retries and
  zero provider invocations.
- [ ] A forged `DataRequest.tenant` naming a different tenant can neither widen
  nor read another tenant's data.
- [ ] The provider receives only the context-authoritative tenant (or `None`).
- [ ] The tenant-agnostic `None` path (`DataRequest::new`) and existing
  `DataRequest` constructors keep working; existing tenant-agnostic callers
  compile unchanged.
- [ ] The established tenant reaches the chokepoint via an immutable per-dispatch
  wrapper; an end-to-end real-dispatch test proves a handler under `tenant-a`
  with `request.tenant=None` causes the provider to receive `tenant-a`.
- [ ] Positive, negative, and cross-tenant tests pass; `cargo test --workspace`
  green; #234 timeout/retry/observability behavior unchanged.
