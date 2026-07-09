# Tasks: CORE-024 — Validate `Principal.tenant_id` once at construction

Single atomic delivery per design AD-4 — the workspace does not compile in a
half-migrated state, so these tasks land together in one PR, in the order
below (each step assumes the previous one is already applied in the working
tree; only the final task requires a clean `cargo build`/`cargo test`).

## Phase 1 — security-sdk: field type + builder

### TASK-001: Change `Principal.tenant_id` to `Option<TenantId>` — [x] DONE
- File: `crates/security-sdk/src/principal/principal.rs`
- Add `use ego_domain::context::TenantId;`
- Field `:63`: `Option<String>` → `Option<TenantId>`
- Builder `:83-86` (`with_tenant_id`): signature `impl Into<String>` → `TenantId`; body unchanged (`self.tenant_id = Some(tenant_id); self`)
- Acceptance: crate does not yet compile (expected — callers not updated), but the field/builder diff matches design.md §2 exactly.

### TASK-002: Update security-sdk's own tests for the new type — [x] DONE
- File: `crates/security-sdk/src/principal/principal.rs` (tests at `:140-152`, `:212-218`)
- `.with_tenant_id("acme")` → `.with_tenant_id(TenantId::new("acme").unwrap())`
- Assertions on `tenant_id` compare against `Some(TenantId::new("acme").unwrap())` (or `.as_ref().map(TenantId::as_str)` where a string comparison reads better)
- Acceptance: `cargo test -p ego-security-sdk` compiles (crate-local; workspace still broken until Phase 4 completes — that's expected under AD-4).

## Phase 2 — security-jwt: validate at mapping time

### TASK-003: Validate tenant claim in `DefaultPrincipalMapper::map()` — [x] DONE
- File: `crates/security-jwt/src/principal_mapper.rs`
- Add `use ego_domain::context::TenantId;`
- Lines `:128-130`: wrap the raw claim in `TenantId::new(tid.clone()).map_err(|_| AuthenticationError::InvalidToken("invalid tenant claim".into()))?` before calling `with_tenant_id(tenant)` — exact diff per design.md §3 and AD-1 (reuse `InvalidToken`, no new error variant)
- Acceptance: an invalid (whitespace-only) tenant claim causes `map()` to return `Err(AuthenticationError::InvalidToken(_))` before any `Principal` is constructed.

### TASK-004: Update security-jwt tests for the new type and add the login-time failure test — [x] DONE
- Files:
  - `crates/security-jwt/src/principal_mapper.rs:347` — `.as_deref()` → `.as_ref().map(TenantId::as_str)`
  - `crates/security-jwt/tests/oidc_integration.rs:503` — same treatment
  - `crates/security-jwt/src/validation.rs:461` — no change needed (`Option<TenantId>` still compares equal to `None`)
  - `crates/security-jwt/src/validation.rs:478` — `.as_deref()` → `.as_ref().map(TenantId::as_str)` (this one does not compile otherwise — `TenantId` has no `Deref<Target=str>`)
- New test: `principal_mapper::tests::maps_invalid_tenant_claim_fails` — whitespace-only `"tid"` claim → asserts `Err(AuthenticationError::InvalidToken(_))`, no `Principal` returned (per spec's "Invalid tenant claim fails at mapping time, not later" scenario)
- Acceptance: `cargo test -p ego-security-jwt` compiles and passes (once Phase 1+2 code changes are both in place).

## Phase 3 — testkit: validate in `build()`

### TASK-005: `PrincipalBuilder::build()` validates the tenant fixture — [x] DONE
- File: `crates/testkit/src/identity.rs`
- Add `use ego_domain::context::TenantId;`
- Keep field `tenant: Option<String>` (`:18`) and setter `tenant(impl Into<String>)` (`:49-52`) unchanged
- In `build()` (`:75-77`): validate via `TenantId::new(tenant).expect("PrincipalBuilder tenant must not be empty or whitespace-only")`, mirroring the existing `SubjectId::new(...).expect(...)` at `:72-73`, then pass the resulting `TenantId` to `with_tenant_id`
- Acceptance: `PrincipalBuilder::new().tenant("").build()` panics with a message identifying the invalid tenant.

### TASK-006: Update testkit tests — [x] DONE
- File: `crates/testkit/src/identity.rs` (test at `:148` asserting `tenant_id.as_deref()`)
- `.as_deref()` → `.as_ref().map(TenantId::as_str)`
- New tests: `identity::tests::empty_tenant_override_panics`, `identity::tests::whitespace_only_tenant_override_panics`
- Acceptance: `cargo test -p ego-testkit` compiles and passes.

## Phase 4 — service-sdk: `resolve()` stops re-validating

### TASK-007: Rewrite `TenantResolver::resolve()` and delete `validated()` — [x] DONE
- File: `crates/service-sdk/src/runtime/tenant.rs`
- Delete the private `validated()` helper (`:155-160`) entirely
- Replace `resolve()` (`:118-153`) with the exact body shown in design.md §4: Principal-derived branches (`security.principal().tenant_id.as_ref()`) perform zero validation, just clone; the `AllowSystemInternal` system/internal hint branch inlines `TenantId::new(hint).map(CanonicalTenant::scoped).map_err(|_| SecurityError::MissingContext)` — do **not** leave a `Self::validated(hint)` call anywhere in the final code (this was the exact defect caught and fixed in design review)
- Comparison fix: `hint == principal_tenant` → `hint == principal_tenant.as_str()`; `TenantMismatch.expected` → `principal_tenant.as_str().to_string()`
- Acceptance: no reference to `Self::validated` or `validated(` remains in `tenant.rs`; `cargo build -p ego-service-sdk` compiles.

### TASK-008: Update the 4 direct-field-assignment test sites — [x] DONE
- `crates/service-sdk/src/runtime/tenant.rs:174` (test helper `principal_with_tenant`) — `tenant.map(|t| t.to_string())` → `tenant.map(|t| TenantId::new(t).unwrap())`
- `crates/service-sdk/tests/tenant_scoped_codegen.rs:145` — `.to_string()` → `TenantId::new("...").unwrap()`
- `crates/service-sdk/tests/common/mod.rs:22` — same
- `crates/service-sdk/src/runtime/runtime_builder.rs:660` — same
- No field-visibility change (stays `pub` per proposal Decision 4 / design — the type itself is the safety guarantee).
- Acceptance: all 4 sites compile against `Option<TenantId>`.

### TASK-009: Update/add service-sdk `resolve()` tests — [x] DONE
- Existing: `resolve_authenticated_hint_absent_resolves_to_principal_tenant`, `resolve_authenticated_hint_agrees_resolves_to_principal_tenant`, `resolve_authenticated_hint_disagrees_is_tenant_mismatch`, `resolve_unauthenticated_allow_system_internal_with_hint_resolves_to_hint` — update fixture construction to build a `Principal` with `Option<TenantId>` directly, assert identical behavior to pre-change.
- The "no re-validation on the Principal path" property (spec's structural scenario) is **not** a runtime-testable assertion once the field is type-guaranteed valid — do not attempt to fabricate a test for it. Satisfy it by code inspection during PR review instead (note this in the PR description).
- Acceptance: `cargo test -p ego-service-sdk` compiles and passes.

## Phase 5 — workspace-wide verification

### TASK-010: Full workspace build and test — [x] DONE
- Run `cargo build --workspace` — must succeed with zero errors across all 4 touched crates plus any downstream crate that transitively depends on them.
- Run `cargo test --workspace` — must pass, including all updated/new tests from Phases 1-4.
- Optional scratch verification (per verify skill, not merged): a throwaway `crates/security-jwt/examples/verify_tenant_validation.rs` exercising (a) a valid tenant claim → `Ok` with `Principal.tenant_id = Some(expected)`, and (b) a whitespace-only claim → `Err(AuthenticationError::InvalidToken(_))`. Delete the example file before opening the PR — it is a manual check, not a permanent artifact.
- Acceptance: green `cargo build --workspace` and `cargo test --workspace`; no scratch example file left in the diff.

---

## Review Workload Forecast

Estimated changed lines by task:

| Phase | Files touched | Est. lines changed |
|---|---|---|
| 1 (security-sdk) | 1 file, field + builder + 2 test blocks | ~15 |
| 2 (security-jwt) | 2 files, 1 validation add + 4 test-site fixups + 1 new test | ~35 |
| 3 (testkit) | 1 file, build() validation + 1 test-site fixup + 2 new tests | ~25 |
| 4 (service-sdk) | 3 files, resolve() rewrite + validated() deletion + 4 test-site fixups + 4 test updates | ~70 |
| 5 (verification) | 0 merged files (scratch example deleted before PR) | 0 |
| **Total** | **7 files** | **~145** |

- **Chained PRs recommended: No** — AD-4 forces a single atomic PR (no safe half-migrated intermediate state); the estimate confirms this is small enough (~145 lines) that chaining would add process overhead without a real review-size benefit.
- **400-line budget risk: Low** — well under the 400-line threshold.
- **Decision needed before apply: No** — proceed directly to `sdd-apply` as a single-PR delivery.
