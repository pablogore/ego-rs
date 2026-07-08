# Proposal: CORE-008A — Canonical Tenant Model & Runtime Enforcement

Converts tenant enforcement from a documented-but-absent invariant into a real,
fail-closed runtime guarantee backed by a single canonical tenant model.
Completes the work tracked as "TASK-014" (~20 stale references across code and
archived docs). This is NOT "implement the body of `enforce_tenant()`" — it is
the definition of one tenant authority and the enforcement contract around it.

## Intent

Today the framework *talks about* tenancy everywhere (1,201 case-insensitive
"tenant" hits repo-wide) but *enforces* it nowhere. The only runtime hook is a
literal no-op:

```rust
// crates/service-sdk/src/runtime/runtime_builder.rs:229
pub fn enforce_tenant(&self, _ctx: &ServiceContext) {}
```

It is called unconditionally from every macro-generated `#[operation]` method
(`crates/service-sdk-macros/src/lib.rs:296-298`), so every service operation
appears tenant-checked while checking nothing. Meanwhile there are at least
four disconnected tenant representations — `Principal.tenant_id` (from JWT),
`ServiceContext.tenant_id` (a public mutable `Option<String>` any holder can
overwrite), `ExecutionContext`/`TenantId` in the domain crate, and
`ClaimSet::tenant()` — with zero code synchronizing any of them. A caller can
authenticate as tenant A and execute as tenant B by setting one field.

This change establishes exactly one canonical in-runtime tenant model, defines
who may create/derive it and when it becomes immutable, and makes the runtime
fail closed on tenant-scoped operations before they reach the application
layer.

## Findings (verified against code)

| # | Question | Answer | Evidence |
|---|----------|--------|----------|
| 1 | Is tenant enforcement real? | No — literal no-op | `enforce_tenant(&self, _ctx: &ServiceContext) {}` (`crates/service-sdk/src/runtime/runtime_builder.rs:229`), comment "pending TASK-014"; emitted into every `#[operation]` (`crates/service-sdk-macros/src/lib.rs:296-298`) |
| 2 | Is `ServiceContext.tenant_id` protected? | No | `pub tenant_id: Option<String>` — public mutable field (`crates/service-sdk/src/context/mod.rs:48-50`); the `with_tenant_id()` builder (`:98-101`) is bypassable by direct field write. Only `allow_cross_tenant` has a compile-fail guard; `tenant_id` does not |
| 3 | Are `Principal.tenant_id` and `ServiceContext.tenant_id` synchronized? | Never | `Principal.tenant_id` set from JWT (`crates/security-jwt/src/principal_mapper.rs:117-129`); zero code copies it into `ServiceContext` or validates them against each other |
| 4 | Does authorization see tenant? | No | `AuthorizationProvider::authorize` has no tenant parameter (`crates/security-sdk/src/authorization/mod.rs:24-36`); `SecurityContext` = `{principal, claims}` (`context/mod.rs:21-26`); `AccessRequest` = `{resource, action}` (`authorization/access_request.rs:36-41`); `RbacProvider` never reads `principal.tenant_id` |
| 5 | Is `CrossTenantPermit` authorized? | No | Zero-sized compile-time witness (`crates/service-sdk/src/runtime/permit.rs:34`); `issue_cross_tenant_permit()` (`runtime_builder.rs:240-242`) mints it with zero runtime authorization check |
| 6 | Single tenant representation? | No — at least four | `TenantId` newtype (`crates/domain/src/context.rs:48`), `ExecutionContext::tenant_id()` (`crates/domain/src/context.rs:76`) duplicated by `RuntimeExecutionContext` (`crates/runtime/src/context.rs:12-89`), `ServiceContext.tenant_id`, `Principal.tenant_id`, `ClaimSet::tenant()` — all `Option`, none treated as an error when `None` |
| 7 | Can a resolver/provider be registered? | No | `RuntimeBuilder` (`crates/service-sdk/src/runtime/builder.rs:26`) exposes only new/with_security/with_logger/with_adapter/with_config/build — no tenant hook of any kind |
| 8 | Do tenant errors exist? | Barely | No tenant variant in `SecurityError` (full enum checked); `RuntimeError::CrossTenantViolation`/`CrossTenantDenied` appear only in archived design docs, never defined in code; `PersistenceError::MissingTenant` (`crates/domain/src/persistence/error.rs:28`) constructed only by the in-memory backend for the `Some("")` case |
| 9 | Do docs match code? | No | `openspec/specs/service-sdk/spec.md:76` mandates fallible `self.enforce_tenant(&ctx)?` — actual signature returns `()` and is called without `?`; `spec.md:427` documents "INV-003 — Tenant Enforcement Preserved" for an invariant that is not enforced; `docs/architecture.md:89` claims "TaskLocal-scoped" context, removed in `2026-06-22-remove-ambient-service-context` |
| 10 | Do tests exercise enforcement? | No | Existing tests (context_propagation, context_explicit_propagation, context_cross_service, smoke.rs:202-210, cross_tenant_access_contract) verify only field survival through clone/spawn/pass-through; `MissingTenant` appears in zero test files; no mismatch/rejection test exists |

