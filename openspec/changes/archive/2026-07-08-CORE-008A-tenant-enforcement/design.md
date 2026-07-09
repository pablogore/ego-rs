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
`Serialize`/`Deserialize` nor exposed to transport (FR-007/AD-002). This is
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

### AD-013 — Fact Establishment vs. Policy Evaluation

**Decision.** Components responsible for policy evaluation MUST derive their
decision exclusively from a closed, immutable set of Established Facts. They
MUST NOT establish new facts during evaluation, nor perform the action implied
by the decision they produce.

**Pipeline:**

```
Infrastructure
  ↓
Fact Establishment
  ↓
Established Facts
  ↓
Policy Evaluation
  ↓
Decision
  ↓
Enforcement
```

**Definition.** A Policy Evaluator is a component whose sole responsibility is
to derive a deterministic decision from a closed, immutable set of Established
Facts. It neither establishes new facts during evaluation nor performs the
action implied by the decision.

**Operational rule.** A component belongs to Policy Evaluation only if its
decision depends exclusively on facts already present in its input. If the
component can establish additional facts during execution — whether by
discovering, querying, observing mutable state, or any other mechanism — it
belongs to Fact Establishment or to another stage, not to Policy Evaluation.

**Scope.** Framework-wide, as the preferred architectural shape for
policy-evaluating components. Deviation requires explicit architectural
justification, not merely convenience.

**Evidence available today** (not a claim of universal conformance — Config,
TestKit, and the logging subsystem were not audited):

- `TenantResolver::resolve()` (`crates/service-sdk/src/runtime/tenant.rs`) —
  Policy Evaluator. Depends only on already-resolved `SecurityContext`/hint;
  establishes nothing during evaluation.
- `AuthorizationProvider`/`RbacProvider`
  (`crates/security-sdk/src/authorization/`,
  `crates/security-sdk/src/providers/rbac/`) — Fact Establishment.
  `authorize()` is `async` and fetches permissions mid-decision
  (`self.store.permissions_for_role(role).await?`) — it establishes the fact
  it then reasons over, so it is not itself a Policy Evaluator, even though it
  also contains matching logic internally.
- `ServiceRegistry::resolve()` (DI) — Policy Evaluator. Pure, in-memory,
  deterministic lookup over already-registered facts.

**Consequence for FR-006.** Cross-tenant authorization (`CrossTenantPermit` /
the resulting grant) must reach `TenantResolver` as an Established Fact,
already established by the authorization subsystem before policy evaluation
begins — never as a callback performed during resolution.

**Status.** This AD defines the architectural seam for closing the FR-006
gap (`CrossTenantPermit` issuance exists; consumption in the enforcement
path does not — confirmed by direct verification, not yet implemented). It
does not itself satisfy FR-006; the implementation that wires the
cross-tenant grant into `TenantResolver::resolve()` as an Established Fact
is a separate, still-pending change.

(See tasks.md for implementation-specific Notes and Phase descriptions, and see proposal.md/spec.md for full architectural decisions AD-007 through AD-012.)
