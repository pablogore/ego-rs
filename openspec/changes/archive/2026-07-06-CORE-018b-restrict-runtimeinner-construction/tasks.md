# Tasks: CORE-018b — Restrict RuntimeInner Construction to RuntimeBuilder

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~100-180 (mechanical call-site rewrites across 5 files) |
| 400-line budget risk | Low |
| Chained PRs recommended | No |
| Suggested split | Single PR — visibility restriction + call-site migration, no cross-crate fan-out |
| Delivery strategy | ask-on-risk |
| Chain strategy | n/a |

Decision needed before apply: No
Chained PRs recommended: No
400-line budget risk: Low

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Single PR | PR 1 | Compiler-enforced migration (`cargo build --workspace` after each visibility narrowing surfaces the next site). All 5 files land together; splitting would leave the crate in a non-compiling intermediate state. |

---

## Phase 1: Narrow Visibility (`src/runtime/runtime_builder.rs`)

- [x] TASK-001 Add `#[cfg(test)] pub(crate) fn for_test() -> Self` to `impl RuntimeInner` in `crates/service-sdk/src/runtime/runtime_builder.rs`, wrapping `Self::new(ServiceRegistry::new(), Arc::new(InterceptorChain::new()), None)`, per design.md's Interfaces/Contracts block verbatim. Purely additive — do not remove or narrow anything yet. (Spec: "In-crate test helper stays crate-private") — Verify: `cargo build -p ego-service-sdk` compiles unchanged.
- [x] TASK-002 Narrow `RuntimeInner::new()` (line ~138) from `pub` to `pub(crate)`. (Spec: "RuntimeInner Not Publicly Constructible"; Design Decision 3) — Verify: `cargo build --workspace` — record every error the compiler surfaces (expect `tests/authorization_integration.rs`); in-crate call sites (including the `new(.., Some(..))` test at ~line 513) keep compiling unchanged since they're same-crate.
- [x] TASK-003 Remove `impl Default for RuntimeInner` (line ~251) entirely — no `pub(crate)` variant exists for a trait impl (Design Decision 1). (Spec: "RuntimeInner Not Publicly Constructible") — Verify: `cargo build --workspace` — record every error the compiler surfaces (expect: ~13 sites in this file's own test module, 2 in `context/mod.rs`, 6 in `tests/proxy_codegen.rs`, 1 in `tests/compile_fail/issue_cross_tenant_permit_external.rs`). This is the compiler-driven survey confirming the Migration Map is complete — if additional sites appear, add them to the phases below before continuing.
- [x] TASK-004 Migrate the ~13 `RuntimeInner::default()` call sites broken by TASK-003, inside this file's own test module, to `RuntimeInner::for_test()`. Sites that additionally mutate the private `resolved` table afterward keep working unchanged (`for_test()` returns an owned `Self`, same as `default()` did). (Design: Migration Map row 1) — Verify: `cargo test -p ego-service-sdk runtime_builder` passes.

## Phase 2: In-Crate Test Migration (`src/context/mod.rs`)

- [x] TASK-005 Migrate the 2 `RuntimeInner::default()` call sites (lines ~363, ~372) to `RuntimeInner::for_test()`. (Design: Migration Map row 2) — Verify: `cargo test -p ego-service-sdk context` passes.

## Phase 3: External Test Migration — `authorization_integration.rs`

- [x] TASK-006 Rewrite the `make_runtime` helper (line ~181) in `crates/service-sdk/tests/authorization_integration.rs` from `Arc::new(RuntimeInner::new(reg, chain, Some((authn, authz))))` to `RuntimeBuilder::new().with_security(authn, authz).build()`, returning `(Runtime, Weak<RuntimeInner>)` via `Arc::downgrade(rt.inner())` so callers keep both the live `Runtime` and the drop-detection handle. (Design: Migration Map row 3) — Verify: `cargo test -p ego-service-sdk --test authorization_integration` passes, including the `t22` drop-path assertion that `Weak::upgrade()` returns `None` after the `Runtime` is dropped (semantics must match pre-migration behavior exactly).

## Phase 4: External Test Migration — `proxy_codegen.rs`

- [x] TASK-007 Migrate the 6 `RuntimeInner::default()` call sites (lines ~72, ~138, ~163, ~203 → need a `Weak`; ~264, ~282 → need `&RuntimeInner`) to `let rt = RuntimeBuilder::new().build();` followed by `Arc::downgrade(rt.inner())` or `rt.inner()` (deref to `&RuntimeInner`) as each site requires. (Design: Migration Map row 4) — Verify: `cargo test -p ego-service-sdk --test proxy_codegen` passes.

## Phase 5: Compile-Fail Test Rewrite + Stderr Regeneration

- [x] TASK-008 Rewrite `crates/service-sdk/tests/compile_fail/issue_cross_tenant_permit_external.rs` to construct via `RuntimeBuilder::new().build().inner().issue_cross_tenant_permit()` in place of the old `RuntimeInner::default()` + private-method-call sequence. Do NOT touch the `.stderr` file in this task — it must still reflect the OLD source so the diff in TASK-009 is meaningful. (Design: Migration Map row 5)
- [x] TASK-009 Regenerate `issue_cross_tenant_permit_external.stderr` (`TRYBUILD=overwrite cargo test -p ego-service-sdk --test cross_tenant_access_contract` — the trybuild harness that includes this compile-fail case; no test binary named `compile_fail` exists in this crate). Diff the old `.stderr` against the regenerated one and manually confirm: the failure is still "method `issue_cross_tenant_permit` is private" (or equivalent visibility-error wording) targeting the call introduced in TASK-008 — not a different line, a different error class, or an incidental typo/type mismatch. (Spec: "External crate cannot construct RuntimeInner directly"; Design: Testing Strategy row 1) — Verify: `cargo test -p ego-service-sdk --test cross_tenant_access_contract` passes with the regenerated `.stderr` committed.

## Phase 6: Full Workspace Verification

- [x] TASK-010 Run `cargo build --workspace` and `cargo test --workspace`; confirm every test passes with no site missed (Proposal Success Criteria; Design Testing Strategy "Build" row — the compiler is the enforcement mechanism). Then run `rg "RuntimeInner\s*\{|RuntimeInner::new\(|RuntimeInner::default\(\)" crates/` and confirm every remaining match resolves to `RuntimeBuilder::build()`'s internal `new_with_logger` chain or an in-crate `#[cfg(test)]`/`pub(crate)` helper, with none originating from a crate other than `service-sdk`. (Spec: "RuntimeBuilder::build() remains the sole construction path"; Proposal Success Criteria)

## Acceptance Criteria (per task)

Each TASK above is verifiable independently: TASK-001 by a clean build (additive only), TASK-002/003 by the compiler enumerating the exact call sites that must move next (confirming the Migration Map's completeness), TASK-004 through TASK-009 by their respective crate/file-scoped `cargo test` invocation passing, and TASK-010 by the full-workspace build/test run plus the `rg` grep matching the spec's stated invariant.
