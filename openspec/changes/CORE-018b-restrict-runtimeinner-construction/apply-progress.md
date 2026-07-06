# Apply Progress: CORE-018b — Restrict RuntimeInner Construction to RuntimeBuilder

**Mode**: Strict TDD (Approval Testing variant — see note below)
**Batch**: 1 of 1 (all tasks completed in a single batch)

## Completed Tasks

- [x] TASK-001 Added `#[cfg(test)] pub(crate) fn for_test() -> Self` to `RuntimeInner`
- [x] TASK-002 Narrowed `RuntimeInner::new()` from `pub` to `pub(crate)`
- [x] TASK-003 Removed `impl Default for RuntimeInner` entirely
- [x] TASK-004 Migrated 13 in-crate `default()` sites in `runtime_builder.rs` test module to `for_test()`
- [x] TASK-005 Migrated 2 `default()` sites in `context/mod.rs` to `for_test()`
- [x] TASK-006 Rewrote `make_runtime` in `authorization_integration.rs` to use `RuntimeBuilder`
- [x] TASK-007 Migrated 6 `default()` sites in `proxy_codegen.rs` to `RuntimeBuilder`
- [x] TASK-008 Rewrote `compile_fail/issue_cross_tenant_permit_external.rs` to construct via `RuntimeBuilder`
- [x] TASK-009 Regenerated `issue_cross_tenant_permit_external.stderr` via `TRYBUILD=overwrite`; manually confirmed same failure class
- [x] TASK-010 Full workspace build + test pass; `rg` invariant confirmed

10/10 tasks complete.

## Files Changed

| File | Action | What Was Done |
|------|--------|----------------|
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modified | Added `#[cfg(test)] pub(crate) fn for_test()`; narrowed `new()` to `pub(crate)`; removed `impl Default for RuntimeInner`; migrated 13 in-crate test call sites from `default()` to `for_test()` |
| `crates/service-sdk/src/context/mod.rs` | Modified | Migrated 2 test call sites from `RuntimeInner::default()` to `RuntimeInner::for_test()` |
| `crates/service-sdk/tests/authorization_integration.rs` | Modified | `make_runtime` now builds via `RuntimeBuilder::new().with_security(authn, authz).build()`, returns `(Runtime, Weak<RuntimeInner>)` via `Arc::downgrade(rt.inner())`; import list updated (dropped `ServiceRegistry`, added `Runtime`/`RuntimeBuilder`) |
| `crates/service-sdk/tests/proxy_codegen.rs` | Modified | 6 sites migrated: 4 need `Weak` (`Arc::downgrade(rt.inner())`), 2 need `&RuntimeInner` (`rt.inner()`, deref-coerced); import list updated (dropped `RuntimeInner`, added `RuntimeBuilder`) |
| `crates/service-sdk/tests/compile_fail/issue_cross_tenant_permit_external.rs` | Modified | Constructs via `RuntimeBuilder::new().build()`, calls `rt.inner().issue_cross_tenant_permit()` |
| `crates/service-sdk/tests/compile_fail/issue_cross_tenant_permit_external.stderr` | Regenerated | Same failure class (`E0624: method is private`), now pointing at line 8 col 30 (the new call site) |
| `openspec/changes/CORE-018b-restrict-runtimeinner-construction/tasks.md` | Modified | All 10 tasks marked `[x]` |

## TDD Cycle Evidence

This change is a **pure visibility restriction + call-site migration** — no new behavior, no new production logic branch. Per the strict-tdd module's "Approval Testing (for refactoring existing code)" pattern: the pre-existing test suite in each touched file already captures the exact behavior that must survive the migration. There is no new expected behavior to describe with a new failing test; the migration itself is the thing under test, and the compiler + existing test suite are the enforcement mechanism (this is explicit in design.md's Testing Strategy: "Build — the compiler is the enforcement").

