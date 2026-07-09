# Design: CORE-024 — Validate `Principal.tenant_id` once at construction

## Context

The proposal decided the WHAT: change `Principal.tenant_id` from
`Option<String>` to `Option<TenantId>`, validate at JWT-mapping time, and let
`TenantResolver::resolve()` clone the pre-validated value instead of
re-running `TenantId::new()` per request. This document fixes the exact
implementation shape (signatures, error variant, diff shape, migration order)
so the tasks phase is mechanical.

No sequence diagram: the flow is synchronous and single-hop (JWT claim →
`Principal` field → `resolve()` clone). A diagram would add nothing over the
signatures below.

## Architecture approach

Type-driven validation: push the validation to the type boundary and let
`TenantId`'s existing constructor be the single validation gate. Once the
field is `Option<TenantId>`, "is this tenant valid?" is answered by the type
system, not by re-checking at each read. This is the standard
"parse, don't validate" move — validation happens once, at the edge
(`DefaultPrincipalMapper::map`), and every downstream consumer receives a
value that is valid by construction.

No new abstraction, no new trait, no new error type. The change is a field
type swap plus caller updates.

## Component changes and exact signatures

### 1. `ego-domain` — unchanged

`TenantId` (`crates/domain/src/context.rs:48`, via `id_type!` macro
lines 9-40) already exists, is `pub`, wraps `String`, and validates
non-empty-after-trim in `TenantId::new(impl Into<String>) -> Result<Self, TenantIdError>`.
No change. security-sdk and testkit already depend on ego-domain
(security-sdk Cargo.toml:18; testkit Cargo.toml:17), so no new dependency
edge and no circular dependency is introduced.

### 2. `ego-security-sdk` — field + builder

`crates/security-sdk/src/principal/principal.rs`:

- Add import: `use ego_domain::context::TenantId;`
- Field (`:63`): `pub tenant_id: Option<TenantId>,`
- Builder (`:83-86`):

```rust
/// Builder: sets the tenant id. Takes a pre-validated `TenantId`;
/// validation is the caller's responsibility (the type is the proof).
pub fn with_tenant_id(mut self, tenant_id: TenantId) -> Self {
    self.tenant_id = Some(tenant_id);
    self
}
```

Signature changes from `impl Into<String>` to `TenantId`. Infallible — no
`Result`, mirroring `with_role`/`with_attribute`. `Principal::new()` still
sets `tenant_id: None`.

Tests in this file (`:140-152`, `:212-218`) construct via
`.with_tenant_id("acme")` and assert `Some("acme".into())`. Updated to
`.with_tenant_id(TenantId::new("acme").unwrap())` and
`assert_eq!(p.tenant_id.as_ref().map(TenantId::as_str), Some("acme"))`
(or `assert_eq!(p.tenant_id, Some(TenantId::new("acme").unwrap()))`).

### 3. `ego-security-jwt` — the production validation site

`crates/security-jwt/src/principal_mapper.rs:117-130`. This is where the raw
claim string is validated exactly once. `map()` already returns
`Result<(Principal, Claims), AuthenticationError>`, so the failure has a
natural home with no new plumbing.

Current `:128-130`:

```rust
if let Some(ref tid) = tenant_id {
    principal = principal.with_tenant_id(tid.clone());
}
```

Becomes:

```rust
if let Some(ref tid) = tenant_id {
    let tenant = TenantId::new(tid.clone())
        .map_err(|_| AuthenticationError::InvalidToken("invalid tenant claim".into()))?;
    principal = principal.with_tenant_id(tenant);
}
```

Add import: `use ego_domain::context::TenantId;` (crate already depends on
ego-domain — see the existing `use ego_domain::auth::...` imports).

The extraction block at `:117-126` (choosing `tenant_id`/`tid`/`tenant`) is
unchanged; it still produces `Option<String>` as the raw claim, and the empty
tenant-key removal logic (`tenant_key_consumed`) is unaffected.

