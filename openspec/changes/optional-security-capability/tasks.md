# Tasks: CORE-010B — Optional Security Capability

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 150-250 |
| 400-line budget risk | Low |
| Chained PRs recommended | No |
| Suggested split | Single PR |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: pending
400-line budget risk: Low

## Phase 1: Foundation — CapabilityNotEnabled Error (error/mod.rs)

- [x] 1.1 RED: Add `display_capability_not_enabled` + pattern-match tests for `SecurityError::CapabilityNotEnabled`
- [x] 1.2 GREEN: Add `CapabilityNotEnabled` variant to `SecurityError` enum in `error/mod.rs`

## Phase 2: Core — Authorization & Context

- [x] 2.1 RED: Update `none_security_returns_missing_context` test to expect `CapabilityNotEnabled` in `authorization/mod.rs`
- [x] 2.2 GREEN: Change `authorize_in_context` to return `CapabilityNotEnabled` (not `MissingContext`) on `None`
- [x] 2.3 RED: Update `none_security_returns_missing_context` test in `declarative_authz.rs` to expect `CapabilityNotEnabled`
- [x] 2.4 RED: Add unit tests for `require_security()` (Err/Ok) in `service-sdk/src/context/mod.rs`
- [x] 2.5 GREEN: Add `require_security()` method to `ServiceContext`

## Phase 3: Integration Wiring — RuntimeBuilder

- [x] 3.1 RED: Add tests for `RuntimeBuilder::new().build()` and `.with_security().build()` in `runtime/builder.rs`
- [x] 3.2 GREEN: Add `security_providers: Option<(Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>)>` to `RuntimeInner`
- [x] 3.3 GREEN: Create `runtime/builder.rs` with `RuntimeBuilder` (`.new()`, `.with_security()`, `.build()`) + `Runtime` public types
- [x] 3.4 GREEN: Export `Runtime` + `RuntimeBuilder` from `runtime/mod.rs`

## Phase 4: Verification

- [x] 4.1 Verify `cargo build --workspace` passes
- [x] 4.2 Verify `cargo test --workspace` passes
