# Spec: CORE-008A — Canonical Tenant Model & Runtime Enforcement

## Purpose

Defines observable contracts for tenant identity and enforcement in
`service-sdk`: exactly one canonical in-runtime tenant representation, who
may derive it and when it is authoritative, fail-closed enforcement scoped
to tenant-scoped operations, and authorization-gated cross-tenant access.
New capability: `tenant-enforcement`. This is also a **delta** against
`openspec/specs/service-sdk/spec.md`: `:76` (the enforcement call contract)
and INV-003 (`:427`, "Tenant Enforcement Preserved") move from
documented-but-false to enforced, per Finding 9 and FR-013 below.

These requirements describe WHAT the runtime must guarantee, not HOW.
Several mechanisms are intentionally left open for `design.md` — see "Open
Questions Deferred to design.md". Acceptance scenarios below are phrased so
they hold regardless of how those questions are answered.

---

## Scope

| Domain | Spec Type | Description |
|--------|-----------|-------------|
| `tenant-enforcement` (new) | New capability | Canonical tenant model, resolution authority, fail-closed enforcement, cross-tenant permit authorization |
| `service-sdk` | Delta — MODIFIED requirement | `:76` enforcement call contract and INV-003 (`:427`) become true statements |

Base spec file for the delta: `openspec/specs/service-sdk/spec.md`.

Non-goals (transport extraction, persistence multitenancy, general doc
cleanup, primitive/storage prescriptions) are listed under "Non-Goals" below
and are unchanged from the proposal.

---

## Requirements

### FR-001 — Fail-Closed Scope Is Operation-Level, Not Global (D1)

Tenant-scoped operations MUST fail closed when the canonical tenant cannot
be resolved and validated for that operation. A valid tenant-less
system/single-tenant execution mode MUST remain available; fail-closed
enforcement applies only to operations classified as tenant-scoped, not to
every operation in the runtime. The mechanism used to classify an
individual `#[operation]` as tenant-scoped is a design.md decision (Open
Question 7); this requirement fixes only the observable behavior once that
classification exists.

#### Scenario: Tenant-scoped operation fails closed without resolvable tenant

- GIVEN an operation classified as tenant-scoped
- WHEN it is invoked and no canonical tenant can be resolved and validated
  for the call
- THEN the call fails with an explicit tenant error and the operation is
  not executed

#### Scenario: Non-tenant-scoped operation is unaffected by missing tenant

- GIVEN an operation classified as not tenant-scoped, running in a valid
  system/single-tenant execution mode
- WHEN it is invoked with no tenant present
- THEN the call proceeds and executes normally; no tenant error occurs

---

### FR-002 — Principal Is the Canonical Tenant Authority on the Authenticated Path (D2)

When a request is authenticated (a `Principal` exists via JWT/API
key/OIDC), `Principal.tenant_id` MUST be treated as canonical. The runtime
MUST derive the tenant visible to the service operation from
`Principal.tenant_id` automatically; the framework MUST NOT require a
caller to invoke a manual tenant-setting call on this path for the derived
value to be correct. If a tenant value was already supplied by the caller
before derivation and it disagrees with `Principal.tenant_id`, the call
MUST fail with a `TenantMismatch` error — the runtime MUST NOT silently
prefer either value. If the authenticated Principal carries no tenant claim
at all (`Principal.tenant_id` is absent), the runtime MUST NOT treat any
caller-supplied tenant value as a substitute for it — the call MUST fail
closed with `MissingContext`, regardless of whether a caller-supplied value
is present or absent, so a caller cannot assert a tenant identity an
authenticated Principal does not itself carry.

#### Scenario: Derivation from Principal succeeds without manual tenant assignment

- GIVEN an authenticated request whose Principal has `tenant_id =
  "tenant-a"` and no caller-supplied tenant conflicting with it
- WHEN the operation executes
- THEN the service-visible tenant is `"tenant-a"`, derived from the
  Principal, without the caller having invoked a manual tenant-setting call
  for it to be correct

#### Scenario: Caller-supplied tenant conflicting with Principal is a hard error

- GIVEN an authenticated request whose Principal has `tenant_id =
  "tenant-a"`
- AND a caller-supplied tenant value of `"tenant-b"` is present before
  derivation
