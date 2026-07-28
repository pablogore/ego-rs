# Tasks: CORE-036 — Pre-v0.1 Deprecated API Cleanup

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~180-240 (2 files deleted, ~6 files edited, 1 test added) |
| 400-line budget risk | Low |
| Chained PRs recommended | No |
| Chain strategy | single-pr — one coherent cleanup unit; deletions + mechanical test migration + one lint test |
| Delivery strategy | auto-forecast (no explicit label); Low risk, single PR |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: single-pr
400-line budget risk: Low

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Rollback boundary |
|---|---|---|---|---|
| 1 | Remove `ExecutionBackend`/`TokioExecutionBackend`/`SyncTestBackend` | PR1 | `cargo build -p ego-persistent-entity` | Restore both files + `pub mod` lines |
| 2 | Remove `is_cross_tenant_allowed()` + migrate 4 test refs + doc | PR1 | `cargo test -p ego-service-sdk context:: && cargo test -p ego-service-sdk --test smoke --test cross_tenant_access_contract` | Restore method, revert test edits, restore `COOKBOOK.md:422` |
| 3 | Add `no_deprecated_shims_lint` + run zero-reference grep gates | PR1 | `cargo test -p ego-service-sdk no_deprecated_shims_lint` | Delete the lint test |

## Phase 1: Remove the `ExecutionBackend` Trait & Deprecated Backends (Items 1-3)