| Task | Test File | Layer | Safety Net (baseline) | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------------------|-----|-------|-------------|----------|
| 001 | `runtime_builder.rs` (additive) | N/A | ✅ `cargo test -p ego-service-sdk` 100% green pre-change | N/A (purely additive, no removed behavior) | ✅ `cargo build -p ego-service-sdk` unchanged | ➖ Skipped: structural, one shape (helper wraps one fixed call) | ➖ None needed |
| 002 | workspace build | N/A | ✅ baseline above | ✅ Compiler error is the "RED" — `authorization_integration.rs` fails as predicted | N/A (fix deferred to 006) | N/A | N/A |
| 003 | workspace build | N/A | ✅ baseline above | ✅ Compiler enumerates 15+6+1 broken call sites exactly as Migration Map predicted | N/A (fixes in 004/005/007/008) | N/A | N/A |
| 004 | `runtime_builder.rs` (approval) | Unit | ✅ 58/58 in-crate lib tests green pre-removal (as approval baseline) | N/A — approval-test pattern: existing assertions ARE the spec | ✅ `cargo test -p ego-service-sdk --lib` → 58/58 pass post-migration | ✅ 13 distinct call sites covering plain/mut/Arc-wrapped/private-field-mutation variants | ➖ None needed — mechanical rename only |
| 005 | `context/mod.rs` (approval) | Unit | ✅ same 58/58 baseline (shared lib test binary) | N/A — approval-test pattern | ✅ same lib test run, 58/58 pass | ➖ Single: both sites are the same shape | ➖ None needed |
| 006 | `authorization_integration.rs` (approval) | Integration | ✅ 7/7 tests green pre-migration (verified via prior lib-test run pattern; this file itself uses the old constructor until this task) | N/A — approval-test pattern, semantics must not change | ✅ `cargo test -p ego-service-sdk --test authorization_integration` → 7/7 pass, incl. `t22` drop-path | ✅ 7 call sites across T-18..T-24 all still exercise the rewritten helper identically | ➖ None needed |
| 007 | `proxy_codegen.rs` (approval) | Integration | ✅ 7/7 tests pass pre-migration pattern (same test count post) | N/A — approval-test pattern | ✅ `cargo test -p ego-service-sdk --test proxy_codegen` → 7/7 pass | ✅ 6 sites: 4 `Weak`-needing + 2 `&RuntimeInner`-needing (deref coercion), both shapes verified | ➖ None needed |
| 008-009 | `compile_fail/issue_cross_tenant_permit_external.rs` + `.stderr` | Compile-fail | ✅ pre-existing `.stderr` captured as approval baseline before touching source (task ordering enforced this: source rewritten in 008, `.stderr` regenerated only in 009, diffed manually) | N/A — compile-fail test, "RED" is the compiler already rejecting the old private-method call | ✅ `TRYBUILD=overwrite` regenerated stderr; manually diffed: same `E0624 method is private` class, same target method, only line/col shifted (8:25 → 8:30) due to the new `.inner()` hop | ➖ Single scenario (one compile-fail case) | ➖ None needed |
| 010 | full workspace | N/A | N/A (this IS the final safety net) | N/A | ✅ `cargo build --workspace` + `cargo test --workspace` — zero failures across all crates | ✅ `rg` invariant confirms zero remaining external/production construction sites | N/A |

### Test Summary

- **Total tests written**: 0 new test functions (pure migration; no new behavior introduced)
- **Total tests passing**: 194 (security-jwt/oidc suite, unaffected) + 58 (service-sdk lib) + 7 (authorization_integration) + 7 (proxy_codegen) + 2 (cross_tenant_access_contract, incl. 5 compile_fail sub-cases) + full workspace suite — all green, zero failures, zero pre-existing-failure exceptions triggered
- **Layers used**: Unit (58 in-crate), Integration (14 across 2 external test files), Compile-fail (1 regenerated .stderr)
- **Approval tests** (refactoring): all touched call sites — the existing assertions in each file are the approval baseline; none were weakened or removed, only their construction path changed
- **Pure functions created**: 0 (this is a visibility change, not new logic)

## Deviations from Design