- WHEN the operation is invoked
- THEN the call fails with `TenantMismatch`; neither `"tenant-a"` nor
  `"tenant-b"` is silently chosen

#### Scenario: Authenticated Principal without a tenant claim fails closed regardless of a caller-supplied hint

- GIVEN an authenticated request whose Principal carries no tenant claim
  (`Principal.tenant_id` is absent)
- AND a caller-supplied tenant value may or may not be present
- WHEN a tenant-scoped operation is invoked
- THEN the call fails with `MissingContext`; the caller-supplied value, if
  any, is never used as a substitute for the missing Principal tenant claim

---

### FR-003 — Explicit System/Internal Request Mode (D2)

An unauthenticated call (no `Principal`) MUST be routed through a distinct,
explicit system/internal request execution mode rather than being treated
as a variant or exception of the mismatch case in FR-002. A caller-supplied
tenant is valid in this mode only when the runtime has been explicitly
configured to permit it.

#### Scenario: Internal mode accepts caller-supplied tenant when explicitly permitted

- GIVEN the runtime is configured to permit the system/internal execution
  mode
- AND a call carries no Principal but supplies a tenant value
- WHEN the operation executes
- THEN the call proceeds using the supplied tenant, without being treated
  as a `TenantMismatch`

#### Scenario: Internal mode rejects tenant when not permitted

- GIVEN the runtime is NOT configured to permit the system/internal
  execution mode
- AND a call carries no Principal
- WHEN the operation is invoked
- THEN the call does not proceed as an authenticated-tenant call under
  FR-002; it is handled per FR-004

---

### FR-004 — Neither Authenticated Nor Internal-Permitted Fails Closed (D2)

A call that is neither authenticated (no `Principal`) nor covered by a
runtime-permitted system/internal execution mode MUST fail with an
explicit `MissingAuthentication` or `MissingContext` error before a
tenant-scoped operation body executes.

#### Scenario: Unauthenticated, non-internal call is rejected

- GIVEN a call with no Principal and the system/internal execution mode
  not permitted (or not applicable to this call)
- WHEN a tenant-scoped operation is invoked
- THEN the call fails with `MissingAuthentication` or `MissingContext`, and
  the operation body is never entered

---

### FR-005 — CrossTenantPermit Requires Authorized Capability (D3)

`CrossTenantPermit` MUST be issued only after `AuthorizationProvider`
confirms the requesting Principal holds an explicit cross-tenant
role/capability (e.g. "system-admin", "cross-tenant-migration"). Being
authorized on the target resource/action alone MUST NOT be sufficient to
obtain a permit.

#### Scenario: Permit denied for principal without cross-tenant capability

- GIVEN a Principal authorized for the requested resource/action but
  without a cross-tenant role/capability
- WHEN cross-tenant access is requested
- THEN no `CrossTenantPermit` is issued and the request fails

#### Scenario: Permit denied even with resource/action authorization alone

- GIVEN a Principal for whom `AuthorizationProvider` allows the
  resource/action but does not confirm a cross-tenant capability
- WHEN cross-tenant access is requested
- THEN the permit request fails; resource/action authorization alone does
  not satisfy FR-005

---

### FR-006 — Authorized Cross-Tenant Access Succeeds (D3)

A Principal holding an explicit cross-tenant role/capability, confirmed via
`AuthorizationProvider`, MUST be able to obtain a `CrossTenantPermit` and
successfully execute a cross-tenant operation using it.

#### Scenario: Authorized cross-tenant access succeeds end to end

- GIVEN a Principal with the "system-admin" (or equivalent) capability,
  confirmed by `AuthorizationProvider`
- WHEN the Principal requests cross-tenant access to a resource in a
  tenant other than its own and then invokes the cross-tenant operation
  with the resulting `CrossTenantPermit`
- THEN the permit is issued, the operation executes, and it is not
  rejected as a tenant violation

---

### FR-007 — Runtime Is Transport-Independent for Tenant Resolution (D4)

The runtime and service-operation layer MUST consume an already-resolved,
already-validated tenant. Neither MUST depend on any transport-specific
mechanism (HTTP headers, gRPC metadata, or any other transport concept) to
obtain or validate the tenant. The exact shape of the boundary that hands
the runtime this resolved tenant — a new trait/port or an extension of an
existing contract — is a design.md decision (Open Question 9); this
requirement fixes only that the runtime layer itself carries no transport
dependency.

