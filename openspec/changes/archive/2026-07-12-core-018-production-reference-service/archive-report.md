# Archive Report: CORE-018 — Production Reference Service

**Status**: Archived  
**Archived at**: 2026-07-12  
**Final Verdict**: PASS (0 CRITICAL, 0 WARNING, 0 SUGGESTION)

## Executive Summary

CORE-018 is a complete, fully-verified, and merged production-reference dogfooding milestone. All 31 planned tasks were implemented across 3 chained PRs, independently verified, and merged into `develop` as:
- PR #155 (initial + review-round fixes, commit 3031739)
- PR #161 (tokio "time" feature fix, commit 9c3be8d)

The change delivers a reference implementation of tenant-scoped user registration (`RegisterUser` service operation) exercising `PersistentEntity`, the service-sdk guard chain, and observability recording via existing public APIs. `ego-transport` becomes a minimal, generic HTTP server mechanism; the reference-app extends with two entity aggregates, their integration tests, and a real end-to-end acceptance test.

**Additional fixes discovered and fixed post-verify, prior to archive:**
- **F-01 (HIGH)**: Fixed unbounded busy-loop on stop-channel drop in `TagSchedulerImpl::run_until_stopped` — `select!` was discarding `stop_signal.changed()`'s `Result`, leaving the loop running forever once the stop channel was dropped without sending `true`. Fixed by matching `Ok(true)`/`Ok(other)`/`Err` explicitly (commit 3031739).
- **F-02 (MEDIUM)**: Fixed silently-swallowed shutdown failures in `ReadSideRuntime::stop()` — task's `JoinError` was discarded, so panicked schedulers reported shutdown success. Fixed by changing `Runtime::register_async_teardown` and `shutdown_async` hook signature from `Future<Output = ()>` to `Future<Output = Result<(), RuntimeInfraError>>`, propagating the first hook error (commit 3031739).
- **F-03 (LOW, doc-only)**: Corrected stale doc comment describing an old shared-tag architecture in `ReadSideRuntime` (commit 3031739).
- **F-04 (HIGH, integration defect)**: `crates/runtime/Cargo.toml` was missing tokio's `"time"` feature under `[dependencies]` (only had it under `[dev-dependencies]`) — `ego-runtime`'s production code (`run_until_stopped`) uses `tokio::time::sleep`, so `cargo check -p ego-runtime` in isolation failed with `error[E0433]`, hidden by workspace-wide feature unification. Fixed by adding the feature (commit 9c3be8d, PR #161).

**Current state**: All 17 workspace crates individually isolation-checked clean (`cargo check -p <name>` for each). `cargo build --workspace` green, `cargo test --workspace` 976/0 passed.

## Artifacts and Observation IDs

All SDD artifacts are persisted in Engram and referenced here for traceability:

| Artifact | Type | Observation ID | Status |
|----------|------|---|--------|
| Proposal | architecture | 1210 | Complete |
| Spec (reference-service, http-transport combined) | architecture | 1211 | Complete |
| Design | architecture | 1212 | Complete |
| Tasks (31 tasks, 10 phases + traceability) | architecture | 1213 | Complete (all 31 checked) |
| Verify Report — PR1 (Phases 1-3) | architecture | 1215 | PASS |
| Verify Report — PR2 (Phases 4-5) | architecture | 1216 | PASS |
| Verify Report — Final (all 3 PRs, Phases 1-10) | architecture | 1217 | PASS (+ all 3 WARNINGs fixed pre-archive) |

## Specs Merged Into Living Source of Truth

Two new capabilities, no prior specs existed. Delta specs copied as full specs to living source:

| Delta Spec | Location | Action | Status |
|---|---|---|---|
| `specs/reference-service/spec.md` | `openspec/specs/reference-service/spec.md` | Created | ✅ |
| `specs/http-transport/spec.md` | `openspec/specs/http-transport/spec.md` | Created | ✅ |

Both specs define requirements, scenarios, and non-goals for the reference service (user registration, dual-write, observability) and HTTP transport (route, security extraction, error mapping) capabilities. All requirements are now backed by live implementation and test coverage.

## Post-Verify Fixes (Before Archive)

All fixes discovered during review-round code review and integration testing were applied before merge:

### F-01: Unbounded busy-loop on stop-channel drop (HIGH)

**File**: `crates/runtime/src/read_side/scheduler.rs`

**Issue**: `TagSchedulerImpl::run_until_stopped`'s `select!` macro was discarding the `Result` from `stop_signal.changed()`. When the stop channel was dropped without sending `true`, the channel would close and return `Err`, but the loop ignored this and continued running forever.

**Fix**: Explicitly match all three arms:
```rust
select! {
    Ok(true) => break,
    Ok(other) => { /* handle other values */ },
    Err(_) => break,
}
```

**Merged**: commit 3031739, PR #155.

### F-02: Silently-swallowed shutdown failures (MEDIUM)

**Files**: `examples/reference-app/src/read_side/mod.rs` (`ReadSideRuntime`), `crates/service-sdk/src/runtime/builder.rs` (`Runtime::register_async_teardown`/`shutdown_async`)

**Issue**: `ReadSideRuntime::stop()` called `self.shutdown_async()` and discarded the returned `JoinError`. If a registered async teardown hook panicked, the error was lost and `stop()` reported success.

**Fix**: Changed the async teardown hook signature from `Future<Output = ()>` to `Future<Output = Result<(), RuntimeInfraError>>`. The `shutdown_async()` method now propagates the first hook error, allowing callers to detect shutdown failures.

**Merged**: commit 3031739, PR #155.

### F-03: Stale doc comment (LOW)

**File**: `examples/reference-app/src/read_side/mod.rs` (ReadSideRuntime)

**Issue**: A doc comment described an old shared-tag architecture that no longer applied.

**Fix**: Removed/corrected the stale comment.

**Merged**: commit 3031739, PR #155.

### F-04: Missing tokio "time" feature (HIGH, integration defect)

**File**: `crates/runtime/Cargo.toml`

**Issue**: `ego-runtime`'s production code in `run_until_stopped` uses `tokio::time::sleep`. The `"time"` feature was declared only under `[dev-dependencies]`, not under `[dependencies]`. Running `cargo check -p ego-runtime` in isolation failed with `error[E0433]: cannot find function `tokio::time::sleep` in this scope`, but the error was hidden by workspace-wide feature unification in `cargo build --workspace`.

**Fix**: Added `"time"` to `tokio` under `[dependencies]`:
```toml
[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "sync", "macros", "time"] }
```

**Merged**: commit 9c3be8d, PR #161.

**Verification**: After the fix, `cargo check -p ego-runtime` passes, and all 17 workspace crates individually isolation-check clean.

## Implementation Summary

### Scope Delivered

| Area | Deliverable | Status |
|---|---|---|
| `crates/transport` | Real axum HTTP server mechanism (AppState, security extractor, error mapper, serve bootstrap) | ✅ Implemented |
| `examples/reference-app` | Two PersistentEntity aggregates (User, TenantOrganization), RegisterUser guarded service, HTTP route wiring, tests | ✅ Implemented |
| Observability | Test-double assertion pattern (CORE-012A) applied; no production adapter | ✅ Implemented |
| Guard Chain | #[authorize] + #[tenant_scoped] coverage with 3 test scenarios (unauthorized denied, cross-tenant denied, authorized ok) | ✅ Implemented |
| Non-Atomic Dual Write | Org-first sequencing, no compensation, benign orphan residue proven real | ✅ Implemented |
| E2E Test | Real axum::serve(), real HTTP client, real Hs256 JWT token | ✅ Implemented |

### Non-Goals (Confirmed Absent)

- No saga/compensation mechanism ✅
- No gRPC/tonic dependency ✅
- No production Observability adapter ✅

## Test Coverage (Final State)

| Layer | Test Count | Notable Tests |
|---|---|---|
| Unit | ~9 | User entity, TenantOrganization entity, error mapping table |
| Integration | ~12 | Guard chain (3 cases), partial-failure (1 case), observability (3 cases), HTTP route (4 cases) |
| E2E | 2 | Real axum server + HTTP client, with/without JWT |
| **Total** | **~23** | — |

**Test command**: `cargo test --workspace` — 976 passed / 0 failed / 0 measured.

## Files Modified

| File | Purpose | Status |
|---|---|---|
| `crates/transport/src/state.rs` | AppState type | New ✅ |
| `crates/transport/src/security.rs` | JWT security extractor | New ✅ |
| `crates/transport/src/error.rs` | ServiceError → StatusCode mapping | New ✅ |
| `crates/transport/src/server.rs` | axum::serve bootstrap + graceful shutdown | New ✅ |
| `crates/transport/src/lib.rs` | Module exports | Modified ✅ |
| `crates/transport/Cargo.toml` | Dependencies (ego-service-sdk, ego-security-sdk, async-trait) | Modified ✅ |
| `examples/reference-app/src/domain/user.rs` | User PersistentEntity | New ✅ |
| `examples/reference-app/src/domain/tenant_org.rs` | TenantOrganization PersistentEntity | New ✅ |
| `examples/reference-app/src/service.rs` | RegisterUser guarded service | New ✅ |
| `examples/reference-app/src/routes.rs` | HTTP route handler | New ✅ |
| `examples/reference-app/src/bin/server.rs` | Server main() | New ✅ |
| `examples/reference-app/src/lib.rs` | Runtime wiring + DEV_SIGNING_KEY fix | Modified ✅ |
| `examples/reference-app/tests/*.rs` | Guard chain, partial-failure, observability, HTTP route, E2E tests | New ✅ |
| `crates/testkit/src/fixtures.rs` | FixtureBuilder::with_observability | Modified ✅ |

## Closure Criteria Met

| Criterion | Status |
|---|---|
| All 31 tasks complete | ✅ Yes (all [x] in tasks.md) |
| Final verify: PASS with 0 CRITICAL | ✅ Yes (all 3 WARNINGs fixed pre-archive) |
| Specs merged to living source | ✅ Yes (reference-service, http-transport) |
| Change folder archived | ✅ Yes (2026-07-12-core-018-production-reference-service) |
| No stale implementation tasks remain | ✅ Yes (31/31 complete, 0 unchecked) |
| Post-review fixes included | ✅ Yes (F-01, F-02, F-03 from PR #155; F-04 from PR #161) |
| Isolation testing passed | ✅ Yes (all 17 crates individually check clean) |

## Merged Commits

- **PR #155** ("Production Reference Service" + "fix(core-018): unbounded busy-loop on stop-channel drop, and silently-swallowed shutdown failures")
  - Initial 3-PR chain implementation (Phases 1-10, 31 tasks)
  - Post-verify fixes F-01, F-02, F-03 (commit 3031739)
  
- **PR #161** ("fix(core-018): add tokio 'time' feature")
  - Post-merge integration defect fix F-04 (commit 9c3be8d)

Both PRs merged into `develop` branch.

## Governance

**Change Authority**: User (pablo.gore@renxo.com)  
**Archived by**: sdd-archive executor  
**Archive Decision**: All closure criteria met; no CRITICAL findings; all post-verify fixes applied; change is complete and ready for production integration.

---

*This archive report documents CORE-018's completion and closure. The change is now part of the permanent record in `openspec/changes/archive/`. For implementation details, refer to the proposal (#1210), design (#1212), and verify report (#1217) observations in Engram.*
