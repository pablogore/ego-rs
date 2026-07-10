# Verification Report — CORE-008B Runtime Cleanup & API Consolidation

**Change**: core-008b-runtime-cleanup
**Branch verified**: `opsx/core-008b-runtime-cleanup-pr2-orphan-types` (batch 2, on top of `develop` with batch 1 / PR1 #145 already merged)
**Verdict**: PASS

## Task completeness

12/12 tasks in `tasks.md` marked `[x]`. No unchecked items (verified via `rg "^### TASK-.*\[ \]" tasks.md` — zero matches).

## Runtime evidence (independently re-executed, not trusted from apply-progress)

| Command | Result |
|---|---|
| `cargo build --workspace` | Clean, 0 errors |
| `cargo test --workspace` | 100% pass across all crates/integration tests/doctests, 0 failures (full log grep for `FAILED`/`panicked`/`error[` — zero matches) |
| `cargo doc --workspace --no-deps` | Builds successfully; 20 pre-existing broken-intra-doc-link warnings, none referencing `deprecated` or `ExecutionContext` |
| `cargo test -p ego-domain --lib` | 181 passed — identity types (`AggregateId`/`EntityId`/`TenantId`/etc.) and their tests intact after `context.rs` partial edit |

## Spec scenario re-verification

### MODIFIED: Exactly One Canonical In-Runtime Tenant Representation
`CanonicalTenant::scoped`/`systemwide` confirmed `pub(super)` in `crates/service-sdk/src/runtime/tenant.rs`; both named tests (`canonical_tenant_scoped_is_constructible_within_runtime`, `canonical_tenant_systemwide_is_constructible_within_runtime`) exist and pass.

### ADDED: Tenant Access MUST Match the Pipeline Stage
`ServiceContext::tenant_hint()`/`canonical_tenant()`/`has_tenant_hint()` present with correct `resolved_tenant` gating (set only via `set_resolved_tenant`, `pub(crate)`); no `tenant_id()`/`has_tenant()` methods remain on `ServiceContext` (`rg "fn tenant_id|fn has_tenant" crates/service-sdk/src/context/mod.rs` — zero hits). All 4 integration test files clean of deprecated accessor calls.

### ADDED: Unused Execution-Context Abstractions Are Removed
`rg "ExecutionContext" crates/ --type rust` → 0 matches. `domain/src/context.rs` -370 lines (identity types + their tests kept intact); `runtime/src/context.rs` deleted entirely (-326 lines); both `lib.rs` re-export lists cleaned.

### ADDED: Workspace Contains No Deprecated Tenant Accessors
`rg "\.tenant_id\(\)|\.has_tenant\(\)" crates/` returns matches, but ALL are `CanonicalTenant::tenant_id()` or unrelated domain accessors (`EventStreamElement`, Postgres event row) — none is `ServiceContext::tenant_id()`, confirmed deleted. Zero `#[deprecated]` warnings anywhere in build/doc output.

### ADDED: Architecture Documentation Describes the Explicit-Propagation Model Only
`rg "TaskLocal|ambient" docs/architecture.md` returns 1 match — but it is the negation sentence itself ("no ambient/TaskLocal read"), compliant with the scenario's intent (zero matches *describing* `ServiceContext` propagation as ambient).

### Living spec (`openspec/specs/service-sdk/spec.md`)
`rg "ExecutionContext"` → 1 match, the expected historical note only, per TASK-011 acceptance.

## Proposal Success Criteria

| Criterion | Status |
|---|---|
| Build/test pass, `tenant_id()`/`has_tenant()` fully removed, no `#[allow(deprecated)]` left | CONFIRMED |
| `docs/architecture.md` has no TaskLocal/ambient-propagation claims | CONFIRMED |
| `ExecutionContext`/`DomainExecutionContext`/`RuntimeExecutionContext` deleted, workspace builds green | CONFIRMED |
| No stale documentation examples referencing deleted accessors (e.g. `COOKBOOK.md`) | CONFIRMED |

## Findings

### CRITICAL
None.

### WARNING
None. The COOKBOOK.md File Navigation Map row for `crates/domain/src/context.rs` (flagged below at report-writing time) was fixed within the same PR2 batch — commit `648aa59` includes "docs(cookbook): drop deleted `ExecutionContext` trait from file map." Confirmed clean: `grep -n "ExecutionContext" COOKBOOK.md` returns zero matches.

### SUGGESTION
None outstanding.

## Diff stat (develop..HEAD, batch 2 only)

```
6 files changed, 14 insertions(+), 712 deletions(-)
```

Matches the tasks.md Review Workload Forecast's ~695-line estimate for Phase 3 (pure deletion, zero-caller code).

## Next recommended

`sdd-archive`.
