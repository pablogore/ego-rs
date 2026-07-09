# Verify Report: CORE-024 — Validate `Principal.tenant_id` once at construction

**Verdict: PASS — 0 CRITICAL, 0 WARNING, 2 SUGGESTION (informational, non-blocking)**

## Build/Test evidence

- `cargo build --workspace`: clean, 0 errors.
- `cargo test --workspace`: all green, 0 failures (service-sdk 195 passed, security-jwt lib 117 passed, domain 196 passed, etc. — verified with `rg "FAILED|error\["` on full output, zero matches).

## FR/requirement verification (file:line impl + test)

| Spec | Requirement | Impl | Test |
|---|---|---|---|
| security-sdk FR-001 | `tenant_id: Option<TenantId>`, `with_tenant_id(TenantId)` infallible | `crates/security-sdk/src/principal/principal.rs:65,88-91` | `principal.rs:149-151` (`with_tenant_id_sets_field`), `:157-159` (`with_tenant_id_overwrites`), `:224-226` (`subject_id_and_attributes`) |
| security-jwt (added) | `map()` validates tenant claim once, fails at mapping time | `crates/security-jwt/src/principal_mapper.rs:129-132` (`TenantId::new(tid).map_err(...InvalidToken...)?`) | `principal_mapper.rs:349-352` (valid case), `:355-363` (`maps_invalid_tenant_claim_fails`, new) |
| testkit (added) | `PrincipalBuilder::tenant()` stays ergonomic, `build()` validates + panics | `crates/testkit/src/identity.rs:77-79` | `identity.rs:151` (valid), `:169-179` (2 new panic tests) |
| service-sdk (added) | `resolve()` clones Principal tenant without re-validation; hint path still validated | `crates/service-sdk/src/runtime/tenant.rs:124-149` (clone-only), `:151-157` (branch (d) inline `TenantId::new`) | `tenant.rs` resolve tests (4 existing, fixture-updated) |

`validated()` helper: confirmed fully deleted — `rg "validated\("` in `tenant.rs` returns only a doc-comment reference, no call sites.

## Deviations from tasks.md/design.md (confirmed intentional, not defects)

- `crates/domain/src/context.rs`: `id_type!` macro gained `#[serde(try_from = "String")]` + `TryFrom<String>` impl, closing a Deserialize-validation-bypass bug; `id_type!` exported via `pub(crate) use id_type;`. Verified present and tested for all 6 types (`context.rs:196-313`, one deserialize-valid + two reject tests per type... TenantId gets 3 dedicated tests at `:239-257`).
- `crates/domain/src/idempotency.rs`: `IdempotencyKey` refactored onto the shared `id_type!` macro, fixing the identical bypass bug. Full test module added (6 tests, `:53-102`). **Scope note**: this file sits outside the 4-crate blast radius named in proposal.md/design.md — correctly flagged here as an adjacent-but-related fix, not a defect.
- `service-sdk/src/runtime/tenant.rs::resolve()`: final shape is early-return with hoisted `expected` binding, not design.md's original `match`-on-`supplied_tenant` sketch. Verified current behavior is equivalent: hint absent/blank/agreeing → clone; hint disagrees → `TenantMismatch`; comparison is against `principal_tenant.as_str()` as required.

## Tasks.md (10/10 spot-checked against code, not just checkmarks)

All 10 tasks' acceptance criteria hold in the current diff: field/builder type change (TASK-001/002), JWT validation + new failure test (TASK-003/004), testkit build()-time validation + 2 new panic tests (TASK-005/006), `resolve()` rewrite + `validated()` deletion (TASK-007), 4 direct-assignment sites updated (TASK-008), resolve() tests updated (TASK-009), clean workspace build/test with no leftover scratch example file (TASK-010 — confirmed `crates/security-jwt/examples/verify_tenant_validation.rs` does not exist).

## Non-goals honored

1. No `Arc<str>` migration — `TenantId` still wraps `String`; `git diff --stat` on `context.rs` shows no such change; `rg "Arc<str>"` in `context.rs` is empty.
2. No change to `ServiceContext.tenant_id`/`tenant_hint()` — `git diff HEAD -- crates/service-sdk/src/context/mod.rs` is empty (untouched). Confirmed all other `.tenant_id`/`.tenant_id()` hits across the workspace belong to unrelated types (`ServiceContext`, `ExecutionContext`, `persistent-entity` entity-id tenant) — not in scope.

## "Done looks like" bullets (proposal) — all satisfied

- `Principal.tenant_id` is `Option<TenantId>`, no raw string survives past construction — confirmed.
- `resolve()` performs zero validation on the Principal path — confirmed by code inspection (only `.clone()`).
- Invalid tenant claims fail at login with `AuthenticationError::InvalidToken` — confirmed + tested.
- All existing tenant-enforcement tests pass — confirmed (`cargo test --workspace` green).

## SUGGESTION (non-blocking)

1. Consider a one-line mention in the eventual archive/PR description that `idempotency.rs` picked up the same Deserialize-bypass fix as a drive-by, since it's outside the named blast radius (transparency for reviewers scanning the diff stat).
2. `service-sdk/src/runtime/tenant.rs` resolve()'s current early-return shape differs from design.md's code sketch — no action needed, but future readers comparing design.md to code verbatim should know the design doc shows an earlier draft, not the final shape.

## Next recommended step

`sdd-archive` — no CRITICAL or WARNING issues found; implementation matches spec/design/tasks with all intentional deltas confirmed correct.
