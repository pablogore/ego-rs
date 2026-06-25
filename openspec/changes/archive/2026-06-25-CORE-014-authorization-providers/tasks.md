# Tasks: CORE-014 Built-in Authorization Providers

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 120–160 |
| 400-line budget risk | Low |
| Chained PRs recommended | No |
| Suggested split | Single PR |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: pending
400-line budget risk: Low

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | All phases (stub removal → TDD cycles → wiring → docs → green) | PR 1 | Single PR; well under 400-line budget |

---

## Phase 1: Cleanup — Remove Private Stubs

- [x] 1.1 In `crates/security-sdk/src/authorization/mod.rs`, delete the private `AlwaysAllow` and `AlwaysDeny` struct definitions and their `#[async_trait] impl AuthorizationProvider` blocks (lines ~79–113).
- [x] 1.2 In the same file's `#[cfg(test)] mod tests`, replace all direct uses of `AlwaysAllow` / `AlwaysDeny` with temporary inline replacements or leave the seam stubs that test distinct behaviors (`DenyProvider`, `ErrorProvider`) — do NOT remove those.
- [x] 1.3 Run `cargo test --workspace` and confirm the workspace still compiles and all pre-existing tests pass (stubs were test-only; no production consumer). Satisfies precondition for TDD cycle.

## Phase 2: RED — AllowAll

- [x] 2.1 Create `crates/security-sdk/src/providers/allow_all/mod.rs` with: crate-level `//!` doc, `pub struct AllowAllAuthorizationProvider;` stub (no impl body yet), and an empty `#[async_trait] impl AuthorizationProvider` that panics or returns a compile error.
- [x] 2.2 Add inline `#[cfg(test)] mod tests` in that file with four failing / not-yet-compiling tests:
  - `allow_all_returns_allow_for_any_principal_and_request` (TS-014, `#[tokio::test]`)
  - `allow_all_is_send_sync` (TS-015, compile-time `_assert::<AllowAllAuthorizationProvider>()`)
  - `allow_all_arc_injectable` (FR-017 arc-safety, compile-time assignment to `Arc<dyn AuthorizationProvider>`)
  - `crate_root_reexport_compiles` (TS-019, deferred to Phase 6 — TS-019 confirmed RED via compile error)
- [x] 2.3 Confirm `cargo test --workspace` FAILS on TS-014 (RED gate). ✓ Compile error confirmed RED.

## Phase 3: GREEN — AllowAll

- [x] 3.1 Implement `#[async_trait] impl AuthorizationProvider for AllowAllAuthorizationProvider` returning `Ok(AuthorizationDecision::Allow)` ignoring all inputs.
- [x] 3.2 Add required `///` doc comment to `AllowAllAuthorizationProvider` stating dev/integration-test/demo-only intent (FR-020).
- [x] 3.3 Run `cargo test --workspace` — TS-014 and TS-015 MUST pass; TS-019 deferred to Phase 6. Confirm no regressions. ✓ PASSED.

## Phase 4: RED — DenyAll

- [x] 4.1 Create `crates/security-sdk/src/providers/deny_all/mod.rs` with: `pub struct DenyAllAuthorizationProvider;` stub and empty `#[async_trait] impl` skeleton.
- [x] 4.2 Add inline `#[cfg(test)] mod tests` with four tests:
  - `deny_all_returns_deny_for_any_principal_and_request` (TS-016, `#[tokio::test]`)
  - `deny_all_reason_is_deny_all` (TS-017, asserts `reason == "deny-all"`)
  - `deny_all_is_send_sync` (TS-018, compile-time `_assert::<DenyAllAuthorizationProvider>()`)
  - `deny_all_arc_injectable` (FR-018 arc-safety)
- [x] 4.3 Confirm `cargo test --workspace` FAILS on TS-016 (RED gate). ✓ TS-016 and TS-017 panicked with unimplemented!().

## Phase 5: GREEN — DenyAll

- [x] 5.1 Implement `#[async_trait] impl AuthorizationProvider for DenyAllAuthorizationProvider` returning `Ok(AuthorizationDecision::Deny { reason: "deny-all".to_string() })`.
- [x] 5.2 Add required `///` doc comment to `DenyAllAuthorizationProvider` stating lockdown/secure-by-default intent (FR-020).
- [x] 5.3 Run `cargo test --workspace` — TS-016, TS-017, TS-018 MUST pass. Confirm no regressions. ✓ PASSED.

## Phase 6: Wiring — Re-exports

- [x] 6.1 In `crates/security-sdk/src/providers/mod.rs`: added `pub mod allow_all; pub mod deny_all;` and `pub use allow_all::AllowAllAuthorizationProvider; pub use deny_all::DenyAllAuthorizationProvider;` in the same `pub use` block as `RbacProvider` (FR-019).
- [x] 6.2 In `crates/security-sdk/src/lib.rs`: extended `pub use providers::{...}` block to re-export both new types at crate root alongside `RbacProvider` (FR-019).
- [x] 6.3 Run `cargo test --workspace` — TS-019 (`crate_root_reexport_compiles`) now passes. All TS-014–TS-019 green. ✓ PASSED.

## Phase 7: Cleanup and Verification

- [x] 7.1 REFACTOR pass: reviewed `allow_all/mod.rs` and `deny_all/mod.rs` for idiomatic Rust style — implementations are trivially correct; no logic changes needed.
- [x] 7.2 Run `cargo doc --no-deps 2>&1 | grep warning` — confirmed zero missing-docs warnings (FR-020 / NFR-001 extension). ✓ ZERO WARNINGS.
- [x] 7.3 Run `cargo test --workspace` — full green; 79 security-sdk tests pass. ✓ PASSED.
- [x] 7.4 Confirmed private stubs `AlwaysAllow` and `AlwaysDeny` are absent from `authorization/mod.rs` — `rg "AlwaysAllow|AlwaysDeny" crates/security-sdk/` returned empty. ✓ CLEAN.