Transport corroboration (drives the scope boundary): no `x-tenant`/header
extraction exists anywhere; `crates/transport/` contains only
`GrpcServerConfig` with axum declared but never imported, and no `tonic`
dependency exists in the workspace (PRD.md:177: "gRPC adapter not started").
There is nothing upstream of the runtime to integrate with yet.

## Desired Outcome

The specification phase must define, as observable contracts (not mechanisms):

- **Canonical Tenant Model.** Exactly one canonical in-runtime representation
  of tenant. JWT claims, HTTP headers, gRPC metadata, and builders are mere
  ingress mechanisms that MUST converge onto that one model before any
  operation executes — no JWT-tenant / ServiceContext-tenant /
  ExecutionContext-tenant / Principal-tenant drifting apart. The proposal
  intentionally leaves the lifecycle questions open for `design.md` (see Open
  Questions below).
- **Fail-closed enforcement, scoped.** Tenant-scoped operations fail closed
  when tenant cannot be resolved and validated. A valid tenant-less
  system/single-tenant execution mode remains — fail-closed applies only to
  tenant-scoped operations, not globally.
- **Tenant authority resolution.** When a Principal is authenticated (JWT/API
  key/OIDC), `Principal.tenant_id` is canonical and the Runtime derives the
  service-visible tenant from it automatically — framework users stop calling
  `ServiceContext::with_tenant_id()` manually on the authenticated path. Any
  pre-existing mismatch between a caller-supplied tenant and the Principal's
  tenant is a hard `TenantMismatch` error, never silently resolved. Flow:
  JWT → AuthenticationProvider → Principal → SecurityContext → Runtime →
  ServiceContext (tenant already derived, not user-copied).
- **Explicit system/internal request path.** Unauthenticated calls go through
  a separate, explicit "system/internal request" flow — a distinct execution
  mode, not an exception to mismatch handling. A caller-supplied tenant is
  valid there only if the runtime explicitly permits that mode. Neither
  authenticated nor internal-request-enabled →
  `MissingAuthentication`/`MissingContext`.
- **Authorized cross-tenant access.** `CrossTenantPermit` is issued only to
  principals holding an explicit role/capability (e.g. "system-admin",
  "cross-tenant-migration") verified through `AuthorizationProvider`. Being
  authorized on the resource/action alone is NOT sufficient.
- **Canonical resolution as the upper boundary.** CORE-008A owns everything
  from the point where a tenant is already resolved and validated, downward
  through Runtime → Service. The runtime never knows about HTTP or gRPC; it
  receives an already-resolved tenant. The exact resolution contract — its
  name, whether it becomes a `TenantResolver` trait/port or an extension of
  an existing contract — is a design.md decision. The only place this
  proposal fixes the `TenantResolver` name is the transport AD below (D4),
  where the locked decision requires it verbatim.
- **Fallible enforcement surface.** The enforcement hook becomes genuinely
  fallible, aligning code with the already-published contract at
  `openspec/specs/service-sdk/spec.md:76` and making INV-003 (`spec.md:427`)
  true instead of aspirational.
- **Rejection-path tests.** Tests that exercise missing-tenant rejection,
  tenant mismatch, unauthorized cross-tenant access, and the permitted
  system/internal mode — not just field pass-through.

## Capabilities

### New Capabilities
- `tenant-enforcement`: canonical tenant model, resolution authority,
  fail-closed runtime enforcement, and cross-tenant permit authorization.

