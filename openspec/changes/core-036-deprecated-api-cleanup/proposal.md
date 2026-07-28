# Proposal: CORE-036 — Pre-v0.1 Deprecated API Cleanup

## Intent

`PRD.md:140` (Design Constraints §7) states: *"**No shims** — when a public API is removed, it is
removed. No deprecated aliases in pre-stable crates."* The workspace ships **exactly three live
`#[deprecated]` attributes** (`rg '#\[deprecated' crates/`), all in `version = "0.1.0"` crates —
standing violations of that stated policy. CORE-036 removes the deprecated surface before v0.1,
migrates its in-repo callers to the safe replacements, documents the genuinely-intentional
retentions so they are not re-flagged, and makes the no-shims policy **verifiable** by a
`cargo test`-participant gate. Full per-item inventory and classification: `explore.md`.

## Scope

### In Scope

- **REMOVE** `TokioExecutionBackend`, `SyncTestBackend` (`crates/persistent-entity/src/execution_backend_tokio.rs`) and the `ExecutionBackend` trait (`crates/persistent-entity/src/execution_backend.rs`) — delete both files and the two `pub mod` lines in `crates/persistent-entity/src/lib.rs:40-41`.
- **REMOVE** `ServiceContext::is_cross_tenant_allowed()` (`crates/service-sdk/src/context/mod.rs:339-348`) and **migrate** its four in-repo test references to `is_cross_tenant_allowed_for(&TenantId)`; delete the deprecated-mention doc line at `COOKBOOK.md:422`.
- Remove every now-orphaned `#[allow(deprecated)]` suppressor tied to the removed symbols (2 in `context/mod.rs`, 1 in `smoke.rs`, 1 in `cross_tenant_access_contract.rs`, 5 in `execution_backend_tokio.rs`).
- Add a `no_deprecated_shims_lint` source-scan test enforcing `#[deprecated]` count == 0 in pre-stable crates (the `PRD.md:140` gate) — a pure, fixture-testable detector applied only to crates whose own `Cargo.toml` declares a `0.x` version.
- **DOCUMENT retentions** (no code change, justification recorded in specs): the `#[doc(hidden)] pub` macro hatches, the testkit `log(Severity,&str)` back-compat coverage, and the legacy flat `trace_id` mirror.

### Out of Scope (Non-Goals / Follow-ups)

- The `#[doc(hidden)] pub` macro-visibility hatches (`RuntimeInner::{logger,authorization_provider,record_security_denial}`, `pub use async_trait`, `pub use ego_security_sdk as security`) — **KEEP**: intentional codegen visibility, not deprecated (Item 5).
- The testkit `KITLogger::log(Severity,&str)` back-compat path — **KEEP**: an external `kitlogger` API, not an ego-owned deprecated symbol (Item 6).
- The legacy flat `trace_id` mirror on `ServiceContext` — **KEEP**: not deprecated; authoritative-by-construction under `TraceContext` (PROD-003 ADR-4). Any change is a PROD-003 concern (Item 7).
- Introducing new execution/backend abstractions to replace the removed trait — there is no successor; the hot path already awaits handlers directly.
- Changing `is_cross_tenant_allowed_for` behavior — it already exists and is the target, untouched.

## Frozen Decisions

- **FD-1:** Every removal has **zero external callers** — all live references are in-repo tests/docs — so no deprecation window, alias, or re-export is created. Removal is immediate and total (per `PRD.md:140`).
- **FD-2:** `is_cross_tenant_allowed()` is removed, **not** kept: its deprecation note documents a security foot-gun (a permit for one destination authorizing another), and a safe destination-scoped replacement already exists.
- **FD-3:** Items 5-7 are **retained** with recorded justifications; they are not `#[deprecated]` and are not shims.
- **FD-4:** The no-shims policy is enforced by a `cargo test --workspace` participant (source-scan lint), not an unenforced shell script — mirroring `tenant_scoped_lint.rs`.

## Capabilities

### New Capabilities

- `api-surface-hygiene`: the cross-cutting "no shims in pre-stable crates" policy — zero-reference verification for removed APIs and a verifiable `#[deprecated]`-count-zero gate. Belongs to no single existing capability, so it is its own thin capability.

### Modified Capabilities