- [ ] TASK-001 RED: add a temporary compile-time expectation to `crates/persistent-entity/tests/` (or a doc-scan assertion) that fails while `TokioExecutionBackend`/`SyncTestBackend`/`ExecutionBackend` are still declared — concretely, a source-scan test asserting `rg 'ExecutionBackend'` over `crates/persistent-entity/src/**` yields **0** hits; it fails now because the symbols exist. (This RED is deleted or subsumed by TASK-014's workspace gate in Phase 4 — see AC there.)
- [ ] TASK-002 GREEN: delete `crates/persistent-entity/src/execution_backend.rs` and `crates/persistent-entity/src/execution_backend_tokio.rs`; remove `pub mod execution_backend;` and `pub mod execution_backend_tokio;` from `crates/persistent-entity/src/lib.rs:40-41`. AC: TASK-001 green; `cargo build -p ego-persistent-entity` succeeds; `rg 'TokioExecutionBackend|SyncTestBackend|ExecutionBackend|execution_backend' crates/persistent-entity/` returns 0.

## Phase 2: Remove `is_cross_tenant_allowed()` & Migrate Callers (Item 4)

These four are **caller migrations, not RED steps**: `is_cross_tenant_allowed_for(&TenantId)` already
exists, so each swapped assertion passes both before and after the swap. Their purpose is to leave
`is_cross_tenant_allowed()` unreferenced so TASK-007 can delete it; the compiler is the gate that
proves the migration is complete, and the suite MUST stay green at every step.

- [ ] TASK-003 MIGRATE: in `crates/service-sdk/src/context/mod.rs`, move unit test `with_cross_tenant_access_sets_flag` (`:551-561`) to assert `ctx.is_cross_tenant_allowed_for(&destination)` and remove its `#[allow(deprecated)]`. AC: test body uses `_for(&destination)`; no `#[allow(deprecated)]` on it; the test passes.
- [ ] TASK-004 MIGRATE: move unit test `clone_preserves_cross_tenant_flag` (`:564-576`) to `cloned.is_cross_tenant_allowed_for(&destination)`; remove its `#[allow(deprecated)]`. AC: test body uses `_for(&destination)`; no `#[allow(deprecated)]` on it; the test passes.
- [ ] TASK-005 MIGRATE: move `crates/service-sdk/tests/smoke.rs:210` to `assert!(!a.is_cross_tenant_allowed_for(&TenantId::new("tenant-b").unwrap()))`; remove `#[allow(deprecated)]` at `:203`; add the `TenantId` import if absent. AC: assertion uses `_for`; no `#[allow(deprecated)]` on `test_tenant_isolation`; the test passes.
- [ ] TASK-006 MIGRATE: move `crates/service-sdk/tests/cross_tenant_access_contract.rs:7` to assert `!ctx.is_cross_tenant_allowed_for(&dest)` for an arbitrary `dest`; remove `#[allow(deprecated)]` at `:4`; rename the test `is_cross_tenant_allowed_for_defaults_to_false`. AC: assertion uses `_for`; no `#[allow(deprecated)]`; the test passes.
- [ ] TASK-007 REMOVE: delete `ServiceContext::is_cross_tenant_allowed()` and its `#[deprecated]` attribute (`crates/service-sdk/src/context/mod.rs:339-348`). AC: the crate still compiles, proving TASK-003..006 left zero references; `cargo test -p ego-service-sdk context:: --test smoke --test cross_tenant_access_contract` passes; the method no longer exists.
- [ ] TASK-008: delete the `is_cross_tenant_allowed()` deprecated parenthetical from `COOKBOOK.md:422` (keep the `is_cross_tenant_allowed_for(&TenantId)` entry). AC: `rg 'is_cross_tenant_allowed\b' COOKBOOK.md` (excluding `_for`) returns 0.

## Phase 3: Confirm Retentions (Items 5-7) — No Code Change

- [ ] TASK-009: verify (no edit) that the macro-visibility hatches remain: `#[doc(hidden)] pub fn logger/authorization_provider/record_security_denial` (`crates/service-sdk/src/runtime/runtime_builder.rs:403,504,530`) and `pub use async_trait` / `pub use ego_security_sdk as security` (`crates/service-sdk/src/lib.rs:33-38`) are unchanged. AC: `rg '#\[doc\(hidden\)\]' crates/service-sdk/src/` still lists these; justification recorded in `specs/service-sdk/spec.md`.
- [ ] TASK-010: verify (no edit) the testkit `log(Severity,&str)` back-compat coverage (`crates/testkit/src/logger.rs`) and the `logging_bootstrap.rs` example are unchanged. AC: no diff to those files; justification recorded in `specs/service-sdk/spec.md`.
- [ ] TASK-011: verify (no edit) the legacy flat `trace_id` mirror (`crates/service-sdk/src/context/mod.rs:69-83`) is unchanged and carries no `#[deprecated]`. AC: no diff; `with_trace_id`/`trace_id` still compile and pass existing tests; justification recorded in `specs/service-sdk/spec.md`.

## Phase 4: No-Shims Policy Gate (New Capability `api-surface-hygiene`)

The detector MUST be a **pure, callable function** — something like
`fn count_deprecated_attrs(source: &str) -> usize` plus a `fn is_pre_stable(manifest: &str) -> bool`
— so its own correctness is proven by passing fixtures **as arguments**, not by leaving a failing
test in the suite. No task here may end with a red test: `cargo test --workspace` MUST be green at
every step.

- [ ] TASK-012 RED: create `crates/service-sdk/tests/no_deprecated_shims_lint.rs` modeled on `crates/service-sdk/tests/tenant_scoped_lint.rs`, exposing the detector as a pure function over source text plus a pre-stable check over manifest text. Write its three self-tests first, against fixtures passed as arguments: (a) a clean fixture yields **0** detections, (b) a fixture containing one `#[deprecated]` yields exactly **1** detection, (c) a `version = "1.2.0"` manifest is NOT pre-stable while `version = "0.1.0"` is. AC: all three fail to compile or fail assertions before the detector is written — a genuine RED with no fixture left in a permanently-failing state.
- [ ] TASK-013 GREEN: implement the detector and the pre-stable manifest check until TASK-012's three self-tests pass. AC: `cargo test -p ego-service-sdk no_deprecated_shims_lint` passes; the detector reports 1 detection on the dirty fixture and 0 on the clean one; version classification is correct for both manifests.
- [ ] TASK-014 GATE: add the workspace-scanning integration assertion to the same file — ascend from `env!("CARGO_MANIFEST_DIR")` to the `[workspace]` root, read each member's `Cargo.toml`, apply the detector **only** to crates whose `version` is `0.x`, and assert the total is **0**. Run it against the real workspace after Phases 1-2. AC: passes — the real pre-stable `#[deprecated]` count is 0; a crate at `1.x` or above is skipped rather than scanned (this subsumes and retires the TASK-001 temporary RED).

## Phase 5: Zero-Reference Grep Gates & Cross-Cutting Verification

- [ ] TASK-015: run the zero-reference grep gates and confirm each returns 0 —
  `rg 'TokioExecutionBackend|SyncTestBackend|ExecutionBackend|execution_backend' crates/`;
  `rg 'is_cross_tenant_allowed\b' crates/ COOKBOOK.md` (excluding `_for`);
  `rg '#\[deprecated' crates/`;
  `rg '#\[allow\(deprecated\)\]' crates/`. AC: all four return 0 matches.
- [ ] TASK-016: run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace`. AC: exit 0, no regressions; the workspace compiles with every removed symbol gone (compiler-confirmed zero references).
