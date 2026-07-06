# Verify Report: CORE-018b — Restrict RuntimeInner Construction to RuntimeBuilder

**Status: PASS**
**CRITICAL: 0 — WARNING: 0 — SUGGESTION: 0**

**Updated post-`/judgment-day` (3 rounds).** The original verify pass below
described the state immediately after `sdd-apply`. Judgment Day's review
rounds found and fixed 3 further issues not caught by the original apply/verify
cycle: 2 stale doc comments (Round 1), and — after escalating a judge
contradiction over the `dead_code` warning to the user (Round 2) — the user
chose to actually delete `RuntimeInner::new()` entirely rather than keep
accepting the warning, which Round 3 then caught had left a dangling doc
reference to the deleted method (now also fixed). See "Post-Verify Changes"
below for what changed after this report was first written.

## Executive Summary

Implementation conforms to spec.md, design.md, and tasks.md on every checked
axis. `impl Default for RuntimeInner` no longer exists, and a `#[cfg(test)]
pub(crate) fn for_test()` helper backs all in-crate test call sites. All five
known-affected files were migrated to `RuntimeBuilder`. The compile-fail suite
still asserts the same failure class (`E0624: method is private`) at the new
call site. Independently re-ran `cargo build --workspace` and `cargo test
--workspace`: build succeeds with **zero warnings**, full workspace test suite
green with zero failures. All 10 tasks are checked and match the code state.
Non-Goals (`.with_adapter()`/`.with_config()`, kit-config, new authz/tenant
logic) were respected — confirmed by grep and by diffing this change's actual
file set against HEAD.

## Post-Verify Changes (via `/judgment-day`)

- `RuntimeInner::new()` no longer exists — it was narrowed to `pub(crate)` by
  the original apply, then **deleted entirely** once its one remaining caller
  (the `Some(...)` security-providers test) was migrated to call
  `new_with_logger(...)` directly. `new_with_logger` (`pub(super)`) is now the
  crate's **sole** `RuntimeInner` constructor; `for_test()` routes through it.
- The `dead_code` warning on `new()` (previously accepted as a documented
  trade-off) is gone — not suppressed, the dead function was removed.
- `for_test()`'s doc comment (which named the now-deleted `Self::new` as the
  path for the `Some(...)` case) was corrected to name `new_with_logger`.
- The stale "TASK-014 note" doc comment moved from `new()` to
  `new_with_logger()`, updated to state the `pub(crate)`/sole-constructor
  restriction is done and only the `issue_cross_tenant_permit` authorization
  check itself remains pending.

## Verified Against Code (not just apply-progress.md's claims)

### 1. `runtime_builder.rs` — CONFORMS
- `RuntimeInner::new()` no longer exists — deleted post-verify (see "Post-Verify
  Changes"). `new_with_logger` (`pub(super)`) is the sole constructor.
- `impl Default for RuntimeInner` is gone — confirmed by reading the full file;
  no `Default` impl block exists anywhere in it.
- `#[cfg(test)] pub(crate) fn for_test()` exists, wraps `Self::new_with_logger`
  with `logger: None` and a fresh `TeardownStack`, and is used by all 13 of
  the file's own in-crate tests (verified by reading every test function in
  `mod tests`).

### 2. `context/mod.rs` — CONFORMS
Both former `RuntimeInner::default()` call sites (lines 363, 372) now call
`RuntimeInner::for_test()`.

### 3. `authorization_integration.rs` — CONFORMS
`make_runtime` (line 177) constructs via
`RuntimeBuilder::new().with_security(authn, authz).build()`, returns
`(Runtime, Weak<RuntimeInner>)` via `Arc::downgrade(rt.inner())`. `t22`'s
drop-path test (line 297-319) still drops the strong `Runtime` and asserts
`Weak::upgrade()`-driven failure via the "provider error" message — same
semantics as before migration.

### 4. `proxy_codegen.rs` — CONFORMS
All 6 former `RuntimeInner::default()` sites (lines 72, 138, 163, 203 → need
`Weak`; 264, 282 → need `&RuntimeInner`) now go through
`RuntimeBuilder::new().build()` followed by `Arc::downgrade(rt.inner())` or
`rt.inner()` (deref-coerced). Zero remaining `RuntimeInner::default()` or
`RuntimeInner::new()` references in this file.

### 5. `compile_fail/issue_cross_tenant_permit_external.rs` + `.stderr` — CONFORMS
`.rs` constructs via `RuntimeBuilder::new().build().inner().issue_cross_tenant_permit()`.
`.stderr` still asserts `error[E0624]: method \`issue_cross_tenant_permit\` is
private`, pointing at the `.inner().issue_cross_tenant_permit()` call — same
failure class and same target method as before migration, only the source
column shifted (8:25 → 8:30) due to the added `.inner()` hop. Not a weaker or
different failure.

### 6. No remaining direct construction outside `runtime_builder.rs` — CONFORMS
`rg -n "RuntimeInner::new\(|RuntimeInner::default\(\)|RuntimeInner\s*\{" crates/`
returns exactly:
- `runtime_builder.rs:97` (struct def), `:118` (Debug impl), `:128` (impl
  block) — declarations, not construction calls.
- Zero remaining calls to `RuntimeInner::new(` anywhere — the one former
  in-crate test call with explicit security providers
  (`authorization_provider_returns_arc_when_providers_set`) was migrated to
  `RuntimeInner::new_with_logger(...)` post-verify, and `new()` itself was
  deleted since nothing called it in any build configuration.
- No matches in any file outside `service-sdk`. (`TokioRuntimeInner` hits in
  `runtime-tokio` / `runtime` crates are an unrelated, differently-named type —
  not `RuntimeInner`.)

This matches the spec's "RuntimeBuilder::build() remains the sole construction
path" scenario exactly.

### 7. Build & Test — independently re-run, not trusted from apply-progress
- `cargo build --workspace`: **PASS**. Zero warnings (the `dead_code` warning
  on `RuntimeInner::new` that existed at the original verify pass is gone —
  `new()` was deleted post-verify rather than kept as an unused function).
- `cargo test --workspace`: **PASS**, zero failures across every crate,
  including `ego-service-sdk --lib` (58/58), `authorization_integration` (7/7,
  incl. `t22`), `proxy_codegen` (7/7), `cross_tenant_access_contract` (2/2,
  which subsumes the regenerated compile-fail `.stderr`).

### 8. Non-Goals — CONFORMS
- `rg "with_adapter|with_config" crates/service-sdk/src/runtime/runtime_builder.rs`
  → zero matches. Issue #120 surface not touched.
- `git diff HEAD --stat` for this change touches exactly 5 code files
  (`runtime_builder.rs`, `context/mod.rs`, `authorization_integration.rs`,
  `proxy_codegen.rs`, `compile_fail/issue_cross_tenant_permit_external.rs` +
  `.stderr`) plus `tasks.md`. No kit-config file appears in the diff.
- `git diff HEAD -- crates/service-sdk/src/runtime/runtime_builder.rs` shows
  only the visibility narrowing, `Default`-removal/`for_test()`-addition, and
  mechanical test-callsite rewrites — no new authorization or
  tenant-enforcement branches added.

## Tasks vs. Code

All 10 tasks in tasks.md are marked `[x]` and each verified independently
against the corresponding code location above (TASK-001 through TASK-010).
No unchecked tasks, no discrepancy between tasks.md's checklist and actual
code state.

## Issues Found

**CRITICAL**: None
**WARNING**: None
**SUGGESTION**: None

## Verdict

PASS — implementation matches spec.md, design.md, and tasks.md; independently
re-run build and full workspace test suite are green; Non-Goals held.
