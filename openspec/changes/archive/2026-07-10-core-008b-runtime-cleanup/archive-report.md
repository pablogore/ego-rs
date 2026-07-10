# Archive Report: CORE-008B — Runtime Cleanup & API Consolidation

**Date Archived**: 2026-07-10
**Change Status**: COMPLETE
**Verification Status**: PASS (0 CRITICAL, 0 WARNING)

---

## What Changed

CORE-008A migrated tenant context to `tenant_hint()`/`canonical_tenant()` but left the
workspace describing and exercising two competing models. This change closed that gap:

1. Deleted the deprecated `ServiceContext::tenant_id()`/`has_tenant()` accessor methods
   and their two legacy-alias unit tests, migrating all 15+2 call sites across 4
   integration test files and `context/mod.rs` itself to `tenant_hint()`/`canonical_tenant()`
   per the Accessor Selection Rule (pipeline-stage-based, not habit-based).
2. Corrected `docs/architecture.md` lines describing `ServiceContext` as `TaskLocal`-scoped
   or ambient-propagating — stale since the archived `2026-06-22-remove-ambient-service-context`
   change.
3. Deleted the orphaned pre-migration `ExecutionContext`/`DomainExecutionContext`
   (`crates/domain/src/context.rs`), `RuntimeExecutionContext` (`crates/runtime/src/context.rs`),
   and `ExecutionEnvelope` (`crates/domain/src/envelope.rs`) — implemented, tested, publicly
   exported, but zero production callers (grep-verified). `CommandContext`
   (`crates/persistent-entity/src/command_context.rs`) is the actual runtime path, built
   independently, not a drop-in replacement for the deleted types.

**Scope**: Delivered across two stacked PRs, both merged to `develop`.

---

## Specs Merged

| Spec Domain | Action | Details |
|---|---|---|
| `service-sdk` | Modified | FR-008 reworded to drop the deleted `ExecutionContext` reference (historical note added) |
| `service-sdk` | Added | 4 new requirements: pipeline-stage tenant accessor selection, execution-context abstraction removal, no-deprecated-tenant-accessors, explicit-propagation-only architecture docs |

---

## Implementation Summary

**All 12 tasks completed** ✅, delivered across two PRs:

- **PR1** (`opsx/core-008b-runtime-cleanup-pr1-*`, #145): removed deprecated
  `tenant_id()`/`has_tenant()` accessors and their legacy-alias tests; migrated the 4
  integration test files to `tenant_hint()`/`canonical_tenant()` per the Accessor
  Selection Rule; corrected `docs/architecture.md`; fixed a stale `COOKBOOK.md`
  `tenant_id()` example.
- **PR2** (`opsx/core-008b-runtime-cleanup-pr2-orphan-types`, #146): deleted the orphaned
  `ExecutionContext`/`DomainExecutionContext`/`RuntimeExecutionContext`/`ExecutionEnvelope`
  family and their `lib.rs` re-exports (found during PR2 code review, added to scope with
  an explicit decision entry rather than deferred); fixed the `COOKBOOK.md` File
  Navigation Map row that still described `crates/domain/src/context.rs` as containing
  `ExecutionContext`; reworded the living-spec tenant-authority-precedence line to drop
  the now-deleted identifier.

**Diff (PR2 alone)**: 6 files changed, 14 insertions(+), 712 deletions(-) — pure deletion
of dead, zero-caller code, matching the tasks.md Review Workload Forecast.

---

## Verification Outcome

**Verdict: PASS** — 0 CRITICAL, 0 WARNING.

Runtime evidence independently re-executed at verify time (not trusted from
apply-progress): `cargo build --workspace` clean; `cargo test --workspace` 100% pass,
zero `FAILED`/`panicked`/`error[` across all crates, integration tests, and doctests;
`cargo doc --workspace --no-deps` builds successfully; `cargo test -p ego-domain --lib`
181 passed, identity types intact after the `context.rs` partial edit.

All ADDED/MODIFIED requirement scenarios re-verified by direct grep against the
workspace (zero `ExecutionContext` matches in `crates/`, zero `tenant_id()`/`has_tenant()`
call sites, zero `TaskLocal`/ambient claims in `docs/architecture.md` describing
`ServiceContext`).

The verify-report originally recorded one WARNING (a stale `COOKBOOK.md` File Navigation
Map row). That fix was already included in the same PR2 squash-merge commit (`648aa59`,
"docs(cookbook): drop deleted `ExecutionContext` trait from file map") — confirmed by
`grep -n "ExecutionContext" COOKBOOK.md` returning zero matches. The verify-report was
corrected to PASS with no outstanding findings before archiving.

---

## Non-Goals Honored

1. **The `ServiceContext.tenant_id` field itself was out of scope for this change** — only
   the deprecated *accessor methods* were removed; the field stayed as a non-authoritative
   hint per AD-011. (It was subsequently privatized by a separate, later follow-up PR
   (#151) — tracked independently of this change.)
2. **No runtime behavior changes** — pure API/doc/dead-code cleanup.
3. **No new runtime features, clustering, streaming, or OAuth2 work.**

---

## Rollback Plan

Low-risk: all changes are docs/test/dead-code edits, revertible cleanly via `git revert`.
No data, wire, or persistence impact in either direction.

---

## Known, Out-of-Scope Side Effect: `tenant_id` Visibility Drift

`specs/service-sdk/spec.md`'s archived delta (this change's own frozen copy) contains a
scenario asserting `ServiceContext.tenant_id` "remains a `pub` field" and directly sets
`ctx.tenant_id = Some(...)`. That was accurate when CORE-008B was scoped and implemented —
this change's proposal explicitly listed the field's visibility as **out of scope**,
touching only the deprecated *accessor methods*.

The field was subsequently privatized by an unrelated, later follow-up (CORE-008A
closure fix, PR #151), after CORE-008B's delta spec had already been written. The
archived delta is left unedited as the historical record of what this change actually
scoped and verified — rewriting it now would misrepresent history the same way
CORE-008A's archive-report addendum warns against. The **living spec**
(`openspec/specs/service-sdk/spec.md`, FR-010) has been updated to reflect the current,
post-#151 contract: both `tenant_id` and `resolved_tenant` are private, with no public
mutator reaching either after construction.

---

## SDD Cycle Closure

**Proposal**: ✅ Origin, scope, Accessor Selection Rule, decisions resolved up front
**Spec**: ✅ FR-008 reworded, 4 new requirements added, merged into living `service-sdk` spec
**Tasks**: ✅ All 12 tasks completed across 2 PRs
**Verification**: ✅ PASS, 0 CRITICAL, 0 WARNING (stale COOKBOOK.md note corrected before archive)
**Archive**: ✅ Change folder moved to archive, specs synced, audit trail complete

---

## Artifacts Archived

- `proposal.md` — origin, scope, Accessor Selection Rule, decisions, blast radius
- `tasks.md` — implementation checklist (all 12 tasks ✅)
- `specs/service-sdk/spec.md` — delta spec (1 modified + 4 new requirements)
- `verify-report.md` — full verification audit trail

---

## Source of Truth Updated

`openspec/specs/service-sdk/spec.md` now reflects the post-cleanup contract: no
deprecated tenant accessors, pipeline-stage-correct accessor selection, no orphaned
execution-context abstractions, and an architecture doc describing explicit propagation
only. Future changes to these areas refer to this spec; deviations require a new delta
spec and SDD cycle.

---

**Change archived successfully. SDD cycle complete.**
