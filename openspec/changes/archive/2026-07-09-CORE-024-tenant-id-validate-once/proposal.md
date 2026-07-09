# Proposal: CORE-024 — Validate `Principal.tenant_id` once at construction

**Source:** GitHub issue [#139](https://github.com/pablogore/ego-rs/issues/139)
**Origin:** Flagged as a deliberate follow-up during CORE-008A review — see `openspec/changes/archive/2026-07-08-CORE-008A-tenant-enforcement/tasks.md:148` ("Notes for Future Reference", item on per-call `TenantId` re-allocation, ego-rs#139).

## Why

`Principal.tenant_id` is `Option<String>`, stored unvalidated at parse time
(`crates/security-sdk/src/principal/principal.rs:63`). Every tenant-scoped
request calls `TenantResolver::resolve()`
(`crates/service-sdk/src/runtime/tenant.rs:118-161`), which re-validates and
re-allocates that string into a `TenantId` via the private `validated()`
helper — on EVERY call, even though the principal's tenant claim never changes
between login and each request. This is pure per-request waste, and it also
means an invalid tenant claim is only discovered deep inside request handling
instead of at the authentication boundary where it belongs.

## What Changes

Validate once, at `Principal` construction time (right after JWT mapping),
by changing the field type from `Option<String>` to `Option<TenantId>`.
`TenantResolver::resolve()` then clones the pre-validated value instead of
re-running validation.

### Decisions

1. **Field type**: `Principal.tenant_id: Option<TenantId>`. No new crate
   dependency needed — security-sdk already depends on `ego-domain`
   (Cargo.toml line 18).

2. **Builder shape — infallible, typed at the boundary**:
   - `Principal::with_tenant_id()` accepts a pre-validated `TenantId`
     directly. Validation is pushed to the caller. The production caller is
     `security-jwt`'s `DefaultPrincipalMapper::map()`
     (`crates/security-jwt/src/principal_mapper.rs:78-130`), which already
     returns `Result<(Principal, Claims), AuthenticationError>` — the natural
     home for the validation failure. No new fallible plumbing is needed.
   - `testkit`'s `PrincipalBuilder::tenant()` keeps its ergonomic
     `impl Into<String>` signature and validates in `build()` with
     `.expect(...)`, mirroring the existing `SubjectId::new(...).expect(...)`
     pattern already in `PrincipalBuilder::build()`
     (`crates/testkit/src/identity.rs:72-73`). Test fixtures stay one-liner
     friendly; a bad fixture fails loudly at test setup.

3. **Error path at login**: when a JWT-supplied tenant claim fails
   `TenantId::new()`, `DefaultPrincipalMapper::map()` returns the existing
   `AuthenticationError` — reuse an existing variant that already carries
   claim-level failures in this exact function (e.g. the `InvalidToken` /
   `MissingClaim` family). A dedicated `InvalidTenantClaim` variant is a
   design-phase refinement ONLY if no existing variant can communicate the
   failing claim clearly. Do not invent a new error hierarchy.

4. **Field visibility — unchanged, and that is deliberate**: 4 test sites
   currently bypass the builder via direct field assignment
   (`crates/service-sdk/tests/tenant_scoped_codegen.rs:145`,
   `tests/common/mod.rs:22`, `src/runtime/tenant.rs:174`,
   `src/runtime/runtime_builder.rs:660`). Once the field type is
   `Option<TenantId>`, direct assignment is safe by construction — you cannot
   build a `TenantId` without passing validation, so the type itself is the
   validation proof. Tightening visibility would add churn without adding
   safety. The 4 sites are updated mechanically to
   `Some(TenantId::new("...").unwrap())`.

5. **Validation rules unchanged**: `TenantId::new()`'s existing semantics
   (non-empty after trim, `crates/domain/src/context.rs:9-40` via `id_type!`)
   are preserved exactly. This change RELOCATES validation to construction
   time; it does not strengthen or weaken the rule.

### What "done" looks like

- `Principal.tenant_id` is `Option<TenantId>`; no raw tenant string survives
  past `Principal` construction.
- `TenantResolver::resolve()` performs zero validation — it clones the
  already-validated `TenantId` (still a `String` clone; see Non-goals).
- Invalid tenant claims fail at login with a well-typed
  `AuthenticationError`, not mid-request.
- All existing tenant-enforcement tests pass; touched test fixtures compile
  against the new type.

## Blast Radius

Four crates, all sites confirmed by exploration (`sdd/core-024-tenant-id-validate-once/explore`):

| Crate | Sites |
|---|---|
| security-sdk | `principal.rs:63` (field), `:83-86` (builder), tests at `:121,140-152,212,218` |
| security-jwt | `principal_mapper.rs:117-129` (validate + map), test `:347`, `tests/oidc_integration.rs:503` |
| testkit | `src/identity.rs:18,49-52,76` (`PrincipalBuilder`), tests `:111,123,148` |
| service-sdk | `runtime/tenant.rs:118-161` (resolve + delete `validated()`), `:172-176` (test helper), plus the 4 direct-assignment test sites above |

## Non-goals (explicitly out of scope)

1. **No `Arc<str>` migration.** The clone in `resolve()` is NOT made
   allocation-free — `TenantId` still wraps a `String`. Making clones cheap
   means migrating `TenantId` and its 6-sibling `id_type!` family (`EntityId`,
   `CorrelationId`, `CausationId`, `RequestId`, `AggregateId` — 5 siblings,
   6 types total; exploration corrected the issue's count of 4) to `Arc<str>`.
   That is a separate, bigger decision — future issue.
2. **No change to `ServiceContext.tenant_id` / `tenant_hint()`**
   (`crates/service-sdk/src/context/mod.rs:55,112-113`). That is a
   deliberately-raw ingress hint per AD-011 and a DIFFERENT concept from the
   authenticated principal's tenant claim. `testkit::TestContextBuilder`
   builds this hint and is likewise untouched.
3. **No change to tenant validation rules.** Same trim/non-empty check,
   relocated, not redesigned.
4. **No `Principal` field-visibility tightening** (see Decision 4 — the type
   change already delivers the safety).

## Impact and Risks

- **API shape**: `with_tenant_id()`'s signature changes from
  `impl Into<String>` to `TenantId` — a compile-time-visible breaking change
  for any in-workspace caller (all confirmed sites listed above). No external
  consumers exist outside the workspace. This does change `Principal`'s
  public contract: any future authentication provider (not just
  `security-jwt`'s JWT mapper) that constructs a `Principal` with a tenant
  claim MUST validate it via `TenantId::new()` before calling
  `with_tenant_id()` — the builder itself performs no validation, by design
  (see Decision 2).
- **Behavioral**: requests that today fail at first `resolve()` with an
  invalid tenant claim will instead fail at login. This is the intended
  improvement, but it moves WHERE the error surfaces — spec must cover it.
- **No persistence/wire impact**: nothing is serialized differently; the
  change is internal representation only.

## Rollback Plan

Low-risk internal representation change. Rollback = `git revert` of the
change commit(s). No data migration exists in either direction: nothing is
persisted or transmitted in a new shape, so reverting restores the previous
behavior completely. If the change ships in a chained PR, each slice reverts
independently since the type change and its call-site updates land together
(the workspace does not compile in a half-migrated state).
