# Archive Report: CORE-024 — Validate `Principal.tenant_id` once at construction

**Date Archived**: 2026-07-09  
**Change Status**: COMPLETE  
**Verification Status**: PASS (0 CRITICAL, 0 WARNING, 2 informational SUGGESTIONs)

---

## What Changed

This change relocated tenant ID validation from per-request time (inside `TenantResolver::resolve()`) to construction time (at JWT principal mapping), eliminating per-call re-validation overhead while moving validation failure to the authentication boundary where it belongs.

**Scope**: Four interdependent crates (`security-jwt`, `security-sdk`, `service-sdk`, `testkit`) updated atomically in a single PR — the workspace does not compile in a half-migrated state.

**Key Implementation**: Changed `Principal.tenant_id` from `Option<String>` to `Option<TenantId>`, validated once at `DefaultPrincipalMapper::map()` time.

---

## Specs Merged

| Spec Domain | Action | Details |
|---|---|---|
| `security-jwt` | Created | New spec with one ADDED requirement: `DefaultPrincipalMapper` validates tenant claim once at mapping time |
| `security-sdk` | Modified | FR-001 updated: `Principal.tenant_id` is now `Option<TenantId>` (pre-validated); `with_tenant_id()` signature changed from `impl Into<String>` to `TenantId` |
| `service-sdk` | Added | New requirement: `TenantResolver::resolve()` does NOT re-validate Principal-derived tenant claims; `validated()` helper deleted |
| `testkit` | Added | New requirement: `PrincipalBuilder::build()` validates tenant string via `TenantId::new()`, panics on invalid fixture at test setup |

---

## Implementation Summary

**All 10 tasks completed** ✅

- **Phase 1** (security-sdk): Field type change + builder signature update + tests — ✅
- **Phase 2** (security-jwt): Validation added to `DefaultPrincipalMapper::map()`, invalid-claim-at-login test added — ✅
- **Phase 3** (testkit): `build()` validates tenant via expect, panic tests added — ✅
- **Phase 4** (service-sdk): `resolve()` rewritten to skip validation on Principal path, `validated()` deleted, 4 test sites updated — ✅
- **Phase 5** (workspace): `cargo build --workspace` + `cargo test --workspace` green — ✅

**Code changes**: ~145 lines across 7 files.

---

## Verification Outcome

**Verdict: PASS** — 0 CRITICAL, 0 WARNING

All FR/requirement verification passed:
- `security-jwt` tenant validation at mapping time ✅
- `security-sdk` field type and builder signature ✅
- `service-sdk` zero re-validation on Principal path ✅
- `testkit` fail-fast validation at fixture build ✅

**Build Status**: `cargo build --workspace` clean, `cargo test --workspace` all green (195 service-sdk tests, 117 security-jwt tests, 196 domain tests passed).

---

## Notable Implementation Details

### Deserialize Validation Bypass Fix (Bonus)

During implementation, code review discovered and fixed a Deserialize-validation-bypass vulnerability in the `id_type!` macro (used by `TenantId`, `EntityId`, `CorrelationId`, `CausationId`, `RequestId`, `AggregateId` — 6 types total). Added `#[serde(try_from = "String")]` + `TryFrom<String>` impl to all 6 types, with comprehensive test coverage per type. Also refactored `IdempotencyKey` to use the shared `id_type!` macro (previously had its own inline impl with the same bypass vulnerability), fixing the adjacent bug.

**Where Found**: Code review during verification phase (not in original spec scope, but caught and verified during implementation).

**Tests Added**: 3 tests per id_type (`context.rs:196-313`) + 6 tests in `idempotency.rs:53-102` covering deserialize-valid and reject cases.

### Resolve() Implementation Shape Difference

Design doc showed a `match`-on-`supplied_tenant` sketch; final implementation uses early-return with hoisted `expected` binding. Behavior is equivalent (hint absent/blank/agreeing → clone; disagree → `TenantMismatch`). Verified at code inspection during verify phase.

---

## Non-Goals Honored

1. **No `Arc<str>` migration** — `TenantId` still wraps `String`; clone remains an allocation. Future work.
2. **No `ServiceContext.tenant_id` changes** — `tenant_hint()` untouched; it is a raw ingress hint per AD-011.
3. **No validation rule changes** — non-empty-after-trim preserved exactly; only relocation, not redesign.
4. **No `Principal` field-visibility tightening** — stays `pub`; type change is the safety guarantee.

---

## Rollback Plan

Low-risk, simple rollback: `git revert <commit>` fully restores prior behavior. No data migration in either direction (nothing persisted or transmitted in a new shape). Workspace does not compile in half-migrated state (atomic change), so revert is clean.

---

## SDD Cycle Closure

**Proposal**: ✅ Defined scope, approach, rollback  
**Spec**: ✅ All 4 domain specs merged (created security-jwt, updated security-sdk/service-sdk/testkit)  
**Design**: ✅ Exact signatures, error variants, migration order specified  
**Tasks**: ✅ All 10 implementation tasks completed and verified  
**Verification**: ✅ All requirements met, all tests green, bonus fixes included and verified  
**Archive**: ✅ Change folder moved to archive, specs synced, audit trail complete

---

## Artifacts Archived

- `proposal.md` — original proposal and blast radius
- `design.md` — exact signatures, migration order, architectural approach
- `tasks.md` — implementation checklist (all 10 tasks ✅)
- `specs/security-jwt/spec.md` — NEW requirement spec
- `specs/security-sdk/spec.md` — MODIFIED FR-001, updated tests
- `specs/service-sdk/spec.md` — NEW TenantResolver requirement
- `specs/testkit/spec.md` — NEW PrincipalBuilder requirement
- `verify-report.md` — full verification audit trail

---

## Source of Truth Updated

The following living specs now reflect the validated-once behavior:

- `openspec/specs/security-jwt/spec.md` — new file (first requirements ever for security-jwt)
- `openspec/specs/security-sdk/spec.md` — FR-001 updated with `Option<TenantId>` and new builder signature
- `openspec/specs/service-sdk/spec.md` — new "TenantResolver does not re-validate" requirement
- `openspec/specs/testkit/spec.md` — new "PrincipalBuilder validates at build time" requirement

All four are now the source of truth for their domains. Future changes to these areas refer to these requirements, and deviations require new delta specs and SDD cycles.

---

**Change archived successfully. SDD cycle complete.**