1. **Line numbers shifted from design.md's estimates** (expected — design used `~` line numbers). Actual: `new()` narrowed at (then) line 138 as designed; `Default` impl removed at (then) line 251 as designed; after removal, the in-crate `new(...)` test call moved from ~513 to ~507 (file shrank by 6 lines from the removed `impl Default` block). No functional deviation.
2. **`tasks.md` referenced `--test compile_fail` as the verify command for TASK-009/TASK-010's compile-fail check** — no test binary named `compile_fail` exists in this crate; the actual trybuild harness that includes `issue_cross_tenant_permit_external.rs` is `tests/cross_tenant_access_contract.rs` (confirmed via `rg "compile_fail"` across `tests/*.rs`). Used `cargo test -p ego-service-sdk --test cross_tenant_access_contract` instead — same underlying trybuild case, correct target name. No scope or outcome deviation, just a task-doc naming correction worth flagging.
3. Test function names like `runtime_inner_default_creates_empty` (in `runtime_builder.rs`) were left unrenamed even though they now call `for_test()` internally — tasks.md only asked to migrate call sites, not rename test functions. Flagging as a minor pre-existing-name/intent drift, not fixed (out of scope; would be gratuitous churn beyond the assigned tasks).
4. `cargo build --workspace` (non-test profile) emits one benign `dead_code` warning on `RuntimeInner::new()` — in a non-test build, its only callers are `#[cfg(test)]` code, so with zero production callers rustc's dead_code lint fires. This is expected and matches design's own statement that `new()`'s only callers after migration are in-crate tests. No `deny(warnings)` lint gate exists anywhere in the workspace, so this does not fail any build or CI gate. Not suppressed, per "implement exactly this, don't re-decide" — design's Interfaces/Contracts snippet shows no `#[allow(dead_code)]` on `new()`.

## Issues Found

None. The compiler-driven survey (TASK-002/TASK-003) confirmed the design's Migration Map was exhaustive — no call sites outside the 5 named files were ever surfaced by `cargo build --workspace --tests`.

## Remaining Tasks

None — all 10 tasks complete.

## Workload / PR Boundary

- Mode: single PR (per tasks.md Review Workload Forecast: Low risk, no chaining)
- Current work unit: Unit 1 — the entire change (compiler-enforced migration; splitting would leave the crate non-compiling mid-way)
- Boundary: starts at `RuntimeInner::new()`/`Default` visibility narrowing, ends at full workspace green + `rg` invariant check
- Estimated review budget impact: within the ~100-180 line forecast; actual diff is 6 files (5 code + tasks.md), all mechanical call-site rewrites plus one regenerated `.stderr` snapshot

## Post-Verify Code Review Fixes

`/code-review` (high effort, 8 finder angles + verify pass) ran after `sdd-verify` PASS. 3 candidates survived verification as low-severity PLAUSIBLE (none were correctness bugs); all 3 applied:

1. `for_test()` (`runtime_builder.rs`) now routes through `new_with_logger(...)` instead of `new(...)` — the same constructor `RuntimeBuilder::build()` uses in production — so in-crate tests built on this fixture exercise the same construction path, not a bypass of it. Doc comment added noting it always yields `security_providers: None` and pointing to the explicit `new(...)` call for the `Some(...)` case.
2. `crates/service-sdk/tests/proxy_codegen.rs`: extracted a shared `fn test_runtime() -> Runtime` fixture, replacing 6 identical `RuntimeBuilder::new().build()` call sites.
3. (Same fix as #1 — the "hardcoded `None`" and "bypasses production path" findings were addressed by the same `new_with_logger` change plus its doc comment.)

Re-ran `cargo build --workspace` and `cargo test --workspace` after applying: zero failures, same one pre-existing benign `dead_code` warning on `RuntimeInner::new()` (unchanged from before these fixes — `new()` still has exactly one in-crate test caller at the `Some(...)` providers test).

## Status

10/10 tasks complete, verified, post-verify review fixes applied. Ready to commit.
