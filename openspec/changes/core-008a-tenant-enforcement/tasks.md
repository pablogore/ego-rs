# Tasks: CORE-008A — Canonical Tenant Model & Runtime Enforcement

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~1300-1650 (new `runtime/tenant.rs` + tests, `context/mod.rs`, `runtime_builder.rs`, `builder.rs`, `permit.rs`, macro `lib.rs`, `security-sdk/error/mod.rs`, 2 new integration test files, 1 new scan-test file, 4 existing test files' call-site migration, 2 compile-fail tests + `.stderr`, `service-sdk/spec.md` delta) |
| Crates touched | `service-sdk`, `service-sdk-macros`, `security-sdk` + `openspec/specs/service-sdk/spec.md` delta |
| 400-line budget risk | High |
| Chained PRs recommended | **Yes** |
| Suggested split | PR1 → PR2 → PR3 → PR4 → PR5 → PR6 (see Work Units — mirrors design.md's own 5-step migration + a final integration/spec-delta slice) |
| Delivery strategy | ask-on-risk — **flagged, not decided here**: orchestrator must confirm chain strategy before apply |
| Chain strategy | pending |

Decision needed before apply: **Yes**
Chained PRs recommended: **Yes**
400-line budget risk: **High**

### Suggested Work Units

| Unit | Goal | Migration Step | Likely PR | Notes |
|------|------|-----------------|-----------|-------|
| 1 | Error taxonomy + `CanonicalTenant`/`TenantResolver`/`TenantEnforcementMode` | 1 — Errors + canonical type | PR 1 | Purely additive, nothing wired. ~250-300 lines. Crates: `security-sdk`, `service-sdk`. |
| 2 | `ServiceContext` resolver fields/accessors + fallible `enforce_tenant` + builder mode + macro unmarked-path signature fix (TASK-009B, gap fix) | 2 — Enforcement path | PR 2 | Still inert — no op marked `#[tenant_scoped]` yet, all existing ops keep passing. Includes a minimal `service-sdk-macros` edit (TASK-009B) so the workspace keeps compiling once `enforce_tenant` becomes `&mut`. ~220-270 lines. Crates: `service-sdk` + minimal `service-sdk-macros`. |
| 3 | Macro `#[tenant_scoped]` branch (adds to TASK-009B's already-updated unmarked path) + automated detection (mandatory seed 1) | 3 — Macro | PR 3 | ~130-180 lines. Crate: `service-sdk-macros` + one new scan-test in `service-sdk`. |
| 4 | Cross-tenant issuance authorization-gating + call-site migration (mandatory seed 2) | 4 — Cross-tenant issuance | PR 4 | ~150-200 lines. Crate: `service-sdk`. |
| 5 | Concurrency tests + deprecation-window compat tests (mandatory seeds 3 & 4) | — (test-only, precedes step 5's marker adoption) | PR 5 | ~250-300 lines, test-only. Crate: `service-sdk`. |
| 6 | Adopt markers + full FR/NFR acceptance suite + spec.md delta + final verification | 5 — Adopt markers | PR 6 | ~300-400 lines. Crate: `service-sdk` + `openspec/specs/service-sdk/spec.md`. |

If `feature-branch-chain` is chosen: PR1 base = tracker; PR2 base = PR1; PR3 base = PR2; PR4 base = PR3; PR5 base = PR4; PR6 base = PR5. If `stacked-to-main`: each PR merges to main in order, matching the migration sequencing so the transient window (AD-006) is never widened out of order.

## PR Chain Plan (resolved: `feature-branch-chain`)

Decided per the orchestrator's follow-up: `delivery_strategy = ask-on-risk` → chained PRs confirmed; `chain_strategy = feature-branch-chain`. Only the tracker branch merges to `develop`; every child PR targets the immediate previous branch, not `develop` directly.

**Tracker branch**: `opsx/core-008a-tenant-enforcement` — opened as a draft/no-merge PR to `develop`; merges to `develop` only after PR 6 is integrated into it.

| PR | Branch | Base (target) | Tasks | Phase |
|----|--------|----------------|-------|-------|
| 1 | `opsx/core-008a-tenant-enforcement-01-errors-canonical-type` | `opsx/core-008a-tenant-enforcement` (tracker) | TASK-001 – TASK-005 | Phase 1 |
| 2 | `opsx/core-008a-tenant-enforcement-02-enforcement-path` | PR 1 branch | TASK-006 – TASK-010 (incl. TASK-009B) | Phase 2 |
| 3 | `opsx/core-008a-tenant-enforcement-03-macro-tenant-scoped` | PR 2 branch | TASK-011 – TASK-014 | Phase 3 |
| 4 | `opsx/core-008a-tenant-enforcement-04-cross-tenant-issuance` | PR 3 branch | TASK-015 – TASK-019 | Phase 4 |
| 5 | `opsx/core-008a-tenant-enforcement-05-concurrency-deprecation-tests` | PR 4 branch | TASK-020 – TASK-024 | Phase 5 |
| 6 | `opsx/core-008a-tenant-enforcement-06-adopt-markers-acceptance-spec` | PR 5 branch | TASK-025 – TASK-034 | Phase 6 |

```text
develop
 └── opsx/core-008a-tenant-enforcement                              ← tracker, draft/no-merge until chain completes
      ↑ PR 1 base
      └── opsx/core-008a-tenant-enforcement-01-errors-canonical-type
           ↑ PR 2 base
           └── opsx/core-008a-tenant-enforcement-02-enforcement-path
                ↑ PR 3 base
                └── opsx/core-008a-tenant-enforcement-03-macro-tenant-scoped
                     ↑ PR 4 base
                     └── opsx/core-008a-tenant-enforcement-04-cross-tenant-issuance
                          ↑ PR 5 base
                          └── opsx/core-008a-tenant-enforcement-05-concurrency-deprecation-tests
                               ↑ PR 6 base
                               └── opsx/core-008a-tenant-enforcement-06-adopt-markers-acceptance-spec
```

Each child PR carries a Chain Context section (per `chained-pr` skill) marking its own position with `📍` in this diagram. No child PR merges to `develop`; PR 1 → PR 2 → … → PR 6 merge into each other in sequence, and only the tracker branch (`opsx/core-008a-tenant-enforcement`) merges to `develop` once PR 6 has landed on it.

### Merge Gates

Minimum conditions before merging each PR into the next one. Does not change the PR structure — this only makes explicit what "green" means at each step.

**PR1** (TASK-001–005)
- `cargo build --workspace` and `cargo test --workspace` green
- No behavior change (purely additive: error variants, `CanonicalTenant`/`TenantResolver`/`TenantEnforcementMode`, nothing wired)

**PR2** (TASK-006–010, incl. TASK-009B)
- Workspace still builds and tests green with `enforce_tenant`'s new `&mut`/fallible signature wired everywhere it's called (TASK-009B is part of this gate, not deferred to PR3)
- No operation is marked `#[tenant_scoped]` yet — behavior stays identical to PR1
- PR1's gate remains green

**PR3** (TASK-011–014)
- `#[tenant_scoped]` macro branch compiles and passes its trybuild/behavioral tests
- Automated detection test (`tenant_scoped_lint`) passes with zero violations workspace-wide
- Zero behavior change for existing unmarked operations (TASK-013 regression check)
- PR1/PR2 gates remain green

**PR4** (TASK-015–019)
- `issue_cross_tenant_permit` authorization-gating and destination-scoping tests green
- Every call-site from the migration inventory is migrated (TASK-018) — no caller left on the old signature
- PR1–PR3 gates remain green

**PR5** (TASK-020–024)
- Concurrency tests (TASK-020–023) and deprecated-accessor compatibility tests (TASK-024) green
- No production code changed in this PR beyond what the tests require
- PR1–PR4 gates remain green

**PR6** (TASK-025–034)
- Full FR/NFR acceptance suite green (TASK-025–032)
- `openspec/specs/service-sdk/spec.md` delta applied (TASK-033)
- Full workspace verification passes (TASK-034): `cargo build --workspace`, `cargo test --workspace`, and the two `rg` sweeps confirming no unmigrated legacy call sites remain
- PR1–PR5 gates remain green
- Tracker branch merges to `develop` only after this gate passes

---

## Implementation Notes (resolving two literal ambiguities in design.md's sketches)

Not open questions — design.md's ADs already answered the architecture; these are narrow "HOW" resolutions needed to write compilable tasks, in the same spirit as AD-007's "concrete mechanism is a tasks/implementation decision":

1. **`enforce_tenant` takes `&mut ServiceContext`, not `&ServiceContext`.** design.md's AD-009/Interfaces sketches now show `enforce_tenant(&self, ctx: &mut ServiceContext) -> Result<(), SecurityError>` (patched for consistency with AD-011's requirement that `enforce_tenant` be the sole writer of `ctx.resolved_tenant` via `set_resolved_tenant(&mut self, ..)` — an immutable reference cannot satisfy that without interior mutability, which no AD introduces for this field). Tasks below implement that `&mut` signature; the macro binds `ctx_param` as `mut` for BOTH the unmarked path (TASK-009B, Phase 2) and the `#[tenant_scoped]` path (TASK-012, Phase 3) — see the gap-fix note on TASK-009B for why the unmarked path's macro update cannot wait until Phase 3.
2. **`CanonicalTenant::Systemwide` is not auto-produced by `TenantResolver::resolve` for unmarked ops.** `resolve()` keeps the literal 2-arg signature from Interfaces/Contracts and always returns `Ok(Scoped(..))` or an `Err` (`TenantMismatch`/`MissingContext`) — it has no visibility into `#[tenant_scoped]`-ness. For an **unmarked** operation, the macro keeps discarding `enforce_tenant`'s `Result` (`let _ = rt.enforce_tenant(&mut ctx_param);`), so a resolution failure never surfaces and `ctx.resolved_tenant` simply stays `None` — the practical, observable equivalent of Data Flow's "op not tenant-scoped → Systemwide" branch (FR-001 Scenario 2: call proceeds, no tenant error occurs). `CanonicalTenant::Systemwide` remains a real, unit-tested, constructible variant (AD-002's formal expression of D1's tenant-less mode) for explicit/manual resolver use; the automatic macro path for unmarked ops does not need to construct it to satisfy the spec's observable contract.

---

## Phase 1: Errors + Canonical Type (Migration Step 1 — additive, nothing wired)

- [x] TASK-001 RED (`ego-security-sdk`): failing tests in `crates/security-sdk/src/error/mod.rs` — `SecurityError::TenantMismatch { expected, actual }` and `SecurityError::CrossTenantDenied { reason }` variants exist; `Display` for `TenantMismatch` does NOT contain either raw tenant identifier (AD-010 exposure boundary, NFR-003); `Debug` MAY contain them.
- [x] TASK-002 GREEN: add the two variants to `SecurityError` per AD-010; `#[error("tenant mismatch")]`-style redacted `Display` for `TenantMismatch` (no `{expected}`/`{actual}` interpolation), `#[error("cross-tenant access denied: {reason}")]` for `CrossTenantDenied` (mirrors existing `AuthorizationDenied` pattern). — Verify: `cargo test -p ego-security-sdk error` passes.
- [x] TASK-003 RED (`ego-service-sdk`): failing unit tests for a not-yet-existing `crates/service-sdk/src/runtime/tenant.rs` — `TenantResolver::resolve` 5-branch policy: (a) `Some(security)` + `principal.tenant_id = None` (gap fix — no hint may substitute for a missing Principal tenant claim) → `Err(MissingContext)`; (b) `Some(security)` + `principal.tenant_id = Some(_)` + hint absent/agreeing → `Ok(Scoped(principal.tenant_id))`; (c) `Some(security)` + `principal.tenant_id = Some(_)` + hint disagreeing → `Err(TenantMismatch{expected,actual})`; (d) `None` + `mode=AllowSystemInternal` + hint present → `Ok(Scoped(hint))`; (e) `None` + (`mode=AuthenticatedOnly` OR `AllowSystemInternal` with no hint) → `Err(MissingContext)`. Branch (a) MUST be checked before (b)/(c) — a present-but-conflicting hint must never be evaluated against an absent Principal tenant claim. Plus: `CanonicalTenant::Systemwide` and `CanonicalTenant::Scoped(..)` are constructible only within `crate::runtime` (mirrors `CrossTenantPermit`'s `pub(super)`-constructor pattern in `permit.rs`) and are `Clone`.
- [x] TASK-004 GREEN: create `crates/service-sdk/src/runtime/tenant.rs` — `CanonicalTenant` enum (AD-002), `TenantEnforcementMode` enum (AD-012), `TenantResolver` struct + `pub(crate) fn resolve(&self, security: Option<&SecurityContext>, supplied_tenant: Option<&str>) -> Result<CanonicalTenant, SecurityError>` implementing the D2 policy above (reuses `ego_domain::context::TenantId`, reads `SecurityContext::principal().tenant_id`). Wire `mod tenant;` into `crates/service-sdk/src/runtime/mod.rs`; re-export `CanonicalTenant`, `TenantEnforcementMode`, `TenantResolver`. — Verify: `cargo test -p ego-service-sdk runtime::tenant::` passes (TASK-003 tests green).
  - **Deviation (flagged, not silent):** `CanonicalTenant` is NOT the plain `pub enum { Scoped(TenantId), Systemwide }` literally sketched in design.md's Interfaces/Contracts. Rust enum variants always share the enum's own visibility (`error[E0449]` if you try to narrow a variant), so a public tuple variant carrying a public `TenantId` would be freely constructible by any external crate — defeating AD-003 the moment it compiled. `#[non_exhaustive]` per-variant was evaluated and rejected: it blocks *matching* the variant from other crates too, which would break the Phase 2 `ServiceContext::canonical_tenant()` read path (AD-011) before it's even built. Fix applied: `CanonicalTenant` wraps a private `Repr` enum; `scoped`/`systemwide` are `pub(super)` constructors (mirrors `CrossTenantPermit`'s pattern exactly); `tenant_id() -> Option<&TenantId>` and `is_systemwide() -> bool` are the public read API. Also added `ego-domain = { path = "../domain" }` to `crates/service-sdk/Cargo.toml` — `service-sdk` did not depend on `ego-domain` before this change, despite design.md assuming `ego_domain::context::TenantId` was already reusable there.
- [x] TASK-005 RED+GREEN: add `crates/service-sdk/tests/compile_fail/canonical_tenant_external_construct.rs` (+ `.stderr`, following the exact pattern of `tests/compile_fail/issue_cross_tenant_permit_external.rs`) proving external code cannot construct a `CanonicalTenant` directly; wire it into the existing trybuild harness alongside `cross_tenant_access_contract.rs`'s `t.compile_fail(...)` call (AD-003). — Verify: `cargo test -p ego-service-sdk --test cross_tenant_access_contract` passes with the new `.stderr` committed.

## Phase 2: Enforcement Path (Migration Step 2 — still inert, no op marked yet)

- [x] TASK-006 RED (`ego-service-sdk`): failing tests in `crates/service-sdk/src/context/mod.rs` — `ServiceContext::canonical_tenant()` returns `None` by default; `pub(crate) fn set_resolved_tenant(&mut self, t: CanonicalTenant)` sets it and is not reachable from outside the crate; `tenant_hint()`/`has_tenant_hint()` return the same value as the legacy `tenant_id`/`has_tenant` field today; `tenant_id()`/`has_tenant()` are `#[deprecated]` but still compile and return unchanged values (deprecation-window compatibility, see also Phase 6).
- [x] TASK-007 GREEN: add `resolved_tenant: Option<CanonicalTenant>` field (AD-011); add `pub fn canonical_tenant(&self) -> Option<&CanonicalTenant>`, `pub fn tenant_hint(&self) -> Option<&str>`, `pub fn has_tenant_hint(&self) -> bool`, `pub(crate) fn set_resolved_tenant(&mut self, t: CanonicalTenant)`; redocument the `pub tenant_id: Option<String>` field doc comment as a non-authoritative ingress hint; add `#[deprecated(note = "use canonical_tenant() for the enforced value or tenant_hint() for the raw ingress value")]` to `tenant_id()` and `has_tenant()` (`:264`, `:256`); update `Debug` impl to include `resolved_tenant`. — Verify: `cargo test -p ego-service-sdk context` passes (TASK-006 green); `cargo build -p ego-service-sdk` emits the expected deprecation warnings only at genuinely legacy call sites.
- [x] TASK-008 RED: failing tests in `runtime_builder.rs` — `RuntimeInner::enforce_tenant(&self, ctx: &mut ServiceContext) -> Result<(), SecurityError>` (see Implementation Note 1) returns `Ok(())` and sets `ctx.resolved_tenant` on a resolvable authenticated context; returns `Err` and leaves `ctx.resolved_tenant` unset on an unresolvable one; `RuntimeBuilder::with_tenant_enforcement_mode(TenantEnforcementMode::AllowSystemInternal)` changes resolution behavior; default mode is `AuthenticatedOnly`.
- [x] TASK-009 GREEN: implement `enforce_tenant` — build resolver inputs from `ctx.security()` and `ctx.tenant_hint()`, call `self.tenant_resolver.resolve(..)`, on `Ok(canonical)` call `ctx.set_resolved_tenant(canonical)` and return `Ok(())`, on `Err(e)` return `Err(e)` without mutating `ctx`. Store `tenant_resolver: TenantResolver` (built from `TenantEnforcementMode`) as a new `RuntimeInner` field, threaded through `new_with_logger` and `for_test()` (default `AuthenticatedOnly`). — Verify: `cargo test -p ego-service-sdk runtime_builder` passes.
- [x] TASK-009B GREEN (`ego-service-sdk-macros`, gap fix — MUST land in this PR, not Phase 3): update the existing `enforce_tenant_block` in `lib.rs:296-300` for the **unmarked path only** (no `#[tenant_scoped]` exists yet) — bind the generated `ctx_param` as `mut` and change the call from today's `rt.enforce_tenant(&#ctx_param);` to `let _ = rt.enforce_tenant(&mut #ctx_param);`, matching TASK-008/009's new `&mut self` signature. Without this task in the same PR as TASK-008/009, the workspace fails to compile the instant `enforce_tenant`'s signature changes, because the current codegen still passes an immutable reference. Phase 3 (TASK-011/012) then only ADDS the `#[tenant_scoped]` branch on top of this — it is not the first place the unmarked path's call site is touched. — Verify: `cargo build --workspace` compiles; `cargo test -p ego-service-sdk --test proxy_codegen` passes with zero behavior change for existing operations.
- [x] TASK-010 GREEN: add `RuntimeBuilder::with_tenant_enforcement_mode(mode: TenantEnforcementMode) -> Self` in `crates/service-sdk/src/runtime/builder.rs`, threading the mode into `RuntimeInner` construction; update the `builder.rs:18-25` docstring per AD-012 to disambiguate this enforcement-side `TenantEnforcementMode` from the persistence-side `single_tenant_mode`/`tenant_id` on `EntityRuntimeBuilder` (CORE-016) — no identifier or prose in this change reuses the bare phrase "tenant mode" for the enforcement concept. — Verify: `cargo test -p ego-service-sdk builder` passes; `cargo doc -p ego-service-sdk --no-deps` builds clean.

## Phase 3: Macro `#[tenant_scoped]` (Migration Step 3 — existing unmarked ops unchanged)

- [x] TASK-011 RED (`ego-service-sdk-macros`): failing trybuild/behavioral test — a test-only `#[service] trait` with one `#[operation] #[tenant_scoped] async fn` and one plain `#[operation] async fn`; expand and assert the generated code for the marked method contains `enforce_tenant(&mut ..)?` (fails fast) while the unmarked method's generated code discards the `Result` (`let _ = ..`).
- [x] TASK-012 GREEN: define `#[proc_macro_attribute] pub fn tenant_scoped` and reexport it, analogous to `operation`/`authorize` (`lib.rs:568-582`), so `#[tenant_scoped]` resolves as a real attribute macro. `SdkAttr` gains a `TenantScoped` variant recognized by `SdkAttr::detect` (`lib.rs:10-24`, matching `#[tenant_scoped]` via `attr.path().is_ident("tenant_scoped")`); in `expand_service_trait`, detect the attribute per method the same way `has_operation`/`authorize_attr` are detected (`lib.rs:111,116-119`). For a `#[tenant_scoped]` method: emit `rt.enforce_tenant(&mut #ctx_param)?;` in place of the unmarked path's discarding call (`ctx_param` is already bindable as `mut` as of TASK-009B), and add a const-closure `From<SecurityError>` assertion on the method's error type mirroring the `#[authorize]` pattern (`lib.rs:239-253`) so a spanned compile error fires if the return type can't absorb `SecurityError`. The unmarked path itself was already updated in TASK-009B (Phase 2, ships in the same PR as the signature change) — this task only adds the `#[tenant_scoped]` branch on top of it, it does not touch the unmarked path again. Strip `#[tenant_scoped]` in the `clean.attrs.retain` step (`lib.rs:323-326`) alongside `Operation`/`Authorize`.
- [x] TASK-013 Regression check: run `cargo test -p ego-service-sdk --test proxy_codegen` and the full macro test suite; confirm zero behavior change for existing unmarked operations (no `#[tenant_scoped]` exists anywhere yet in this phase).
- [x] TASK-014 (**Mandatory Seed 1 — automated `#[tenant_scoped]` detection, in scope for this change**) RED+GREEN: add `crates/service-sdk/tests/tenant_scoped_lint.rs` — a workspace-scanning `#[test]` (not a shell script) that resolves the workspace root via `env!("CARGO_MANIFEST_DIR")` ascending to the ancestor directory containing the workspace `Cargo.toml` (do NOT use a path relative to the crate under test — `cargo test`'s CWD is the crate root, so a literal `crates/*/src/` path resolves to a non-existent nested directory and silently scans zero files), then walks every `.rs` file under `<workspace_root>/crates/*/src/`, finds each `#[operation]`-annotated method, and fails with a listed file:line if the method body references a tenant-related identifier (`tenant_hint`, `canonical_tenant`, `TenantId`, `ExecutionContext` tenant accessors) without a `#[tenant_scoped]` attribute on the same method. **Known limitation, documented not hidden**: this identifier-name heuristic only catches direct references — an operation that touches tenant-scoped data through an indirect path (e.g. a repository method that filters by tenant internally) produces a false negative. This is an accepted best-effort gap given AD-007's opt-in model; the definitive fix is the secure-by-default flip already tracked as a design.md follow-up, not a stronger heuristic here. **Mechanism choice and rationale**: implemented as a `cargo test --workspace` participant (this project's Strict TDD test command and the exact gate already enforced in `.gitlab-ci.yml`'s `test` stage) rather than a new, unenforced `scripts/detect-*.sh` (the existing scripts of that shape are not wired into any CI stage — see `.gitlab-ci.yml`). This makes the detection genuinely automated and CI-gated on day one, per AD-007's "not a manual review checklist, not an indefinitely deferred follow-up." — Verify: `cargo test -p ego-service-sdk tenant_scoped_lint` passes with zero violations at each subsequent phase, AND fails when pointed at a deliberately-unmarked tenant-touching test fixture (proving it doesn't silently scan zero files).

## Phase 4: Cross-Tenant Issuance (Migration Step 4, AD-008)

- [x] TASK-015 RED (`ego-service-sdk`): failing tests — `issue_cross_tenant_permit` becomes `pub(crate) async fn issue_cross_tenant_permit(&self, ctx: &ServiceContext, destination: TenantId) -> Result<CrossTenantPermit, SecurityError>`; a `Deny` decision from `AuthorizationProvider` yields `Err(SecurityError::CrossTenantDenied{..})`; resource/action authorization alone (no cross-tenant capability) does not yield a permit (FR-005); an `Allow` decision yields `Ok(CrossTenantPermit{destination, issued_to})` (FR-006).
- [x] TASK-016 GREEN: in `crates/service-sdk/src/runtime/permit.rs`, change `CrossTenantPermit` to `#[derive(Debug, Clone)] pub struct CrossTenantPermit { destination: TenantId, issued_to: SubjectId }` — drop `Copy`, keep `Clone`. Update the type-level doc's "Copy + Clone design decision" section to reflect the removal (no longer a zero-size `Copy` witness).
- [x] TASK-017 GREEN: implement the new `issue_cross_tenant_permit` in `runtime_builder.rs` — resolve `Principal` from `ctx.security()`, build `AccessRequest::new(Resource{ kind: Cow::Borrowed("tenant"), id: Some(destination.to_string()) }, Action(Cow::Borrowed("cross-tenant-access")))`, call `ego_security_sdk::authorization::authorize_in_context(ctx.security(), resource, action, provider)`; map `Err(AuthorizationDenied{reason})` → `Err(CrossTenantDenied{reason})`, map other provider errors through unchanged, on success construct `CrossTenantPermit{destination, issued_to: principal.subject_id.clone()}` (`subject_id` is a public field, not a method). Require an `AuthorizationProvider` to be configured (`CapabilityNotEnabled` if none). — Verify: `cargo test -p ego-service-sdk runtime_builder` and `cargo test -p ego-service-sdk permit` pass (TASK-015 green).
  - **Deviation (flagged):** `TenantId` has no `Display` impl (only `as_str() -> &str`), so `destination.to_string()` as literally written here does not compile. Used `destination.as_str().to_string()` instead — same resulting `String`, no behavior change.
- [x] TASK-018 (**Mandatory Seed 2 — complete call-site migration inventory**) GREEN: migrate every real caller found by codebase search (exhaustive — no others exist, re-verified against current line numbers which shifted from Phases 2-3):
  - `crates/service-sdk/src/runtime/runtime_builder.rs` (test `issue_cross_tenant_permit` smoke assertion, was `:465` now `:499` pre-edit) → `.await`, supply a test `ServiceContext` with security + a mock `AuthorizationProvider` returning `Allow`, and a `destination` `TenantId`.
  - `crates/service-sdk/src/context/mod.rs` (`with_cross_tenant_access_sets_flag`, `clone_preserves_cross_tenant_flag`; was `:364`/`:373`, now `:409`/`:418` pre-edit) → same `.await` + ctx/destination/provider wiring; added `RuntimeInner::for_test_with_authz(provider) -> Self` sibling test helper (mirrors `for_test_with_mode`) for these two tests plus the new TASK-017/019 tests.
  - `crates/service-sdk/tests/compile_fail/issue_cross_tenant_permit_external.rs:8` → rewrote to `rt.inner().issue_cross_tenant_permit(&ctx, destination)` (no `.await` needed — `E0624` fires at the call-expression's method-resolution stage regardless, still fails to compile on visibility, not on the new params); regenerated `.stderr` via `TRYBUILD=overwrite cargo test -p ego-service-sdk --test cross_tenant_access_contract`, confirmed the failure is still `E0624: method is private` targeting this call.
- [x] TASK-019 GREEN: destination-scope the consumer side — in `crates/service-sdk/src/context/mod.rs`, changed `with_cross_tenant_access(mut self, permit: &CrossTenantPermit) -> Self` to record `permit.destination()` via `allow_cross_tenant: Option<TenantId>` (replaced the bare `bool`) instead of discarding `_permit`; added `pub fn is_cross_tenant_allowed_for(&self, destination: &TenantId) -> bool` checking equality against the stored destination; kept `is_cross_tenant_allowed() -> bool` as `.is_some()` for source compatibility with existing tests (`with_cross_tenant_access_sets_flag`, `clone_preserves_cross_tenant_flag`). This is the wiring that makes AD-08's "permit authorizing tenant-b cannot be reused to reach tenant-c" observable — required for Phase 5's TASK-023. Added `CrossTenantPermit::destination()`/`issued_to()` `pub(crate)` accessors (permit fields stayed private per AD-008's sketch; `context/mod.rs` reads them cross-module via the accessor, not raw field access). — Verify: `cargo test -p ego-service-sdk context` passes.

## Phase 5: Concurrency Tests (Mandatory Seed 3) + Deprecation-Window Compatibility (Mandatory Seed 4)

- [ ] TASK-020 RED+GREEN: new `crates/service-sdk/tests/tenant_enforcement_concurrency.rs` — **two concurrent operations carrying different tenant hints**: spawn two `#[tokio::test(flavor = "multi_thread")]` tasks, each calling `enforce_tenant` through its own `ServiceContext` with a different authenticated tenant; assert each resolves its own tenant with no cross-contamination (`CanonicalTenant` is a small owned value per AD-004/AD-005, not shared state).
- [ ] TASK-021 RED+GREEN: **retried calls under tenant resolution** — simulate a caller retrying the same operation after a transient failure (e.g. provider hiccup) with the same `Principal`/hint; assert resolution is idempotent — the retried call resolves to the identical `CanonicalTenant` value, no state leaks between the failed and retried attempt.
- [ ] TASK-022 RED+GREEN: **`ServiceContext` clone behavior under tenant resolution** — clone a `ServiceContext` before `enforce_tenant` runs (assert `canonical_tenant() == None` on both the original and the clone) and after it runs (assert both the original and a fresh `.clone()` carry the same resolved `CanonicalTenant`, and neither can independently diverge — no public mutator exists per AD-004).
- [ ] TASK-023 RED+GREEN: **`CrossTenantPermit` proven non-reusable for a destination other than the one it was issued for** — issue a permit for `tenant-b` via `issue_cross_tenant_permit`, call `ctx.with_cross_tenant_access(&permit)`, then assert `ctx.is_cross_tenant_allowed_for(&tenant_b) == true` and `ctx.is_cross_tenant_allowed_for(&tenant_c) == false` (uses TASK-019's wiring).
- [ ] TASK-024 (**Mandatory Seed 4 — compatibility/deprecation-window tests**) RED+GREEN: new test module (or extend `context/mod.rs`'s existing `#[cfg(test)] mod tests`) proving the deprecated `tenant_id()`/`has_tenant()` accessors **keep functioning correctly**, not merely marked `#[deprecated]`: `#[allow(deprecated)] fn deprecated_accessors_still_return_ingress_hint_during_migration_window()` asserts `ctx.with_tenant_id("t").tenant_id() == Some("t")` and `.has_tenant() == true`, and that these values are identical to the new `tenant_hint()`/`has_tenant_hint()` accessors on the same context — proving the legacy names are a live, correct alias, not a silently broken relic.

## Phase 6: Adopt Markers + Full FR/NFR Acceptance Suite + Spec Delta (Migration Step 5)

- [ ] TASK-025 RED: new `crates/service-sdk/tests/tenant_enforcement_contract.rs` — define a test-only `#[service] trait TenantContractService` with one `#[operation] #[tenant_scoped] async fn scoped_op(..)` and one plain `#[operation] async fn unscoped_op(..)` (adopting `#[tenant_scoped]` on a genuinely tenant-sensitive test operation, per Migration Step 5). Write failing tests for FR-001: tenant-scoped op invoked with no resolvable tenant fails closed with an explicit tenant error before the body runs; the unscoped op invoked with no tenant proceeds normally with no tenant error (NFR-001).
- [ ] TASK-026 GREEN: implement the test service/handlers so TASK-025 passes; this is the first and only marker adoption in this change (design.md's Non-Goals keep everything else — real application operations — out of scope).
- [ ] TASK-027 RED+GREEN: FR-002/FR-003/FR-004 scenarios in the same file — authenticated derivation succeeds without manual tenant assignment; caller-supplied tenant conflicting with `Principal.tenant_id` is a hard `TenantMismatch` (neither value silently wins); an authenticated Principal with no tenant claim fails with `MissingContext` regardless of whether a caller-supplied hint is present (gap-fix branch a — the hint is never trusted as a substitute); internal mode accepts a caller-supplied tenant only when `TenantEnforcementMode::AllowSystemInternal` is configured, and rejects it (routes to FR-004) when not; a call that is neither authenticated nor internal-permitted fails with `MissingContext` before the operation body executes.
- [ ] TASK-028 RED+GREEN: FR-005/FR-006/NFR-002 scenarios — a `Principal` authorized for the resource/action but without the cross-tenant capability is denied a permit (two sub-cases: no authorization at all, and resource/action-only authorization); a `Principal` with the cross-tenant capability obtains a permit and successfully executes a cross-tenant operation end to end (dedicated positive-path coverage, not only rejection).
- [ ] TASK-029 RED+GREEN: FR-007 structural test — extend TASK-014's scan test to additionally assert that `crates/service-sdk/src/runtime/tenant.rs` and the enforcement path in `runtime_builder.rs` reference no `axum`, `tonic`, HTTP header, or gRPC metadata identifier — the runtime layer carries no transport dependency.
- [ ] TASK-030 RED+GREEN: FR-008 scenario — construct a request where a JWT-derived `Principal.tenant_id`, a pre-existing `ServiceContext` hint, and the domain `ExecutionContext`/`TenantId` could, before this change, disagree; assert that after `enforce_tenant` runs, exactly one tenant value (`ctx.canonical_tenant()`) is authoritative and every downstream tenant-aware check (enforcement, cross-tenant authorization) reads that same value.
- [ ] TASK-031 RED+GREEN: FR-009/FR-010/FR-014 immutability triad — enforcement failure aborts before the operation body executes and enforcement success allows it to run exactly as without enforcement (FR-009); an attempt to mutate the service-visible tenant via `with_tenant_id` on an already-resolved, authenticated `ServiceContext` does not change what enforcement reads (FR-010); a downstream mutation attempt after resolution does not affect an operation already in progress — all subsequent enforcement decisions observe the original canonical tenant (FR-014).
- [ ] TASK-032 RED+GREEN: FR-011/FR-012/NFR-003 — a canonical tenant is available at the start of execution on the authenticated path without manual per-call assignment (FR-011); each of `TenantMismatch`, `MissingContext`, and `CrossTenantDenied` is independently reachable in code and distinguishable via `match` — not merely documented (FR-012, NFR-003 — assert on the specific error variant, not just "the call failed").
- [ ] TASK-033 GREEN (doc delta, no code): update `openspec/specs/service-sdk/spec.md:76` (the enforcement call contract) and INV-003 (`:427`, "Tenant Enforcement Preserved") to describe the now-true, fallible-check behavior implemented in Phases 1-6 (FR-013) — no aspirational or outdated claim remains.
- [ ] TASK-034 Full workspace verification: run `cargo build --workspace` and `cargo test --workspace`; confirm every TASK above is green. Run `rg "\.tenant_id\(\)|\.has_tenant\(\)" crates/service-sdk` and confirm every remaining match is either the deprecated accessor definitions themselves (`context/mod.rs:256,264`) or a call site explicitly exercising deprecation-window compatibility (TASK-024) — no other production call path relies on the legacy names. Run `rg "issue_cross_tenant_permit\(" crates/service-sdk` and confirm every match is `.await`ed with `(ctx, destination)` arguments.

---

## Traceability

| FR/NFR/AD | Covered by |
|---|---|
| FR-001 | TASK-003/004 (Systemwide/Scoped branches), TASK-012 (macro gating), TASK-025/026 (acceptance scenarios) |
| FR-002 | TASK-003/004 (resolve branches a/b/c, incl. gap-fix branch a for a tenant-less authenticated Principal), TASK-027 |
| FR-003 | TASK-003/004 (resolve branch d), TASK-010, TASK-027 |
| FR-004 | TASK-003/004 (resolve branches a, e), TASK-025, TASK-027 |
| FR-005 | TASK-015/016/017, TASK-028 |
| FR-006 | TASK-015/016/017, TASK-028 (NFR-002 positive path) |
| FR-007 | TASK-004 (transport-neutral inputs), TASK-029 |
| FR-008 | TASK-004 (AD-002/003), TASK-030 |
| FR-009 | TASK-008/009, TASK-031 |
| FR-010 | TASK-007 (no public setter), TASK-019, TASK-031 |
| FR-011 | TASK-007/009, TASK-032 |
| FR-012 | TASK-001/002, TASK-032 |
| FR-013 | TASK-033 |
| FR-014 | TASK-007 (immutable field, no public mutator), TASK-022, TASK-031 |
| NFR-001 | TASK-025, TASK-027 (rejection-path dedicated coverage beyond field pass-through tests) |
| NFR-002 | TASK-028 (positive cross-tenant path) |
| NFR-003 | TASK-001, TASK-032 (assert on distinguishable error variant) |
| AD-001 (`TenantResolver` seam) | TASK-004 |
| AD-002 (`CanonicalTenant` type) | TASK-004 |
| AD-003 (only resolver mints it) | TASK-004, TASK-005 |
| AD-004 (immutable, no setters) | TASK-004, TASK-022 |
| AD-005 (operation-scoped lifecycle) | TASK-004, TASK-009 |
| AD-006 (transient coexistence precedence) | TASK-030 |
| AD-007 (`#[tenant_scoped]` opt-in + mandatory detection) | TASK-011/012/013, TASK-014 |
| AD-008 (authorization-gated, destination-scoped permit) | TASK-015/016/017/018/019, TASK-023 |
| AD-009 (fallible enforcement, macro `?`) | TASK-008/009, TASK-012 |
| AD-010 (error taxonomy, redaction boundary) | TASK-001/002 |
| AD-011 (`ServiceContext` hint vs canonical split) | TASK-006/007, TASK-024 |
| AD-012 (`TenantEnforcementMode` naming disambiguation) | TASK-010 |
| AD-013 (transport-independent resolution) | TASK-029 |
| Mandatory Seed 1 (automated detection) | TASK-014 |
| Mandatory Seed 2 (call-site inventory) | TASK-018 |
| Mandatory Seed 3 (concurrency tests) | TASK-020/021/022/023 |
| Mandatory Seed 4 (compat tests) | TASK-024 |

## Call-Site Inventory (Mandatory Seed 2 — exhaustive, confirmed by codebase search)

`ServiceContext::has_tenant()` — **zero external callers found**; only the definition (`context/mod.rs:256`) exists today. No migration needed beyond deprecation (TASK-007).

`ServiceContext::tenant_id()` — all callers are test-only, all in `crates/service-sdk/tests/`: `context_cross_service.rs` (`:18,32`), `context_explicit_propagation.rs` (`:20,31,34,64,65,69,70`), `smoke.rs` (`:189,198,207,208`), `context_propagation.rs` (`:19,30`). None require migration for this change to compile (the accessor keeps working, deprecated) — TASK-024 proves they still function; a future scoped follow-up (AD-011) removes them.

`issue_cross_tenant_permit(...)` callers — exactly four, all test-only: `runtime_builder.rs:465`, `context/mod.rs:364`, `context/mod.rs:373`, `tests/compile_fail/issue_cross_tenant_permit_external.rs:8`. All four are migrated in TASK-018.