#### Scenario: Runtime enforcement contains no transport-specific dependency

- GIVEN the runtime's tenant-enforcement code path
- WHEN reviewed for dependencies
- THEN it references no HTTP, gRPC, or other transport-specific type, or
  header/metadata extraction logic — only an already-resolved tenant value

---

### FR-008 — Exactly One Canonical In-Runtime Tenant Representation (D5)

Exactly one representation of tenant MUST be canonical inside the runtime
at the point an operation executes. JWT claims, HTTP headers, gRPC
metadata, and any builder-supplied value are ingress mechanisms only; each
MUST converge onto the single canonical representation before the
operation body runs. No two of `Principal.tenant_id`, `ServiceContext`'s
tenant field, `ExecutionContext`/`TenantId` (domain crate), and
`ClaimSet::tenant()` MUST be independently authoritative for the same
operation at execution time. The exact type, owning crate, and lifecycle
(creation, mutation, immutability point, ownership) of the canonical
representation are design.md decisions (Open Questions 1-6); this
requirement fixes only that convergence to one authoritative value happens
before execution.

#### Scenario: Divergent ingress values converge to one authoritative value

- GIVEN a request where a JWT claim, an existing `ServiceContext` tenant
  field, and any other ingress tenant source could, before this change,
  disagree
- WHEN the operation executes
- THEN exactly one tenant value is authoritative for that execution, and
  every downstream tenant-aware check (enforcement, cross-tenant
  authorization) reads that same value

---

### FR-009 — Tenant Enforcement Is Fallible and Aborts Before the Operation Body (Finding 1, INV-003)

Tenant enforcement MUST be capable of failing. When enforcement fails for a
tenant-scoped operation, the service operation's body MUST NOT be
entered — this MUST hold regardless of the mechanism design.md selects to
achieve it (early-return propagation, interceptor, middleware, wrapper, or
otherwise).

#### Scenario: Enforcement failure aborts execution before the body runs

- GIVEN a tenant-scoped operation whose tenant enforcement check fails
  (e.g. missing or mismatched tenant)
- WHEN the operation is invoked
- THEN the operation's body never executes and the caller observes the
  enforcement failure as the outcome of the call

#### Scenario: Enforcement success allows the body to run

- GIVEN a tenant-scoped operation whose tenant enforcement check succeeds
- WHEN the operation is invoked
- THEN the operation's body executes exactly as it would have without
  enforcement

---

### FR-010 — ServiceContext Is Not a Parallel Writable Tenant Authority on the Authenticated Path (Finding 2)

On the authenticated path, the service-visible tenant MUST be derived per
FR-002, not independently settable by arbitrary code holding a
`ServiceContext`. A tenant mutation that bypasses derivation and disagrees
with `Principal.tenant_id` MUST NOT be treated as authoritative for
enforcement.

#### Scenario: Direct tenant mutation cannot override the derived, authenticated tenant

- GIVEN an authenticated `ServiceContext` whose tenant was derived from
  `Principal.tenant_id = "tenant-a"`
- WHEN code attempts to set the service-visible tenant to a different
  value through any mutation path
- THEN the operation either rejects the attempt or still enforces against
  the Principal-derived tenant — the mutated value is never treated as
  authoritative for enforcement

---

### FR-011 — A Canonical Tenant Is Available Before Operation Execution (Finding 7)

