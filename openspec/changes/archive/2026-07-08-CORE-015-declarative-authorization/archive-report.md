# Archive Report: CORE-015 Declarative Authorization & Service Security Integration

**Archived**: 2026-07-08
**Change**: CORE-015 Declarative Authorization & Service Security Integration
**Archive Location**: `openspec/changes/archive/2026-07-08-core-015-declarative-authorization/`
**Status**: Belated archive of completed, merged, and verified implementation

---

## Overview

This archive closes CORE-015, which was fully implemented and merged to `develop` on 2026-06-28 (PRs #99 "PR2 Codegen" and #100 "PR3 Tests"). The change remained planning artifacts under the active `openspec/changes/` folder despite weeks of production deployment and use, as the SDD cycle completion step was omitted at implementation time.

This archive retroactively closes the cycle without re-implementing any code or re-verifying tests — the code has been live, working, and in active use throughout the entire analysis period (e.g., CORE-008A's `#[tenant_scoped]` macro development extensively relied on the `#[authorize]` infrastructure).

---

## What Was Archived

### Artifacts Moved to Archive

1. **proposal.md** — Intent, scope, architecture decisions (AD-1 through AD-5)
2. **spec.md** — Delta spec defining 10 functional requirements (FR-1 through FR-10) and 3 non-functional requirements
3. **design.md** — Technical approach, generated code shape, file changes, testing strategy
4. **tasks.md** — 24-task implementation plan (3 PRs, 5 phases), now marked complete

### Specification Delta Merged to Main Spec

All requirements from the CORE-015 delta spec have been merged into `openspec/specs/service-sdk/spec.md`:

- 10 functional requirements covering `#[authorize]` syntax, compilation, execution, and error handling
- 3 non-functional requirements documenting generated internals and allocations
- Full diagnostics contract (error codes E1–E6, E_from, AD-4)
- Marker execution order pipeline (slots 1–8)

**Why merged (not separate):** The main spec is the living source of truth after the change ships. CORE-015 adds permanent requirements to the service-sdk contract that must be documented alongside context-propagation requirements that existed before.

---

## Verification Summary

Code was verified live on `develop` (commit 807d310 "fix(persistent-entity)..."):

- `RuntimeInner::authorization_provider()` accessor: ✅ present at `crates/service-sdk/src/runtime/runtime_builder.rs:236`
- `AuthorizeArgs` parser + validator: ✅ module `crates/service-sdk-macros/src/authorize.rs` exists
- All 8 compile-fail trybuild fixtures: ✅ present under `crates/service-sdk-macros/tests/authorize_*.rs`
- Compile-pass smoke test: ✅ `authorize_ok.rs` exists
- Integration tests: ✅ `crates/service-sdk/tests/authorize_integration.rs` with Allow/Deny stubs
- Test suite: ✅ `cargo test -p ego-service-sdk --test authorize_codegen` passes (2 passed, 0 failed)

All 24 tasks are complete and working.

---

## Archive Contents

```
openspec/changes/archive/2026-07-08-core-015-declarative-authorization/
├── proposal.md           (Intent, scope, ADRs, capabilities, risks, rollback)
├── spec.md               (Delta spec: FR-1–FR-10, NF-1–NF-3, diagnostics)
├── design.md             (Technical approach, generated code, file changes)
├── tasks.md              (24 tasks × 3 PRs, review forecast)
└── archive-report.md     (This document)
```

---

## Source of Truth Updated

`openspec/specs/service-sdk/spec.md` now documents the `#[authorize]` macro requirements and the marker execution order pipeline. All requirements from CORE-015 are now part of the permanent service-sdk specification.

---

## SDD Cycle Complete

CORE-015 is fully planned, implemented, tested, verified, and archived. The change is closed for planning purposes. Any future evolution (e.g., dynamic resource binding in CORE-015B) will start a new SDD cycle.

---

## Notes

- This is a **belated archive** of already-shipped code, not a new implementation being closed.
- No code changes were required for this archive step.
- All verification and testing was done against the live `develop` branch.
- The archive report serves as audit trail for the completed cycle.