### AD-1 — Error variant: reuse `AuthenticationError::InvalidToken`

**Decision:** reuse the existing `AuthenticationError::InvalidToken(String)`
variant (`crates/domain/src/auth/error.rs:22-23`) with message
`"invalid tenant claim"`.

**Rationale:** This exact function already uses
`InvalidToken("invalid subject id".into())` at `:94` when `SubjectId::new()`
fails — the tenant claim is the structurally-identical sibling case (a claim
present in the token but failing its typed-value invariant). The domain spec
(`openspec/specs/domain/auth.md:86`, CLAR-005) explicitly documents that
`InvalidToken` covers "wrong-type / structurally invalid claim values", not
only malformed token envelopes. A blank/whitespace tenant claim is a bad claim
value, so `InvalidToken` is the precise fit.

**Rejected — new `InvalidTenantClaim` variant:** `AuthenticationError` is a
plain (non-`#[non_exhaustive]`) enum consumed by exhaustive matches across
security-jwt; adding a variant is a wider breaking change than the problem
warrants, and it would be inconsistent with the adjacent `SubjectId` failure
that already funnels through `InvalidToken`. The proposal authorized a new
variant ONLY if no existing variant could communicate the failure clearly —
`InvalidToken` communicates it clearly, so no new variant.

`MissingClaim` is NOT used here: a missing tenant claim is legitimately
`None` (tenant is optional on `Principal`), never an error.

### 4. `ego-service-sdk` — resolve() clones, `validated()` deleted

`crates/service-sdk/src/runtime/tenant.rs`.

**Delete** the `validated()` helper entirely (`:155-160`) — validation has
moved upstream to JWT mapping. `SecurityError` is unchanged (still used for
`MissingContext`/`TenantMismatch`).

`resolve()` (`:118-153`) new body. `security.principal().tenant_id` is now
`Option<TenantId>`; read it as `Option<&TenantId>` via `.as_ref()` (was
`.as_deref()` at `:124`):

```rust
match security {
    Some(security) => match security.principal().tenant_id.as_ref() {
        None => Err(SecurityError::MissingContext),
        Some(principal_tenant) => match supplied_tenant {
            None => Ok(CanonicalTenant::scoped(principal_tenant.clone())),
            Some(hint)
                if hint.trim().is_empty() || hint == principal_tenant.as_str() =>
            {
                Ok(CanonicalTenant::scoped(principal_tenant.clone()))
            }
            Some(hint) => Err(SecurityError::TenantMismatch {
                expected: principal_tenant.as_str().to_string(),
                actual: hint.to_string(),
            }),
        },
    },
    None => match (self.mode, supplied_tenant) {
        // (d) System-internal hint is untrusted raw input — parse it
        // inline here (validated() is deleted, see AD-2). This is the
        // ONLY remaining raw-string→TenantId parse in resolve().
        (TenantEnforcementMode::AllowSystemInternal, Some(hint)) => {
            TenantId::new(hint)
                .map(CanonicalTenant::scoped)
                .map_err(|_| SecurityError::MissingContext)
        }
        _ => Err(SecurityError::MissingContext),
    },
}
```

Add import: `use ego_domain::context::TenantId;` (already present at
`crates/service-sdk/src/runtime/tenant.rs:9`). `TenantId::new(hint)` returns
`Result<TenantId, TenantIdError>`; the `map_err` converts the parse failure to
`SecurityError::MissingContext`, matching the deleted helper's exact behavior.

**Branch (d) still needs validation.** The system/internal path
(`None` security, `AllowSystemInternal`, `Some(hint)`) receives a RAW
caller-supplied `&str` hint that was never validated at a login boundary, so
it MUST still be parsed into a `TenantId`. Two options:

- **Keep `validated()` only for branch (d)** — but the proposal says delete
  it. The honest resolution: branch (d)'s hint is not a `Principal` claim; it
  is a fresh untrusted string, so it genuinely still needs a construction-time
  parse. Replace the helper call with an inline `TenantId::new(hint)`:
  `TenantId::new(hint).map(CanonicalTenant::scoped).map_err(|_| SecurityError::MissingContext)`.

**AD-2 — `validated()` is deleted; branch (d) inlines `TenantId::new`.**
The `validated()` helper existed to dedupe two call paths (Principal + hint).
After this change the Principal path clones a pre-validated value and no longer
validates, leaving `validated()` with a single caller (branch d). A
single-use private helper is not worth its own name, so inline it. The
Principal branches (a/b/c) perform ZERO validation — that is the perf win the
proposal targets. Branch (d) still validates because a system-internal hint is
untrusted input at that seam, not a post-login claim — this is unchanged
security behavior, correctly preserved.

Test helper `principal_with_tenant` (`:172-176`) currently does
`p.tenant_id = tenant.map(|t| t.to_string());`. Becomes:
`p.tenant_id = tenant.map(|t| TenantId::new(t).unwrap());`.

### 5. `ego-testkit` — `PrincipalBuilder::tenant()` stays ergonomic

`crates/testkit/src/identity.rs`. Keep the field
`tenant: Option<String>` (`:18`) and the setter `tenant(impl Into<String>)`
(`:49-52`) UNCHANGED — test fixtures stay one-liner friendly. Validation moves
into `build()` (`:75-77`):

```rust
if let Some(tenant) = self.tenant {
    let tenant_id = TenantId::new(tenant)
        .expect("PrincipalBuilder tenant must not be empty or whitespace-only");
    principal = principal.with_tenant_id(tenant_id);
}
```

Add import: `use ego_domain::context::TenantId;` (testkit already depends on
ego-domain — Cargo.toml:17).

**AD-3 — testkit validates in `build()` with `.expect()`, not `Result`.**
This mirrors the existing `SubjectId::new(self.subject).expect(...)` pattern
already in the same `build()` (`:72-73`). A bad fixture fails loudly at test
setup with a clear message — the desired behavior for test code. The public
builder ergonomics (`impl Into<String>`) are preserved; only the production
`with_tenant_id` becomes strictly typed. This is the deliberate asymmetry the
proposal called for: typed at the production boundary, ergonomic + panic at the
test boundary.

testkit tests asserting `p.tenant_id.as_deref()` (`:148`) become
`p.tenant_id.as_ref().map(TenantId::as_str)`.

### 6. Direct field-assignment test sites (service-sdk)

Four sites bypass the builder (proposal Decision 4). Each `.to_string()`
becomes `TenantId::new("...").unwrap()`:

- `crates/service-sdk/tests/tenant_scoped_codegen.rs:145`
- `crates/service-sdk/tests/common/mod.rs:22`
- `crates/service-sdk/src/runtime/tenant.rs:174` (covered in §4)
- `crates/service-sdk/src/runtime/runtime_builder.rs:660`

No field-visibility change (proposal Decision 4): the field stays `pub`, and
`Option<TenantId>` makes direct assignment safe by construction.

### 7. security-jwt tests

`principal_mapper.rs:347` and `tests/oidc_integration.rs:503` assert
`principal.tenant_id.as_deref() == Some("tenant-42")`. Both become
`principal.tenant_id.as_ref().map(TenantId::as_str) == Some("tenant-42")`.

Two further assertions in `validation.rs` read `Principal.tenant_id` directly:

- `validation.rs:461` — `assert_eq!(ctx.principal.tenant_id, None)`. UNCHANGED:
  `Option<TenantId>` still compares equal to `None`, so this compiles as-is.