Before a tenant-scoped operation executes, a canonical tenant value MUST
be available to the runtime for that operation. On the authenticated path,
this MUST NOT depend exclusively on manual per-call caller code (superseded
there by FR-002's automatic derivation). How the canonical tenant becomes
available — a new builder method, a new trait/port, an extension of an
existing contract, or any other mechanism — is a design.md decision (Open
Question 9); this requirement fixes only that the value is present when
execution begins.

#### Scenario: A canonical tenant is present at the start of execution without manual per-call assignment

- GIVEN an authenticated request
- WHEN the operation begins execution
- THEN a canonical tenant value is available to the runtime for that
  operation, without the calling code having manually set it per call

---

### FR-012 — Tenant Error Taxonomy Exists (Finding 8)

`TenantMismatch`, `MissingAuthentication`/`MissingContext`, and an explicit
unauthorized-cross-tenant error MUST each be distinguishable by callers —
reachable in code, not only referenced in documentation or archived design
notes. This spec does not prescribe a single enum or error type; the three
conditions may surface through `RuntimeError`, `ServiceError`,
`SecurityError`, or any combination design.md chooses.

#### Scenario: Each tenant failure mode is programmatically distinguishable

- GIVEN the three failure conditions defined in FR-002, FR-004, and FR-005
- WHEN each is triggered independently
- THEN a caller can distinguish which of the three occurred — no two
  conditions are indistinguishable to the caller, and none is only
  documented but unreachable in code

---

### FR-013 — service-sdk Spec Contract Matches Enforced Behavior (Finding 9)

`openspec/specs/service-sdk/spec.md:76` (the enforcement call contract) and
INV-003 (`:427`) MUST describe behavior the code actually enforces once
this change lands — a fallible enforcement check that can prevent the
operation body from executing, per FR-009.

#### Scenario: Published contract matches implementation

- GIVEN `openspec/specs/service-sdk/spec.md` after this change is applied
- WHEN `:76` and INV-003 are read against the actual enforcement code path
- THEN the documented behavior (fallible check, operation body not entered
  on failure) matches what the code does — no aspirational or outdated
  claim remains

---

### FR-014 — Tenant Authority Is Immutable During Operation Execution (Finding 2, D2)

Once the canonical tenant has been established for an operation (per
FR-002/FR-003/FR-011), the tenant used for enforcement MUST remain stable
for the duration of that operation. This makes explicit, as its own
requirement, an invariant otherwise only implicit between FR-002 and
FR-010: the audit found nothing today prevents a caller-held context from
being mutated mid-call-chain to a different tenant after resolution
(`crates/service-sdk/src/context/mod.rs:98-101` — `with_tenant_id` is a
consuming builder callable again on an already-resolved context).

#### Scenario: Downstream mutation attempts do not affect an operation already in progress

- GIVEN an operation whose canonical tenant has already been resolved
- WHEN downstream code attempts to alter the tenant information associated
  with that operation
- THEN all subsequent enforcement decisions for that operation observe the
  original canonical tenant, not the attempted alteration

---

## Test Coverage Requirements (NFR)

| # | Requirement |
|---|---|
| NFR-001 | Missing-tenant rejection, tenant mismatch, unauthorized cross-tenant access, and permitted system/internal mode (Finding 10) MUST each have dedicated test coverage — not only field pass-through tests like the existing `context_propagation`/`context_explicit_propagation`/`context_cross_service`/`cross_tenant_access_contract` suites. |
| NFR-002 | Authorized cross-tenant access (a valid `CrossTenantPermit` actually granted and used) MUST have dedicated positive-path test coverage — not only rejection-path tests. |
| NFR-003 | Each rejection-path test MUST assert on a distinguishable error value from FR-012, not merely "the call failed". |

---

## Non-Goals

- **Transport extraction (HTTP/axum, gRPC/tonic).** No header/metadata
  extraction code is implemented in this change. `design.md` MUST include
  an AD ("Transport-independent Tenant Resolution") stating that each
  future transport adapter implements its own extractor and all extractors
  must converge on whatever resolution contract FR-007/FR-011 resolve to
  (locked name for that AD: `TenantResolver`, per D4).
- **Persistence multitenancy.** No repository changes, no SQL filters, no
  DB adapter changes. This spec's guarantee ends at "tenant resolved and
  validated by the time a request reaches the application layer" (D6).
- **General documentation cleanup.** Stale tenant-adjacent docs are
  corrected only where they intersect this change's requirements (e.g.
  FR-013); `docs/architecture.md:89`'s unrelated TaskLocal claim is out of
  scope beyond its tenant-context intersection.
- **Prescribing synchronization primitives, storage, or type shapes.**
  Left to design.md and tasks.

---

## Findings Traceability