### Modified Capabilities
- `service-sdk`: `openspec/specs/service-sdk/spec.md:76` (enforcement call
  contract) and `:427` (INV-003) move from documented-but-false to enforced.

## Non-Goals

- **Transport extraction (HTTP/axum, gRPC/tonic).** No header/metadata
  extraction code is implemented in this change — the transport layer barely
  exists in this repo (axum unused, tonic absent). This is simultaneously a
  non-goal and a forward-looking architecture decision: `design.md` MUST
  include an AD ("Transport-independent Tenant Resolution") stating that each
  future transport adapter implements its own extractor and ALL extractors
  must converge on the same `TenantResolver` contract.
- **Persistence multitenancy.** No repository changes, no SQL filters, no DB
  adapter changes (the untested Postgres `WHERE tenant_id = $1` paths stay as
  they are). CORE-008A only guarantees the tenant is resolved and validated by
  the time a request reaches the application layer.
- Fixing stale documentation beyond what this change makes true/false
  (e.g. `docs/architecture.md:89` TaskLocal claim is corrected only where it
  intersects tenant context).
- Prescribing synchronization primitives, storage, or type shapes — spec and
  design phases decide.

## Product Decisions (resolved 2026-07-07)

| # | Question | Decision |
|---|----------|----------|
| D1 | Is fail-closed global? | No. Fail-closed applies only to tenant-scoped operations; a valid tenant-less system/single-tenant mode remains a first-class execution mode. |
| D2 | Who is the tenant authority? | Authenticated path: `Principal.tenant_id` is canonical; the Runtime derives `ServiceContext.tenant_id` automatically and users stop calling `with_tenant_id()` manually there; any mismatch is a hard `TenantMismatch`, never silently resolved. Unauthenticated path: a separate, explicit system/internal request mode (not an exception to mismatch handling) where a caller-supplied tenant is valid only if the runtime explicitly permits that mode. Neither → `MissingAuthentication`/`MissingContext`. |
| D3 | Who gets `CrossTenantPermit`? | Only principals with an explicit role/capability (e.g. "system-admin", "cross-tenant-migration") checked via `AuthorizationProvider`. Resource/action authorization alone is not enough. |
| D4 | Transport in scope? | No. CORE-008A owns the canonical tenant resolution boundary → Runtime → Service; the exact resolution contract (name, trait/port shape) is defined in `design.md`. Transport extractors are future adapters that must converge on whatever that contract turns out to be, captured as an explicit AD in `design.md`. |
| D5 | One tenant model or many? | Exactly one canonical in-runtime representation. All ingress mechanisms converge onto it before an operation executes. Its lifecycle is a design-phase question (see Open Questions). |
| D6 | Persistence multitenancy? | Out of scope. No repository/SQL/adapter changes; the guarantee ends at "tenant resolved and validated before the application layer". |

## Open Questions for design.md

**Per D5 (canonical model, lifecycle)** — `design.md` MUST explicitly answer:

1. Where does the canonical tenant finally live (which type, which crate)?
2. Who can create it?
3. Who can modify it?
4. At what point does it stop being mutable?
5. Who owns its lifecycle?
6. How is transient coexistence of multiple tenant representations handled
   during incremental rollout — which representation is authoritative if two
   are populated and not yet converged?

**Per D1 (fail-closed scope)**:

7. How does the runtime/macro determine whether a given `#[operation]` is
   tenant-scoped (opt-out attribute, presence of tenant in the request,
   execution-mode configured on `RuntimeBuilder`, or something else)? D1 is
   not testable without this criterion.

**Per D3 (`CrossTenantPermit`)**:

8. Is `CrossTenantPermit` scoped to a (source, destination) pair / time
   window / purpose, or does it remain a global capability? This affects
   whether the permit type can stay `Copy`/`Clone`
   (`crates/service-sdk/src/runtime/permit.rs:21-29` already flags this as a
   pending fork: per-grant re-authorization would require removing `Copy`, a
   breaking change).

**Per D4 (resolution contract)**:

9. Does the canonical resolution mechanism become a new trait/port (e.g. a
   `TenantResolver`), or an extension of an existing contract (e.g.
   `AuthenticationProvider`)?

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/service-sdk/src/context/mod.rs` | Modified | `pub tenant_id: Option<String>` cannot remain freely mutable; derivation replaces manual `with_tenant_id()` on the authenticated path |
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modified | No-op `enforce_tenant()` becomes fallible enforcement; `issue_cross_tenant_permit()` becomes authorization-gated; the runtime needs some way to enable the explicit system/internal execution mode (mechanism decided in design) |
| `crates/service-sdk/src/runtime/builder.rs` | Modified | Some mechanism to register/consume the canonical resolved tenant (exact form decided in design) |
| `crates/service-sdk/src/runtime/permit.rs` | Modified | `CrossTenantPermit` issuance contract |
| `crates/service-sdk-macros/src/lib.rs` | Modified | `#[operation]` expansion calls the now-fallible enforcement (`?`) |
| `crates/security-sdk/src/authorization/*` | Modified | Capability check backing D3 (surface decided in design) |
| `crates/domain/src/context.rs`, `crates/runtime/src/context.rs` | Modified | Fragmented `TenantId`/`ExecutionContext` representations converge on the canonical model |
| `openspec/specs/service-sdk/spec.md` | Modified | `:76` and INV-003 (`:427`) become true statements |
| Tests (service-sdk, security-sdk) | Added | Rejection-path coverage: missing tenant, mismatch, unauthorized cross-tenant, permitted internal mode |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Breaking change for callers that set `ServiceContext.tenant_id` manually (public field + builder are today's only mechanism) | High | Spec phase enumerates current construction sites; the tenant-less/internal mode gives non-authenticated callers a supported path instead of silent breakage |
| Turning a no-op into fail-closed breaks existing examples/tests that never resolve a tenant | High | D1 scoping: enforcement applies to tenant-scoped operations only; migration notes for the explicit modes |
| `issue_cross_tenant_permit()` becoming authorization-gated breaks current call sites that mint permits freely | Med | Audit shows the permit is a zero-check witness today; spec enumerates existing issuers before the gate lands |
| Canonical-model convergence touches domain + runtime + service-sdk simultaneously (wide blast radius) | Med | Design answers the lifecycle questions first; tasks phase slices by layer with the resolver contract as the stable seam |
| Design under-specifies the internal/system mode and it becomes a de facto enforcement bypass | Med | D2 makes it an explicit, runtime-permitted execution mode with its own error (`MissingAuthentication`/`MissingContext`) — never a fallthrough |
| Multiple tenant representations (JWT/Principal/ServiceContext/ExecutionContext/TenantId) coexist temporarily during incremental rollout, since convergence spans domain + runtime + service-sdk in separate stages | Med | Design must state which representation is authoritative during the transition; tasks phase sequences convergence to minimize the window |

## Rollback Plan

Planning artifacts only until apply. Implementation lands behind normal PR
revert; no data or persisted-format migration is involved — persistence is
explicitly out of scope (D6).

## Dependencies

- Audit: engram `sdd/core-008a-tenant-enforcement/explore` (verified
  file:line findings above).
- Locked decisions: engram `sdd/core-008a-tenant-enforcement/decisions`
  (D1–D6).
- Prior art: `2026-06-22-remove-ambient-service-context` (explicit-by-value
  context propagation — the canonical model must respect it),
  CORE-014 authorization providers (D3 builds on `AuthorizationProvider`).

## Success Criteria

- [ ] Spec phase defines the contracts under Desired Outcome without
      inheriting implementation details from this proposal.
- [ ] Every finding (1–10) maps to at least one requirement or explicit
      non-goal.
- [ ] All six product decisions (D1–D6) appear as requirements or ADs; none
      is softened, reordered, or dropped.
- [ ] `design.md` contains the "Transport-independent Tenant Resolution" AD
      (D4) and explicit answers to all 9 Open Questions (D1, D3, D4, D5).
- [ ] `openspec/specs/service-sdk/spec.md:76` and INV-003 describe behavior
      the code actually enforces.
- [ ] Rejection paths (missing tenant, mismatch, unauthorized cross-tenant,
      permitted internal mode) each have at least one acceptance scenario.
- [ ] An authorized cross-tenant access (valid `CrossTenantPermit`) has at
      least one acceptance scenario — not only rejection paths.
- [ ] `ServiceContext` no longer behaving as a parallel writable authority on
      the authenticated path has at least one acceptance scenario.
- [ ] A tenant enforcement failure aborts execution before the service
      operation body is entered, regardless of the mechanism design.md
      chooses (`?`, interceptor, middleware, wrapper) — has at least one
      acceptance scenario.