- `persistent-entity`: removes the `ExecutionBackend` trait and its `TokioExecutionBackend`/`SyncTestBackend` implementors from the public surface.
- `service-sdk`: removes `ServiceContext::is_cross_tenant_allowed()`; records the retention of the macro-visibility hatches and the legacy `trace_id` mirror.

## Approach

Delete the deprecated symbols and their files; migrate the four `is_cross_tenant_allowed()` test
call sites to the destination-scoped `is_cross_tenant_allowed_for(&TenantId)`; delete the orphaned
`#[allow(deprecated)]` attributes and the `COOKBOOK.md` deprecated mention. The **compiler is the
primary zero-reference proof** — any dangling reference fails `cargo build/test --workspace`. A grep
gate and a new source-scan lint test corroborate it and lock the policy so a future `#[deprecated]`
in a pre-stable crate fails the standard test run. Retentions (Items 5-7) get documented
justifications in the specs but no code change.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/persistent-entity/src/execution_backend.rs` | Delete | Remove `ExecutionBackend` trait file |
| `crates/persistent-entity/src/execution_backend_tokio.rs` | Delete | Remove `TokioExecutionBackend`/`SyncTestBackend` file |
| `crates/persistent-entity/src/lib.rs` | Modify | Drop `pub mod execution_backend;` + `pub mod execution_backend_tokio;` (`:40-41`) |
| `crates/service-sdk/src/context/mod.rs` | Modify | Delete `is_cross_tenant_allowed()` (`:339-348`); migrate 2 unit tests (`:551-575`) to `_for`; drop `#[allow(deprecated)]` |
| `crates/service-sdk/tests/smoke.rs` | Modify | Migrate `:210` to `_for`; drop `#[allow(deprecated)]` (`:203`) |
| `crates/service-sdk/tests/cross_tenant_access_contract.rs` | Modify | Migrate `:7` to `_for`; drop `#[allow(deprecated)]` (`:4`); rename test |
| `COOKBOOK.md` | Modify | Delete `is_cross_tenant_allowed()` deprecated mention (`:422`) |
| `crates/service-sdk/tests/no_deprecated_shims_lint.rs` | Create | Source-scan gate: `#[deprecated]` count == 0 in `0.x` crates only (per-crate `Cargo.toml` version gate; detector proven against inline fixtures) |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| An undiscovered external consumer references a removed symbol | Low | Reference scan shows zero non-test in-repo callers; crates are pre-stable (`0.1.0`), no stability promise; compile gate catches any missed ref |
| Test migration changes assertion semantics | Low | `is_cross_tenant_allowed_for(&dest)` returns the same truth for the tested cases (no-permit → false; matching-destination permit → true); destinations already in scope in every migrated test |
| Removing a file breaks an unnoticed `mod`/`use` path | Low | `cargo build --workspace` + grep gate = 0 references; `lib.rs` mod lines removed atomically with the files |
| Future re-introduction of a `#[deprecated]` shim | Med | `no_deprecated_shims_lint` fails the standard `cargo test --workspace` run |

## Rollback Plan

Pure deletion of dead/stub code plus mechanical test migration; no schema, data, or runtime-behavior
change. Rollback = `git revert` the change (restores the files, `pub mod` lines, `#[deprecated]`
methods, and original test call sites). No migration or feature flag needed — the removed symbols had
no live production callers, so behavior is byte-identical before and after.

## Dependencies

- Builds on CORE-008A (`is_cross_tenant_allowed_for` is the AD-008 destination-scoped replacement) — already merged.
- Builds on PROD-003 (legacy `trace_id` mirror retention rationale, ADR-4) — already merged.
- No dependency on other in-flight changes; no dedicated open issue.

## Success Criteria

- [ ] `rg '#\[deprecated' crates/` returns **0** matches.
- [ ] `rg '#\[allow\(deprecated\)\]' crates/` returns **0** matches.
- [ ] `rg 'TokioExecutionBackend|SyncTestBackend|ExecutionBackend|execution_backend' crates/` returns **0** matches.
- [ ] `rg 'is_cross_tenant_allowed\b' crates/ COOKBOOK.md` (excluding `_for`) returns **0** matches.
- [ ] `no_deprecated_shims_lint` passes as part of `cargo test --workspace`.
- [ ] `cargo build --workspace` and `cargo test --workspace` are green (compiler confirms zero dangling references).
- [ ] Retentions (Items 5-7) carry documented justifications in the specs; their code is unchanged.
