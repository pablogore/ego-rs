# Archive Report — Remove Ambient ServiceContext (CORE-010A)

**Change**: remove-ambient-service-context
**Archived at**: 2026-06-22
**Mode**: openspec
**Delivery strategy**: single-pr
**Verdict**: PASS WITH WARNINGS (pre-existing clippy warnings in unrelated crates)
**SDD Cycle**: Complete

---

## Task Completion Gate

- [x] All 17 tasks marked `[x]` in tasks.md
- [x] No stale unchecked implementation tasks
- [x] No CRITICAL issues in verify-report
- [x] No workspace-planning action context mode

## Specs Synced

### 1. security-sdk — MERGED (delta into existing main spec)

**Main spec**: `openspec/specs/security-sdk/spec.md` (updated)
**Delta**: `openspec/changes/archive/2026-06-22-remove-ambient-service-context/specs/security-sdk/spec.md`

| Action | Count | Details |
|--------|-------|---------|
| MODIFIED | 1 | NFR-005: extended prohibition from `SecurityContext` only to also include `ServiceContext` task-local/thread-local/global patterns; added 4 scenarios |

**Merge notes**:
- Replaced existing NFR-005 block (lines 306-311) with expanded version from delta
- Preserved all other requirements, NFRs, invariants, and scenarios unchanged
- No REMOVED requirements — the delta is additive/extending, not destructive

### 2. service-sdk — CREATED (new main spec)

**Main spec**: `openspec/specs/service-sdk/spec.md` (created)
**Delta/Full spec**: `openspec/changes/archive/2026-06-22-remove-ambient-service-context/specs/service-sdk/spec.md`

The delta spec IS a full spec (main spec did not exist). Copied directly.

| Section | Count |
|---------|-------|
| Requirements | 5 (No Ambient Context APIs, Explicit Context in Proxy Dispatch, Explicit Propagation Through Spawned Tasks, Test Suite Uses Explicit Construction Only, Build and Lint Gates Pass) |
| NFRs | 3 (No Behavioral Regression, No New Sync Primitives, Dependency Visibility) |
| Invariants | 3 (Single Context Model, Interceptor Order Preserved, Tenant Enforcement Preserved) |
| Scenarios | 11 total across all requirements |

## Archive Contents

| Artifact | Status |
|----------|--------|
| proposal.md | ✅ Archived |
| spec.md | ✅ Archived |
| specs/ (security-sdk, service-sdk) | ✅ Archived |
| design.md | ✅ Archived |
| tasks.md | ✅ Archived (17/17 tasks complete) |
| verify-report.md | ✅ Archived (PASS WITH WARNINGS) |
| state.yaml | ✅ Archived |

## Source of Truth Updated

- `openspec/specs/security-sdk/spec.md` — NFR-005 updated to include `ServiceContext` ambient prohibition
- `openspec/specs/service-sdk/spec.md` — new full spec for context propagation requirements

## Archive Path

```
openspec/changes/remove-ambient-service-context/
  → openspec/changes/archive/2026-06-22-remove-ambient-service-context/
```

## Warnings Acknowledged

1. **Pre-existing clippy warnings**: `ego-security-sdk` (module_inception) and `ego-domain` (too_many_arguments, implied_bounds_in_impls, assertions_on_constants) — not caused by this change, not a blocker per verify-report verdict
2. **Missing formal TDD Cycle Evidence table**: apply-progress was saved as summary rather than structured table — all empirical checks confirm TDD was followed

No CRITICAL issues found. Archive proceeds without override.

## Verdict

SDD cycle CORE-010A is complete. The change has been fully planned, proposed, specified, designed, implemented (TDD), verified, and archived.