| Finding | Covered By |
|---|---|
| 1 — `enforce_tenant()` is a literal no-op | FR-009 |
| 2 — `ServiceContext.tenant_id` is public and mutable, unprotected | FR-010, FR-014 |
| 3 — `Principal.tenant_id` and `ServiceContext.tenant_id` never synchronized | FR-002 |
| 4 — `AuthorizationProvider` is tenant-blind | FR-005, FR-006 |
| 5 — `CrossTenantPermit` minted with zero runtime authorization check | FR-005 |
| 6 — At least four disconnected tenant representations | FR-008 |
| 7 — No resolver/provider registration hook on `RuntimeBuilder` | FR-011 |
| 8 — Tenant errors barely exist (`SecurityError`, `RuntimeError` variants absent from code) | FR-012 |
| 9 — Docs (`spec.md:76`, INV-003) describe behavior the code doesn't enforce | FR-013 |
| 10 — No test exercises rejection or mismatch | NFR-001, NFR-002, NFR-003 |

## Product Decisions Traceability

| Decision | Covered By |
|---|---|
| D1 — Fail-closed scope is operation-level, not global | FR-001 |
| D2 — Tenant authority resolution (authenticated + internal + neither) | FR-002, FR-003, FR-004, FR-014 |
| D3 — `CrossTenantPermit` requires explicit capability | FR-005, FR-006 |
| D4 — Transport out of scope; runtime is transport-independent | FR-007, Non-Goals |
| D5 — Exactly one canonical tenant model | FR-008 |
| D6 — Persistence multitenancy out of scope | Non-Goals |

## Open Questions Deferred to design.md

These are NOT answered by this spec. Each acceptance scenario above is
written to hold under any answer design.md gives.

| # | Question | Affects |
|---|---|---|
| 1 | Where does the canonical tenant finally live (type, crate)? | FR-008 |
| 2 | Who can create the canonical tenant value? | FR-008 |
| 3 | Who can modify it? | FR-008, FR-010, FR-014 |
| 4 | At what point does it stop being mutable? | FR-008, FR-010, FR-014 |
| 5 | Who owns its lifecycle? | FR-008 |
| 6 | How is transient coexistence of multiple tenant representations handled during incremental rollout — which is authoritative if two are populated and not yet converged? | FR-008 |
| 7 | How does the runtime/macro determine whether a given `#[operation]` is tenant-scoped? | FR-001 |
| 8 | Is `CrossTenantPermit` scoped to a (source, destination) pair / time window / purpose, or a global capability? | FR-005, FR-006 |
| 9 | Does the resolution mechanism become a new trait/port (e.g. `TenantResolver`) or an extension of an existing contract (e.g. `AuthenticationProvider`)? | FR-007, FR-011 |

---

## Success Criteria

- [x] Every finding (1-10) maps to at least one requirement — see Findings
      Traceability.
- [x] All six product decisions (D1-D6) appear as requirements or
      Non-Goals, none softened, reordered, or dropped — see Product
      Decisions Traceability.
- [x] Rejection paths (missing tenant, mismatch, unauthorized cross-tenant,
      permitted internal mode) each have at least one acceptance scenario
      — FR-002, FR-003, FR-004, FR-005.
- [x] An authorized cross-tenant access has at least one acceptance
      scenario, not only rejection paths — FR-006.
- [x] `ServiceContext` no longer behaving as a parallel writable authority
      on the authenticated path has at least one acceptance scenario —
      FR-010.
- [x] A tenant enforcement failure aborts execution before the service
      operation body is entered, regardless of mechanism — FR-009.
- [x] Tenant authority remains stable for the duration of an operation once
      established — FR-014.
- [ ] `design.md` contains the "Transport-independent Tenant Resolution"
      AD (D4) and explicit answers to all 9 Open Questions — pending
      design phase, not this spec's deliverable.
- [ ] `openspec/specs/service-sdk/spec.md:76` and INV-003 describe behavior
      the code actually enforces — FR-013 defines the contract; satisfied
      at apply/archive time, not by this spec document itself.

## Assumptions

- This spec fixes observable contracts only; it does not choose the
  canonical tenant's type, crate, or the resolution boundary's exact shape
  — those are explicit design.md dependencies (Open Questions 1-9).
- "Tenant-scoped" is a classification that exists per FR-001, but its
  criterion is not yet defined; FR-001's scenarios hold for whatever
  classification mechanism design.md selects.
- `AuthorizationProvider` (CORE-014) is assumed capable of expressing a
  cross-tenant role/capability check as used by FR-005/FR-006; this spec
  does not require changing `AuthorizationProvider`'s trait shape itself.
