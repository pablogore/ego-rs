# Design: CORE-008A — Canonical Tenant Model & Runtime Enforcement

## Technical Approach

Introduce **one concrete, runtime-owned resolution seam — `TenantResolver`** — that
turns transport-neutral inputs (an already-produced `SecurityContext`, an optional
caller-supplied tenant hint, and the runtime's tenant-enforcement mode) into exactly one
immutable canonical tenant value (`CanonicalTenant`), and make `enforce_tenant`
fallible so the macro-generated `#[operation]` path aborts before the operation
body on any tenant failure.

The single most consequential decision — **whether the boundary is a new
port/trait or an extension of `AuthenticationProvider`** — is resolved first
(AD-001) because it conditions the canonical model's shape (AD-002…AD-006), the
classification mechanism (AD-007), the `CrossTenantPermit` contract (AD-008), and
the enforcement mechanism (AD-009). It also shapes the runtime-configuration
surface (AD-012), which registers concrete configuration precisely because AD-001
chose a concrete resolver rather than a `dyn` port. A few decisions are largely
**independent** of AD-001's outcome and would hold under any resolution-boundary
choice — the `ServiceContext` hint demotion (AD-011) and the error taxonomy
(AD-010) follow from the single-canonical-value goal (FR-008), not from AD-001
specifically. None of the decisions below were retrofitted.

Reuse over invention: the domain already owns a validated `TenantId` newtype
(`crates/domain/src/context.rs:48`), the authenticated tenant already lives on
`Principal.tenant_id` (`crates/security-sdk/src/principal/principal.rs:63`), and
`AuthorizationProvider` (CORE-014) already expresses capability checks. This change
adds a seam and an enforcement path over those, not a parallel tenant subsystem.

---

## AD-001 — The resolution boundary is a concrete `TenantResolver`, not an `AuthenticationProvider` extension and not a `dyn` plugin (Open Question 9)

**Decision.** Tenant resolution becomes a **new, dedicated, CONCRETE type
`TenantResolver`** owned by the runtime layer (`crates/service-sdk/src/runtime/`).
It is NOT an extension of `AuthenticationProvider`, and it is NOT a `dyn`-dispatched
pluggable port with multiple implementations. It exposes one fixed operation:

```rust
// crates/service-sdk/src/runtime/tenant.rs  (new)
pub struct TenantResolver { mode: TenantEnforcementMode }

impl TenantResolver {
    /// The single resolution algorithm mandated by D2. Transport-neutral inputs only.
    pub(crate) fn resolve(
        &self,
        security: Option<&SecurityContext>, // already produced upstream (authn)
        supplied_tenant: Option<&str>,      // ingress hint (builder today, header later)
    ) -> Result<CanonicalTenant, SecurityError>;
}
```

### Options considered

| Option | Shape | Tradeoff | Verdict |
|--------|-------|----------|---------|
| **(a) New concrete `TenantResolver`** | One runtime type, one fixed algorithm | Names the D4 convergence seam; keeps the fixed D2 policy in one place; no speculative indirection; covers authenticated AND system/internal paths uniformly | **CHOSEN** |
| (b) Extend `AuthenticationProvider::authenticate` to also resolve tenant | Add a tenant method / richer return to the existing trait (`authentication/mod.rs:14`) | `authenticate` ALREADY returns `SecurityContext` carrying `Principal.tenant_id` — the authenticated tenant is already resolved, so the extension is redundant there; and it CANNOT cover FR-003's unauthenticated system/internal path (no credential → provider never runs). Would also force every existing `AuthenticationProvider` impl (JWT, API-key, OIDC) to change. | Rejected |
| (c) `dyn TenantResolver` pluggable port | Trait + `Arc<dyn>`, registered like authn/authz | The resolution POLICY is fixed by D2 (authenticated → Principal canonical; mismatch → error; internal → only if permitted; neither → `MissingContext`). It is an invariant, not a per-deployment knob. A `dyn` port with exactly one implementation is a speculative abstraction — the same anti-pattern CORE-010A rejected ("Only one execution context implementation exists"). | Rejected (see Future evolution) |
| (d) No seam — inline resolution in the macro / in `enforce_tenant` | Fewest types | Scatters the D2 algorithm across generated code and `RuntimeInner`; gives transport adapters (D4) nothing concrete to converge on; untestable in isolation. | Rejected |

### Why concrete-but-named beats both a trait and an extension

The audit shows two *different* boundaries were being conflated:

1. **Transport → transport-neutral inputs** (extract credential + raw tenant hint
   from HTTP headers / gRPC metadata). This is per-transport, pluggable, and
   **out of scope** (Non-Goals; no `x-tenant` extraction exists anywhere today).
2. **Transport-neutral inputs → canonical tenant** (apply the fixed D2 policy).
   This is the seam CORE-008A owns.

D4's requirement that "all extractors converge on the same `TenantResolver`
contract" is satisfied by boundary (1) feeding boundary (2): future adapters
produce a `SecurityContext` (via `AuthenticationProvider`) plus an optional tenant
hint and call the SAME concrete `TenantResolver::resolve`. The resolver does not
know HTTP or gRPC (FR-007). Extractors are the pluggable per-transport part; the
resolver is the single convergence point. A concrete type is therefore both the
correct model AND the lazy one — no `dyn` where there is no variability.

### How AD-001 shapes everything downstream

- **Inputs are transport-neutral** → the runtime carries no transport dependency (FR-007).
- **`security: Option<&SecurityContext>`** → authenticated path reads `Principal.tenant_id` as canonical; `None` routes to the system/internal branch (FR-002/FR-003/FR-004).
- **`supplied_tenant: Option<&str>`** → the mismatch check (FR-002) and the "permitted internal tenant" check (FR-003) both operate on this single hint.
- **Single return `CanonicalTenant`** → defines the canonical model's home, creator, and mutability (AD-002…AD-005).
- **`pub(crate)` resolve** → only the runtime can mint a canonical tenant (AD-003), mirroring the `CrossTenantPermit` capability-token discipline.

**Future evolution.** If genuine per-deployment resolution variability ever
emerges (e.g. custom Principal→tenant mapping), `TenantResolver` may be promoted to
a trait in a later change without touching callers — same escape hatch CORE-010A
reserved for `ExecutionContext`. Until then it stays concrete.

Satisfies: **FR-007, FR-011** (Open Question 9).

---

## Architecture Decisions (canonical model, lifecycle, enforcement)

### AD-002 — Canonical tenant type: a runtime enum reusing domain `TenantId` (Open Question 1)

**Decision.** The canonical in-runtime representation is a new enum in
`crates/service-sdk/src/runtime/tenant.rs`, wrapping the existing domain newtype:

```rust
pub struct CanonicalTenant(Repr);   // opaque wrapper — see note below

enum Repr {
    /// A concrete resolved tenant (authenticated or permitted-internal path).
    Scoped(ego_domain::context::TenantId),
    /// Valid tenant-less system / single-tenant execution (D1).
    Systemwide,
}

impl CanonicalTenant {
    pub fn tenant_id(&self) -> Option<&ego_domain::context::TenantId>;  // None for Systemwide
    pub fn is_systemwide(&self) -> bool;
}
```

**Opaque wrapper, not a raw public enum (implementation-verified at apply time).**
A plain `pub enum CanonicalTenant { Scoped(TenantId), Systemwide }` cannot satisfy
AD-003: Rust enum variants always share the visibility of their enum, so a public
`Scoped(TenantId)` tuple variant would be freely constructible by any external
crate holding a `TenantId` (itself public) — defeating "only `TenantResolver` may
create a `CanonicalTenant`" the moment it compiled. `#[non_exhaustive]` doesn't fix
this either: applied per-variant it also blocks external *matching*, which would
break `ServiceContext::canonical_tenant()`'s intended read path (AD-011). The
wrapper above — a private `Repr` enum behind a public struct, `pub(super)`
constructors, public read-only accessors — is the smallest fix consistent with
this AD's actual intent, and mirrors `CrossTenantPermit`'s existing
`pub(super)`-constructor pattern in `permit.rs`.

- **Reuses** `ego_domain::context::TenantId` (already validated non-empty) — no new identity type. Rung 2 of the ladder: it already lives here.
- **Lives in service-sdk**, not domain: the `Systemwide` arm is a *runtime execution* concept (D1), and enforcement is a runtime concern. Domain stays runtime-neutral (its `ExecutionContext`/`TenantId` are unchanged; they become *derived-from*, never independently authoritative — AD-006).

**When `Systemwide` actually occurs (illustrative, not exhaustive).** This variant
is not a theoretical placeholder — it is the resolved value for non-tenant-scoped
operations running in D1's valid tenant-less mode, for example: a database
migration runner, startup/bootstrap wiring, a liveness/health-check endpoint, or an
internal scheduler tick that has no per-tenant meaning. None of these carry
`#[tenant_scoped]`, so `resolve()` is never even consulted for scoping purposes on
these paths (Implementation Note 2 in tasks.md); `Systemwide` remains available as
a real, constructible value for any code that explicitly needs to state "this
execution is intentionally tenant-less," rather than only being an implicit default.

Rejected: a bare `TenantId` (cannot express D1's tenant-less mode); a brand-new
`String`-based tenant type (duplicates `TenantId`, drops its non-empty invariant).

Satisfies: **FR-008**.

### AD-003 — Only `TenantResolver` may create a `CanonicalTenant` (Open Question 2)

**Decision.** `CanonicalTenant`'s constructors are `pub(crate)` within the
`service-sdk` runtime module; the only public path to obtain one is
`TenantResolver::resolve`. Application code, service handlers, and other crates
CANNOT fabricate a canonical tenant. This mirrors the existing `CrossTenantPermit`
capability-token pattern (`permit.rs`: private constructor, trusted-module-only) —
a value whose mere existence is proof it passed the D2 policy.

Rejected: public constructor / public fields (would let any holder assert an
unvalidated tenant into the *canonical* value — the same class of hole
`ServiceContext.tenant_id` has as a hint field. Wording correction,
code-review fix: AD-011 deliberately keeps `tenant_id` itself `pub` as a
non-authoritative hint — see AD-011's transition rule — so this AD closes
the hole for the *canonical* value, not for the raw hint field's visibility).

Satisfies: **FR-008**.

### AD-004 — `CanonicalTenant` is immutable from creation; there is no mutation point (Open Questions 3 & 4)

**Decision.** `CanonicalTenant` has **no setters, no public fields, and no `&mut`
API**. It is immutable the instant `TenantResolver::resolve` returns it. The answer
to "at what point does it stop being mutable?" is therefore: **it is never mutable —
the resolution point is the freeze point.** It is `Clone` (cheap: an `Arc`-free
small value) so it can be read wherever needed, but a clone cannot diverge because
there is no way to change either copy.

This is precisely how **FR-014** (tenant immutable during operation execution) is
achieved technically: the authoritative value has no mutation path, so no
downstream code — including a later `ServiceContext::with_tenant_id` call on a
cloned context — can alter the tenant an in-flight operation enforces against.

Satisfies: **FR-008, FR-010, FR-014**.

### AD-005 — Lifecycle owner: the runtime, operation-scoped (Open Question 5)

**Decision.** A `CanonicalTenant` is **owned by the runtime for the duration of a
single operation invocation**. It is created at the enforcement seam (before the
operation body), used by enforcement and — via the accessor in AD-011 — read by the
operation body, and dropped when the operation returns. It is **request/operation-
scoped**, exactly like `SecurityContext`/`Claims` (AD-002 in security-sdk: claims
are request-scoped, MUST NOT be persisted). It is never stored in aggregates,
events, snapshots, projections, or repositories (that would be persistence
multitenancy — D6, out of scope).

**Boundary containment (reinforces AD-002).** `CanonicalTenant` is a
runtime-internal type that never crosses the runtime boundary: it is not part of
the domain layer, not part of any event/message payload, not part of persistence or
wire/serialized contracts, and not part of any public cross-crate API beyond the
read-only `ServiceContext::canonical_tenant()` accessor (AD-011). It is neither
`Serialize`/`Deserialize` nor exposed to transport (FR-007/AD-013). This is
already implied by AD-002 (lives in service-sdk; domain stays runtime-neutral) and
by the operation-scoped, never-persisted lifecycle above; it is stated here
explicitly so the containment invariant is unmissable rather than given its own
redundant AD.

Satisfies: **FR-008**.

### AD-006 — Transient coexistence: `Principal.tenant_id` is the authoritative INPUT, the resolver output is the authoritative RUNTIME value (Open Question 6)

**Decision.** During incremental rollout the four legacy representations
(`Principal.tenant_id`, `ServiceContext.tenant_id`, domain
`ExecutionContext`/`TenantId`, `ClaimSet::tenant()`) coexist, but authority is
resolved by a strict, testable precedence:

| Representation | Role after this change |
|---|---|
| `Principal.tenant_id` | **Authoritative INPUT** on the authenticated path. The resolver reads it as canonical. |
| `TenantResolver` → `CanonicalTenant` | **Authoritative RUNTIME value.** The only thing enforcement and cross-tenant checks read. |
| `ServiceContext.tenant_id` | Demoted to an **ingress hint** (resolver input for the mismatch check). Never authoritative once resolution runs (AD-011). |
| domain `ExecutionContext`/`TenantId`, `ClaimSet::tenant()` | Unchanged; **derived-from / validated-against** the canonical value, never independently authoritative for the same operation. |

**Rule for "two populated and disagree":** if a caller-supplied hint disagrees with
`Principal.tenant_id`, resolution fails with `TenantMismatch` (FR-002) — the runtime
never silently picks one. This makes the transition window safe: no operation can
execute against an ambiguous tenant; ambiguity is always an error, never a guess.
The tasks phase sequences convergence (domain → runtime → service-sdk) so the
window is minimized, but even mid-migration the precedence above holds.

**`RuntimeExecutionContext` (`crates/runtime/src/context.rs`) — OUT of scope for
CORE-008A.** Proposal Finding 6 names it as a fifth, duplicated tenant carrier (it
mirrors the domain `ExecutionContext`/`TenantId`). Converging or removing it is
**explicitly out of scope for this change**, for three reasons: (1) it is not on
the enforcement read path — enforcement reads the `CanonicalTenant` produced by
`TenantResolver` (AD-001/AD-009), never `RuntimeExecutionContext`, so leaving it
untouched cannot create a parallel authority for an enforced operation; (2) it sits
in the `runtime` crate as an execution-context duplicate, and reshaping it is a
`runtime`/domain-context convergence concern with its own blast radius, orthogonal
to the resolution/enforcement seam CORE-008A owns; (3) pulling it in would widen
the transient window this AD is trying to keep narrow, contradicting the migration
sequencing. It therefore inherits the same status as domain
`ExecutionContext`/`TenantId` in the table above — **derived-from /
validated-against the canonical value, never independently authoritative for an
enforced operation** — and its structural convergence is deferred to a later change
(same escape hatch as AD-001's future evolution). This is a deliberate scope
boundary, not an oversight.

Satisfies: **FR-008**.

### AD-007 — Tenant-scoped classification: opt-in `#[tenant_scoped]` at the macro (Open Question 7, D1)

**Decision.** An `#[operation]` is classified tenant-scoped by an **explicit opt-in
attribute** the proc-macro reads at expansion time, alongside the existing
`#[operation]` / `#[authorize]` handling (`SdkAttr::detect`,
`service-sdk-macros/src/lib.rs:16`):

```rust
#[operation]
#[tenant_scoped]                 // <-- marks this op fail-closed
async fn transfer(&self, ctx: ServiceContext, req: TransferReq) -> Result<..> { .. }
```

For a marked operation the macro emits the **fallible** enforcement call (AD-009,
`?`); for an unmarked operation it preserves today's behavior (no tenant
enforcement — the valid tenant-less system/single-tenant mode of D1).

| Option | Tradeoff | Verdict |
|--------|----------|---------|
| **Opt-in `#[tenant_scoped]` attribute** | Per-operation (D1 wants operation-level, not global); reuses the existing attribute-detection machinery; unmarked ops keep working, so existing examples/tests stay green (mitigates the High-likelihood "no-op → fail-closed breaks everything" risk) | **CHOSEN** |
| Infer from "tenant present in request" | Not fail-closed: absence would silently skip enforcement — the exact hole FR-001/FR-009 close | Rejected |
| Global `RuntimeBuilder` execution mode only | Too coarse; D1 is explicitly operation-level | Rejected (mode still exists for the internal-path knob — AD-012 — but does not classify individual ops) |
| Opt-out (all ops tenant-scoped unless marked `#[system]`) | Secure-by-default, but breaks EVERY existing operation at once — collides head-on with the proposal's High breakage risk and D1's first-class tenant-less mode | Rejected for this change |

**Honest security tradeoff (called out, not hidden):** opt-in means a forgotten
marker leaves an operation unenforced — the classic fail-open hole of an opt-in
security model.

**Required automated mitigation (in scope for THIS change).** Because a silently
unenforced operation is a real security regression, the opt-in model MUST ship with
an **automated technical detection** as part of CORE-008A's own scope — not a
manual review checklist and not an indefinitely deferred follow-up. The concrete
form of that detection (a compiler lint / proc-macro diagnostic, a CI validation
step, a workspace-wide verification pass, or an equivalent automated mechanism that
flags operations which look tenant-scoped but carry no `#[tenant_scoped]` marker) is
left to tasks.md/apply to design and build; this AD fixes only the non-negotiable
requirement that SOME automated detection exists and runs in CI before this change
is considered complete. A human checklist is explicitly rejected as the mitigation.

**Detector limitations (explicit, not hidden).** The automated detection required
above is intentionally **best-effort, not exhaustive**. It works by recognizing
tenant-related identifiers referenced directly in an operation's body; an operation
that touches tenant-scoped data through an indirect path (for example, a repository
or projection call that filters by tenant internally without the operation itself
naming a tenant identifier) can produce a false negative the detector cannot see.
This is an **accepted tradeoff during the migration window**, not an oversight —
closing every indirect path would require whole-program data-flow analysis, which is
out of scope for this change. The long-term architectural direction that actually
closes this residual gap is the secure-by-default flip already recorded below, not a
progressively stronger heuristic.

**The detector is not part of the security model (explicit, to prevent future
confusion).** Enforcement is `TenantResolver` + `#[tenant_scoped]` + fail-closed
(AD-001/AD-007/AD-009) — that triad is what actually rejects a request. The
automated detector is a **migration aid**, not a fourth enforcement mechanism: it
only catches operations that were plausibly meant to carry `#[tenant_scoped]` but
don't yet, so the opt-in migration doesn't quietly stall. A detector false negative
does not weaken enforcement for any operation that already carries the marker; a
detector false positive does not grant or deny access either. Nothing about the
runtime's fail-closed guarantee depends on the detector running, passing, or even
existing — it exists purely to reduce how many markers get forgotten, and must
never be read as a substitute for `#[tenant_scoped]` itself. **Passing the detector
MUST NOT be interpreted as proving an operation is correctly classified** — it only
means no *recognized* tenant-identifier pattern was found unmarked; it is not a
security audit and must never be cited as one.

**Transitional nature of this decision (explicit).** The opt-in classification is a
**migration-era strategy, not necessarily the framework's permanent end-state.** It
was chosen to keep the first slice migration-safe under D1 (unmarked ops keep
working). The **default classification behavior may be revisited once migration is
complete** — specifically, the upgrade path is to flip to secure-by-default
(`#[system]`/opt-out, so operations are tenant-scoped unless explicitly excused)
after the ecosystem has adopted markers. That flip is a scoped follow-up change (it
is a default-behavior change, distinct from the automated detection required above,
which lands now). Recording it here keeps the transitional intent from being read as
a permanent design commitment.

Satisfies: **FR-001**.

### AD-008 — `CrossTenantPermit` becomes destination-scoped and authorization-gated; `Copy` is dropped (Open Question 8, D3)

**Decision.** Two coupled changes to `crates/service-sdk/src/runtime/permit.rs`
and its issuer:

1. **Authorized issuance (required by FR-005).** `issue_cross_tenant_permit`
   becomes **fallible and async** and runs an `AuthorizationProvider` capability
   check before minting:

   ```rust
   pub(crate) async fn issue_cross_tenant_permit(
       &self,
       ctx: &ServiceContext,
       destination: TenantId,
   ) -> Result<CrossTenantPermit, SecurityError>;
   ```

   It resolves the `Principal` from `ctx.security`, builds an `AccessRequest` for the
   explicit cross-tenant capability (e.g. `Resource { kind: "tenant", id: dest }`,
   `Action("cross-tenant-access")`), and calls the existing `authorize_in_context`
   seam. A `Deny` → `SecurityError::CrossTenantDenied` (AD-010). Resource/action
   authorization alone never yields a permit (FR-005).

2. **Destination scoping (answer to Q8).** The permit carries the tenant it was
   authorized for and drops `Copy`:

   ```rust
   #[derive(Debug, Clone)]           // Copy removed — was flagged pending in permit.rs:21-29
   pub struct CrossTenantPermit { destination: TenantId, issued_to: SubjectId }
   ```

   A permit authorizing access to `tenant-b` cannot be reused to reach `tenant-c`
   (closes the privilege-escalation reuse hole — security skill Rule 4). **Not**
   added: time-window or purpose scoping (YAGNI — no requirement, no expiry/audit
   infrastructure in scope; noted as future hardening).

Dropping `Copy` is safe: `with_cross_tenant_access(&CrossTenantPermit)` borrows the
permit, and the existing `clone_preserves_cross_tenant_flag` test clones the
`ServiceContext`, not the permit.

**Conceptual boundary (explicit): a permit authorizes, it does not re-identify.**
Holding a `CrossTenantPermit` for `destination` grants the caller's already-resolved
`CanonicalTenant` permission to reach into `destination`'s data — it does **not**
change, replace, or override the caller's own authenticated tenant. `ctx.canonical_tenant()`
(AD-011) still reports the Principal-derived tenant throughout; the permit is
additional, separately-checked authority layered on top of it, never a substitute
for it. This keeps "who the caller is" (AD-002/AD-011) and "what the caller may
additionally reach" (this AD) as two distinct, non-overlapping concerns.

**Breaking-signature migration note.** Changing `issue_cross_tenant_permit` from
today's synchronous, zero-argument, infallible signature to an **async, fallible,
destination-scoped** one is a breaking API change: every existing call site must be
migrated (add `.await`, propagate/handle the `Result`, and supply the new `ctx` +
`destination` arguments). This design fixes only the target contract and the
architectural fact that no caller may any longer mint a permit without an
`AuthorizationProvider` capability check. **Enumerating and migrating each concrete
call site is tasks.md's responsibility** — the audit already indicates current
issuers are effectively test-only, but the exhaustive call-site inventory (and its
per-site migration steps) belongs to the task breakdown, not here.

Satisfies: **FR-005, FR-006**.

### AD-009 — Enforcement becomes fallible; the macro emits `?` before the body (FR-009, FR-013)

**Decision.** Change the signature and the generated call:

```rust
// runtime_builder.rs  (was: `fn enforce_tenant(&self, _ctx: &ServiceContext) {}`)
pub fn enforce_tenant(&self, ctx: &mut ServiceContext) -> Result<(), SecurityError>;
```

`ctx` is `&mut` because `enforce_tenant` is the sole writer of `ctx`'s
resolver-derived tenant value on success (AD-011) — an immutable reference
cannot satisfy `set_resolved_tenant(&mut self, ..)`, and no AD introduces
interior mutability for that field.

Internally `enforce_tenant` builds the resolver inputs from `ctx`
(`security = ctx.security()`, `supplied = ctx.tenant_hint()`), calls
`TenantResolver::resolve`, and on success stashes the `CanonicalTenant` for the
operation (AD-011). On failure it returns the error.

The macro's `enforce_tenant_block` (`lib.rs:296-300`) changes, **for
`#[tenant_scoped]` operations only**, from best-effort:

```rust
if let Some(rt) = self.runtime.upgrade() { rt.enforce_tenant(&ctx); }   // today
```

to fallible, placed before the inner call so the **body is never entered on
failure** (FR-009):

```rust
let rt = self.runtime.upgrade().ok_or(SecurityError::MissingContext)?;
rt.enforce_tenant(&mut ctx)?;   // aborts before inner_ref.method(..); writes ctx.resolved_tenant on success
```

Unmarked operations keep the current best-effort no-fail path. This makes
`openspec/specs/service-sdk/spec.md:76`'s `self.enforce_tenant(&ctx)?` and INV-003
true statements (FR-013).

Satisfies: **FR-009, FR-013**.

### AD-010 — Error taxonomy: extend `SecurityError` minimally; reuse `MissingContext` (Open Question, FR-012)

**Decision.** FR-012 requires three *distinguishable* conditions, not a single
enum. Home them on `SecurityError` (`crates/security-sdk/src/error/mod.rs`) — the
type the enforcement path already returns and the macro already maps through:

| Condition (FR) | Variant | New? |
|---|---|---|
| Caller tenant ≠ Principal tenant (FR-002) | `SecurityError::TenantMismatch { expected, actual }` | **Add** |
| Neither authenticated nor internal-permitted (FR-004) | `SecurityError::MissingContext` | **Reuse** (exists) |
| Unauthorized cross-tenant (FR-005) | `SecurityError::CrossTenantDenied { reason }` | **Add** |

Only two new variants — `MissingContext` already covers FR-004 (spec allows
`MissingAuthentication` OR `MissingContext`), and a dedicated `CrossTenantDenied`
keeps the cross-tenant denial distinguishable from a plain resource/action
`AuthorizationDenied`.

**Tenant-ID exposure boundary (`Debug` vs `Display` vs external).** The
`{ expected, actual }` tenant identifiers on `TenantMismatch` are the sensitive
part. The design fixes exactly where they may and may not appear:

- **`Display` (and any user-/wire-/log-facing rendering): MUST redact.** No raw
  tenant identifier appears in `Display`, in error responses returned to callers,
  or in structured log fields intended for external sinks (security skill Rule 3 —
  do not log private claims). The programmatic fields stay for `match`-based
  handling (NFR-003), but reading them for output is a deliberate act, not a
  side effect of formatting.
- **`Debug`: MAY contain the raw identifiers, for local diagnostics only.** Because
  `#[derive(Debug)]` would print `expected`/`actual` verbatim and many logging
  paths use `{:?}`, `Debug` is treated as an internal-diagnostics channel, NOT a
  safe external surface. Any log line that could reach an external sink MUST format
  the error via `Display`, never `{:?}`. (If a hand-written `Debug` is cheaper to
  reason about than this discipline, tasks.md may redact in `Debug` too — the
  invariant is "no tenant ID crosses an external boundary," not the specific
  formatter.)
- **External-facing responses: never carry either identifier** — a caller learns
  only that a mismatch occurred, not which tenants were involved (avoids
  cross-tenant enumeration).

This closes an accidental-leakage vector the bare "`Display` redacts" note left
implicit.

**Error-conversion model (clarification, no new mechanism).** Tenant enforcement
returns `SecurityError`, exactly as `#[authorize]` already does today. Service
operations convert it into their own error type through the **existing**
`From<SecurityError>` model the macro already generates for `#[authorize]`
(`<#err_ty as From<SecurityError>>::from(..)`, `service-sdk-macros/src/lib.rs:243-276`).
CORE-008A introduces no new error-conversion mechanism, no new trait, and no
parallel path alongside this one — `enforce_tenant`'s `?` relies on the same
`From<SecurityError>` bound already required of every `#[operation]`'s error type.

Satisfies: **FR-012** (and NFR-003).

### AD-011 — `ServiceContext` shape: demote `tenant_id` to a hint, add a resolver-only `resolved_tenant` (FR-002, FR-010, FR-011, FR-014)

**Decision.** Minimal, migration-safe change to
`crates/service-sdk/src/context/mod.rs`:

- **Keep** `pub tenant_id: Option<String>` (avoid breaking every construction site
  — the proposal's High-likelihood risk). Redocument it as a **non-authoritative
  ingress hint**: it is a resolver *input* only. Per FR-010, a mutation that
  disagrees with the Principal is never authoritative for enforcement — satisfied
  because enforcement reads the canonical value below, not this field.
- **Add** a resolver-only-writable, publicly-readable field:

  ```rust
  resolved_tenant: Option<CanonicalTenant>,     // set ONLY via pub(crate) setter
  pub fn canonical_tenant(&self) -> Option<&CanonicalTenant> { self.resolved_tenant.as_ref() }
  ```

  **Naming note (internal vs. public):** `resolved_tenant` is the private field —
  internal storage, never referenced outside this module. `canonical_tenant()` is
  the public accessor that reads it — the name every caller outside `context/mod.rs`
  should use. The two names are not synonyms for two different values; they are the
  storage/API pair for the same one authoritative value, matching the existing
  private-field/public-accessor pattern already used elsewhere in this struct.

  There is **no public setter**; `enforce_tenant` (AD-009) is the only writer
  (`pub(crate)` within service-sdk). This gives the authenticated path its
  automatic derivation (FR-002: no manual `with_tenant_id` needed — the resolver
  fills `resolved_tenant` from `Principal.tenant_id`), guarantees a canonical value
  is present at execution start (FR-011), and — having no public mutator — is
  immutable for the operation's duration (FR-014). Cloning a `ServiceContext`
  carries `resolved_tenant`; downstream code can neither overwrite it nor forge one
  (AD-003/AD-004).

**Accessor naming — canonical vs. legacy hint (transition strategy).** The
existing public getters `tenant_id()` and `has_tenant()`
(`crates/service-sdk/src/context/mod.rs:252-266`) today return the raw
`tenant_id` field — i.e. the ingress hint, *not* the authoritative value. Leaving
them under those obvious names is a trap: a service author reaching for
`ctx.tenant_id()` would read the hint and believe it is the enforced tenant. The
design resolves this with an explicit, unambiguous naming split:

| Accessor | Meaning after this change | Status |
|---|---|---|
| `canonical_tenant()` (new, AD-011) | The authoritative, resolver-produced `CanonicalTenant` — the ONLY value enforcement and cross-tenant checks read | **Canonical.** The name every service author should reach for. |
| `tenant_hint()` / `has_tenant_hint()` (new names for the hint) | The non-authoritative ingress hint (the old `tenant_id` field) | **Hint accessor.** Honest name; safe to read as "what the caller *asked for*, before validation". |
| `tenant_id()` / `has_tenant()` (existing names) | Same value as the hint accessors | **Legacy, deprecated.** Retained only to avoid breaking existing call sites during migration; documented as deprecated and slated for removal. |

**Transition rule:** `tenant_id()`/`has_tenant()` are marked deprecated (with a
doc note pointing readers to `canonical_tenant()` for the enforced value and to
`tenant_hint()` for the raw ingress value) at the moment this change lands. Their
**removal is a scoped follow-up** — deferred only because deleting them now would
force a churn of every current caller, which belongs to the migration sequencing
(tasks.md enumerates those callers), not to this design. The name that means "the
tenant this operation is enforced against" is `canonical_tenant()`, unambiguously
and permanently; the deprecated names never regain authoritative meaning. This
leaves zero ambiguity about which accessor returns which value.

**`SecurityContext` does not change** — a deliberate non-decision. It already
carries `Principal.tenant_id` (`principal: Principal`, non-optional,
`context/mod.rs:21`), which AD-006 designates the authoritative input. Adding a
tenant field to `SecurityContext` would create a *fifth* representation, the
opposite of FR-008.

Satisfies: **FR-002, FR-010, FR-011, FR-014**.

### AD-012 — `RuntimeBuilder` registers a tenant ENFORCEMENT mode, not a resolver plugin (FR-003, FR-011)

**Decision.** Because the resolver is concrete (AD-001), the builder registers
configuration, not a `dyn`. Add one method to `RuntimeBuilder`
(`crates/service-sdk/src/runtime/builder.rs`), consistent with `with_security` /
`with_logger`:

```rust
pub enum TenantEnforcementMode {
    /// Default. Only authenticated principals resolve a tenant (FR-002).
    /// Unauthenticated tenant-scoped calls fail closed with MissingContext (FR-004).
    AuthenticatedOnly,
    /// Additionally permit an explicit system/internal caller-supplied tenant (FR-003).
    AllowSystemInternal,
}

impl RuntimeBuilder { pub fn with_tenant_enforcement_mode(self, mode: TenantEnforcementMode) -> Self; }
```

The mode flows into `RuntimeInner` and is the "runtime explicitly permits that
mode" knob FR-003 requires. Default `AuthenticatedOnly` gives FR-004 its
fail-closed default (neither authenticated nor internal-permitted → error). The
runtime constructs its `TenantResolver` from this mode; no `Arc<dyn TenantResolver>`
is registered because there is nothing pluggable to register.

**`Runtime::for_test()` default (explicit, not left to tasks to infer):**
`for_test()` uses `TenantEnforcementMode::AuthenticatedOnly`, the same default as
`RuntimeBuilder::build()` — there is no separate, more permissive default for
tests. A test that needs `AllowSystemInternal` calls
`with_tenant_enforcement_mode(TenantEnforcementMode::AllowSystemInternal)`
explicitly, same as production code would.

**Naming disambiguation (mandatory — collision with CORE-016).** The term "tenant
mode" is ALREADY reserved in this codebase for a genuinely different concept: the
`RuntimeBuilder` docstring (`crates/service-sdk/src/runtime/builder.rs:18-25`)
states that the persistence-side tenant mode (`single_tenant_mode` / `tenant_id`,
CORE-016) belongs to `persistent_entity::EntityRuntimeBuilder`, not to this
builder. That is a **persistence/storage** knob; this AD's concept is an
**enforcement/resolution** knob. To keep two unrelated concepts from sharing one
name, the enforcement-side type is named `TenantEnforcementMode` and its builder
method `with_tenant_enforcement_mode(...)` — deliberately distinct from the
persistence `single_tenant_mode`/`tenant_id` configuration. The
`builder.rs:18-25` docstring is updated at apply time to note both: persistence
tenant mode lives on `EntityRuntimeBuilder`; enforcement mode
(`TenantEnforcementMode`) is set here via `with_tenant_enforcement_mode`. No
identifier and no prose in this change reuses the bare phrase "tenant mode" for the
enforcement concept.

**Configuration immutability (clarification, no new mechanism).**
`TenantEnforcementMode` is selected once, during `RuntimeBuilder` construction,
exactly like `with_security`/`with_logger`. Once `build()` produces `RuntimeInner`,
the mode is fixed for that runtime's lifetime — there is no setter to change it
afterward. Changing enforcement policy means constructing a new `Runtime` with a
different `with_tenant_enforcement_mode(..)` call, not mutating an existing one.
This follows the same immutable-after-construction pattern `RuntimeBuilder`'s other
configuration already has; it introduces no new mechanism.

Satisfies: **FR-003, FR-011**.

### AD-013 — Transport-independent Tenant Resolution (D4-mandated AD)

**Decision.** The runtime and service-operation layer consume an
already-resolved, already-validated `CanonicalTenant` and reference **no**
transport-specific type (no HTTP header, no gRPC metadata, no
extraction logic) — FR-007. Each **future** transport adapter (HTTP/axum,
gRPC/tonic) will implement its **own** extractor that:

1. extracts the credential and turns it into a `SecurityContext` via the existing
   `AuthenticationProvider`, and
2. extracts any raw tenant hint (e.g. `x-tenant` header / gRPC metadata),

then hands both to the **same** concrete `TenantResolver::resolve` (AD-001). All
extractors converge on that one contract; the resolver's fixed D2 policy is applied
identically regardless of transport.

**No transport code is implemented in this change** — the transport layer barely
exists (axum declared-unused, no `tonic` in the workspace). This AD fixes the
convergence rule so future adapters cannot re-fragment tenant resolution.

Satisfies: **FR-007, Non-Goals**.

---

## Data Flow

```
                         (future, OUT OF SCOPE — AD-013)
  HTTP/gRPC request ──► transport extractor ──► SecurityContext + raw tenant hint
                                                       │
  (today: test harness / builder supplies these) ─────┤
                                                       ▼
        ServiceContext { security, tenant_id(hint) } ──► #[tenant_scoped] #[operation]
                                                       │  (macro-generated proxy)
                                                       ▼
                          rt.enforce_tenant(&ctx)?  ── AD-009 (aborts here on Err)
                                                       │
                                                       ▼
        TenantResolver::resolve(security, supplied_tenant)   ── AD-001, D2 policy:
             ├─ Some(principal) & principal.tenant_id=None ─► Err(MissingContext)    (FR-002, gap fix)
             ├─ Some(principal) & hint agrees / absent ─► CanonicalTenant::Scoped   (FR-002)
             ├─ Some(principal) & hint disagrees ───────► Err(TenantMismatch)        (FR-002)
             ├─ None & mode=AllowSystemInternal & hint ─► CanonicalTenant::Scoped    (FR-003)
             ├─ None & op not tenant-scoped ────────────► CanonicalTenant::Systemwide(FR-001)
             └─ None & tenant-scoped & not permitted ───► Err(MissingContext)        (FR-004)
                                                       │  Ok(canonical)
                                                       ▼
        ctx.resolved_tenant = canonical (pub(crate) setter, no public mutator) ── AD-011, FR-014

  Gap fix: an authenticated Principal with no tenant claim (`principal.tenant_id
  == None`) never falls through to the hint-based branches — a caller-supplied
  hint is never trusted as a substitute for a missing Principal tenant claim on
  the authenticated path, since that would reopen the exact untrusted-input
  trust boundary this change closes (D2). This branch always fails closed,
  independent of whatever hint value is present.
                                                       │
                                                       ▼
                              operation body  ── reads ctx.canonical_tenant()  (FR-011)

  Cross-tenant path (AD-008):
    issue_cross_tenant_permit(&ctx, dest).await
        └─► authorize_in_context(principal, "tenant:cross-tenant-access") ─► Allow ─► CrossTenantPermit{dest, issued_to}  (FR-006)
                                                                          └─► Deny  ─► Err(CrossTenantDenied)             (FR-005)
```

---

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/service-sdk/src/runtime/tenant.rs` | Create | `CanonicalTenant` enum (AD-002/003/004), `TenantResolver` + `TenantEnforcementMode` (AD-001/012), the fixed D2 `resolve` algorithm; inline rejection-path tests (NFR-001/003). |
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modify | `enforce_tenant` → `Result<(), SecurityError>` running the resolver (AD-009); `issue_cross_tenant_permit` → `async` + fallible + authorization-gated + destination-scoped (AD-008); store `TenantEnforcementMode`. |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | Add `with_tenant_enforcement_mode(TenantEnforcementMode)`; thread mode into `RuntimeInner`; update the `:18-25` docstring to disambiguate enforcement mode (here) from persistence tenant mode (`EntityRuntimeBuilder`, CORE-016) (AD-012). |
| `crates/service-sdk/src/runtime/permit.rs` | Modify | `CrossTenantPermit` gains `{ destination: TenantId, issued_to: SubjectId }`; drop `Copy` (keep `Clone`) (AD-008). |
| `crates/service-sdk/src/context/mod.rs` | Modify | Redocument `tenant_id` field as a non-authoritative hint; add resolver-only `resolved_tenant` + `canonical_tenant()` reader (no public setter); add honest `tenant_hint()`/`has_tenant_hint()` accessors; deprecate the legacy `tenant_id()`/`has_tenant()` getters (`:252-266`) with a doc pointer to `canonical_tenant()`, removal scoped to a follow-up (AD-011). |
| `crates/service-sdk-macros/src/lib.rs` | Modify | `SdkAttr` learns `#[tenant_scoped]` (AD-007); emit fallible `enforce_tenant(&ctx)?` for marked ops, current best-effort for unmarked (AD-009). |
| `crates/security-sdk/src/error/mod.rs` | Modify | Add `TenantMismatch { expected, actual }` (redacting `Display`; raw IDs confined to `Debug`/internal diagnostics per AD-010 exposure boundary) and `CrossTenantDenied { reason }`; reuse `MissingContext` (AD-010). |
| `crates/security-sdk/src/authorization/*` | Read/reuse | Cross-tenant capability check reuses `authorize_in_context`; no trait-shape change (spec Assumption). |
| `openspec/specs/service-sdk/spec.md` | Modify (delta) | `:76` and INV-003 (`:427`) become true statements (FR-013) — at apply time. |
| Tests (service-sdk, security-sdk) | Add | Rejection + positive cross-tenant + immutability coverage (NFR-001/002/003). |
| Automated `#[tenant_scoped]` detection (lint / macro diagnostic / CI check — exact form decided in tasks) | Add | AD-007 fail-open mitigation, in scope for this change: flags apparently-tenant-scoped ops missing the marker and runs in CI. Not a manual checklist. |

---

## Breaking Changes

Consolidated migration reference — every breaking API change already decided by
an AD above, gathered in one place. No new breakage is introduced here; this
section only indexes what AD-008/AD-009/AD-011/AD-012 already fix.

| API | Before | After | AD |
|---|---|---|---|
| `RuntimeInner::enforce_tenant` | `fn enforce_tenant(&self, _ctx: &ServiceContext)` — no-op, infallible | `fn enforce_tenant(&self, ctx: &mut ServiceContext) -> Result<(), SecurityError>` — fallible, mutates | AD-009 |
| `RuntimeInner::issue_cross_tenant_permit` | sync, zero-arg, infallible, mints unconditionally | `async fn issue_cross_tenant_permit(&self, ctx: &ServiceContext, destination: TenantId) -> Result<CrossTenantPermit, SecurityError>` | AD-008 |
| `CrossTenantPermit` | `Copy` | `Copy` dropped, `Clone` kept; gains `{ destination, issued_to }` fields | AD-008 |
| `ServiceContext::tenant_id()` / `has_tenant()` | Canonical accessors | `#[deprecated]`, superseded by `tenant_hint()`/`canonical_tenant()`; kept functional during the migration window, removal is a scoped follow-up | AD-011 |
| `RuntimeBuilder` | No tenant configuration | Adds `with_tenant_enforcement_mode(TenantEnforcementMode)`, fixed at construction (see AD-012 immutability note) | AD-012 |

Each of these already has dedicated coverage in tasks.md (call-site migration for
the permit signature, compatibility tests for the deprecated accessors) — this
table adds no new requirement.

---

## Interfaces / Contracts (design-level sketches)

```rust
// AD-001 / AD-002 / AD-012 — crates/service-sdk/src/runtime/tenant.rs
pub struct CanonicalTenant(Repr);   // opaque wrapper (AD-003 note above), not a raw public enum
enum Repr { Scoped(ego_domain::context::TenantId), Systemwide }
pub enum TenantEnforcementMode { AuthenticatedOnly, AllowSystemInternal }

pub struct TenantResolver { mode: TenantEnforcementMode }
impl TenantResolver {
    pub(crate) fn resolve(
        &self,
        security: Option<&SecurityContext>,
        supplied_tenant: Option<&str>,
        // tenant_scoped-ness is decided by the macro (AD-007); a non-scoped op that
        // reaches here in Systemwide mode yields CanonicalTenant::Systemwide.
    ) -> Result<CanonicalTenant, SecurityError>;
}

// AD-009 — runtime_builder.rs
impl RuntimeInner {
    pub fn enforce_tenant(&self, ctx: &mut ServiceContext) -> Result<(), SecurityError>;
    pub(crate) async fn issue_cross_tenant_permit(
        &self, ctx: &ServiceContext, destination: TenantId,
    ) -> Result<CrossTenantPermit, SecurityError>;               // AD-008
}

// AD-011 — context/mod.rs
impl ServiceContext {
    pub fn canonical_tenant(&self) -> Option<&CanonicalTenant>;  // CANONICAL, read-only
    pub fn tenant_hint(&self) -> Option<&str>;                   // honest name for the ingress hint
    pub fn has_tenant_hint(&self) -> bool;
    // tenant_id()/has_tenant() retained as DEPRECATED aliases of the hint accessors
    // (removal is a scoped follow-up; see AD-011 transition rule).
    pub(crate) fn set_resolved_tenant(&mut self, t: CanonicalTenant); // resolver-only writer
}
```

---

## FR Traceability

| FR | Satisfied by |
|----|--------------|
| FR-001 — fail-closed is operation-level | AD-007 (`#[tenant_scoped]`), AD-002 (`Systemwide` arm) |
| FR-002 — Principal is canonical, mismatch is hard error | AD-006, AD-011, Data Flow |
| FR-003 — explicit system/internal mode | AD-012 (`AllowSystemInternal`), Data Flow |
| FR-004 — neither → fail closed | AD-010 (`MissingContext`), AD-012 default |
| FR-005 — permit requires authorized capability | AD-008, AD-010 (`CrossTenantDenied`) |
| FR-006 — authorized cross-tenant succeeds | AD-008 |
| FR-007 — runtime is transport-independent | AD-001, AD-013 |
| FR-008 — exactly one canonical representation | AD-002, AD-003, AD-005, AD-006 |
| FR-009 — fallible enforcement aborts before body | AD-009 |
| FR-010 — ServiceContext not a parallel authority | AD-011, AD-004 |
| FR-011 — canonical tenant available before execution | AD-011, AD-012 |
| FR-012 — distinguishable error taxonomy | AD-010 |
| FR-013 — spec `:76`/INV-003 match behavior | AD-009 |
| FR-014 — tenant immutable during execution | AD-004, AD-011 |
| NFR-001/002/003 — rejection + positive + distinguishable-error tests | AD-010; tests in File Changes |

---

## Migration / Rollout (Open Question 6)

Sequenced to keep the transient window (AD-006) safe and to honor the proposal's
breakage risks:

1. **Errors + canonical type first** (`SecurityError` variants, `CanonicalTenant`,
   `TenantResolver`, `TenantEnforcementMode`) — additive, nothing wired yet.
2. **Enforcement path** (`enforce_tenant` fallible, `with_tenant_enforcement_mode`,
   `resolved_tenant` on `ServiceContext`) — still inert because no operation is
   marked `#[tenant_scoped]` yet, so all existing ops keep passing.
3. **Macro** learns `#[tenant_scoped]` and emits `?` for marked ops only — existing
   unmarked ops unchanged (mitigates "no-op → fail-closed breaks everything").
4. **Cross-tenant issuance** becomes authorization-gated + destination-scoped
   (`Copy` dropped) — audit shows current issuers are effectively test-only.
5. **Adopt markers** on genuinely tenant-scoped operations; add rejection/positive
   tests (NFR-001/002/003); flip the spec-delta `:76`/INV-003 (FR-013).

Throughout, `Principal.tenant_id` is the authoritative input and the resolver
output is the authoritative runtime value; any two disagreeing ingress values are a
`TenantMismatch`, never a silent pick. No data or persisted-format migration
(persistence multitenancy is D6, out of scope). Rollback = revert the PR(s).

---

## Open Questions

None. All nine spec Open Questions are answered:
Q1 → AD-002 · Q2 → AD-003 · Q3/Q4 → AD-004 · Q5 → AD-005 · Q6 → AD-006 +
Migration · Q7 → AD-007 · Q8 → AD-008 · Q9 → AD-001.

**In scope for THIS change (not deferred):** the automated detection that flags
apparently-tenant-scoped operations missing the `#[tenant_scoped]` marker (AD-007).

**Deliberate scoped follow-ups (not open questions, explicitly out of this
change):**
1. Flipping AD-007's default from opt-in to secure-by-default once markers are
   adopted (AD-007 transitional note).
2. Removing the deprecated `tenant_id()`/`has_tenant()` accessors once callers move
   to `canonical_tenant()`/`tenant_hint()` (AD-011 transition rule).
3. Structural convergence/removal of `RuntimeExecutionContext` (AD-006 scope note).
4. Audit/observability for tenant enforcement violation events (`TenantMismatch`,
   `CrossTenantDenied`, `MissingContext`) — today the macro's `?` aborts before
   the interceptor chain's `on_error` hook runs (`lib.rs:296-317`), so no
   existing logging path observes these failures. A future change should emit
   a structured, redacted (per AD-010's exposure boundary) event on enforcement
   failure.
5. Persistence multitenancy (D6) — this change guarantees the tenant is
   resolved and validated by the time a request reaches the application layer;
   enforcing it at the data layer (e.g. `WHERE tenant_id = $1` filtering) is a
   distinct, unscheduled follow-up.
6. `TenantResolver::resolve`'s hot-path allocation (code-review finding,
   deliberately NOT fixed here) — every successful resolution re-validates
   and re-allocates a `TenantId`/`String` from `Principal.tenant_id`, even
   though the same bytes already exist as an owned `String` on `Principal`.
   `resolve()`'s `&SecurityContext` shared-reference signature makes this
   allocation structurally unavoidable without a real API change (e.g.
   `Principal.tenant_id` becoming an `Arc<str>`-backed `TenantId`, or
   `resolve()` consuming an owned `SecurityContext`) — and `Principal` is
   `security-sdk`'s type, not `service-sdk`'s, so that change is out of this
   AD's scope. Left as a named follow-up rather than reopening
   `security-sdk`'s `Principal` shape unilaterally from within CORE-008A.
   Tracked in [ego-rs#139](https://github.com/pablogore/ego-rs/issues/139) —
   scoping confirmed it touches 4 crates (`security-sdk`, `security-jwt`,
   `testkit`, `service-sdk`), not just `security-sdk`.

None of these is an open architectural question; each has a settled direction and a
named future home.

---

## Mandatory Task Seeds for tasks.md

Not blockers to freezing this design — every item below already has a settled
architectural direction in the ADs above. Listed explicitly here so `sdd-tasks`
cannot silently drop them:

1. **Automated `#[tenant_scoped]` detection ships in this change** (AD-007). The
   concrete mechanism (lint, macro-time diagnostic, CI workspace check, or
   equivalent) is a tasks/implementation decision — but *some* automated
   detection is mandatory, not a deferred follow-up.
2. **Complete call-site migration inventory**, enumerated by tasks.md: every
   caller of `tenant_id()` → `tenant_hint()` and `has_tenant()` →
   `has_tenant_hint()` (AD-011), plus every caller of
   `issue_cross_tenant_permit(...)` affected by its signature change to
   async/fallible/destination-scoped (AD-008).
3. **Concurrency tests, not only functional tests**, at minimum: two concurrent
   operations carrying different tenant hints, retried calls, `ServiceContext`
   clone behavior under tenant resolution, and a `CrossTenantPermit` proven
   non-reusable for a destination other than the one it was issued for
   (AD-008).
4. **Compatibility tests for the deprecation window**: the deprecated
   `tenant_id()`/`has_tenant()` accessors (AD-011) must be proven to keep
   functioning correctly for the duration of the migration window, not merely
   marked deprecated and left untested.