- `validation.rs:478` — `assert_eq!(ctx.principal.tenant_id.as_deref(), Some("primary"))`.
  MUST change: `as_deref()` won't compile once the field is `Option<TenantId>`
  (`TenantId` has no `Deref<Target=str>`). Becomes
  `assert_eq!(ctx.principal.tenant_id.as_ref().map(TenantId::as_str), Some("primary"))`,
  the same treatment as `principal_mapper.rs:347` and `oidc_integration.rs:503`.
  Requires `use ego_domain::context::TenantId;` in that test module if not
  already imported.

## Data flow (after change)

```
JWT ClaimSet
  → DefaultPrincipalMapper::map()          [security-jwt]
      raw claim String → TenantId::new()?  ← ONLY validation point
      → Principal { tenant_id: Option<TenantId> }
  → SecurityContext (carries Principal)
  → TenantResolver::resolve()              [service-sdk]
      tenant_id.as_ref().clone()           ← no validation, just clone
      → CanonicalTenant::scoped(TenantId)
```

The only raw-string→`TenantId` parse on the authenticated path is now in
`map()`. Branch (d)'s system-internal hint parse is a separate, untrusted
ingress and is not on the authenticated path.

## AD-4 — Migration order: single atomic change

**Decision:** all five in-workspace crates land in ONE atomic change (one
commit / one PR). There is no safe half-migrated intermediate.

**Rationale:** `cargo` builds the whole workspace; the field type change in
`principal.rs:63` simultaneously breaks `with_tenant_id`'s signature (same
crate), the security-jwt caller, the testkit builder, and every service-sdk
call site. None of these compile until all are updated, and a PR must compile.
There is no ordering that yields a green intermediate state, so incremental
per-crate commits are not possible. The change is small (one field, one
builder, one validation add, ~8 mechanical call-site edits across 4 crates),
so a single atomic change is both necessary and appropriately sized.

Recommended edit sequence WITHIN the single change (for the author's sanity,
not separate commits): (1) domain — none; (2) security-sdk field + builder +
its tests; (3) security-jwt `map()` + its tests; (4) testkit `build()` + its
tests; (5) service-sdk `resolve()` + `validated()` deletion + all test sites.
Then `cargo build --workspace && cargo test --workspace` once at the end.

Rollback = `git revert` of the single commit (proposal Rollback Plan);
nothing is persisted or serialized differently, so revert fully restores prior
behavior.

## AD-5 — No dependency-graph change

Verified: security-sdk→ego-domain (Cargo.toml:18) and testkit→ego-domain
(Cargo.toml:17) already exist; security-jwt already imports `ego_domain::auth`.
No crate gains a new dependency, so no new edge and no cycle is introduced.
`ego-domain` remains a leaf (depended-upon, depends on none of these).

## Verification approach (per verify skill)

These are library crates with no binary surface. Verify through the public API:
a throwaway `crates/security-jwt/examples/verify_tenant_validation.rs` that
maps a `ClaimSet` with (a) a valid tenant claim → assert `Ok` and the
`Principal.tenant_id` is `Some(expected)`, and (b) a whitespace-only tenant
claim → assert `Err(AuthenticationError::InvalidToken(_))`. Delete the example
after running. The bulk of coverage is the existing tenant-enforcement test
suite (must stay green) plus the new invalid-claim-at-login assertion the spec
will require.

## Risks / open items for tasks phase

- **Behavioral shift (intended):** an invalid tenant claim now fails at login
  (`InvalidToken`) instead of mid-request (`MissingContext`). The spec must
  cover this WHERE-it-surfaces change; tasks must add a login-time
  invalid-tenant test.
- **`hint == principal_tenant` comparison:** must become
  `hint == principal_tenant.as_str()` (string compare against the newtype's
  inner). Easy to miss; called out explicitly in §4.
- **Non-goal reaffirmed:** `resolve()`'s clone is still a `String` clone
  (`TenantId` wraps `String`); no `Arc<str>` migration. Per-request
  allocation of the clone remains — the win is eliminating re-validation and
  moving failure to the boundary, not eliminating the clone.
