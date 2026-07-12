# Verify Report: CORE-018 — Production Reference Service (FINAL, whole-change)

```yaml
schema: gentle-ai.verify-result/v1
evidence_revision: sha256:66c0cac682ff07fff7196a338afe087064b9908dd7ceee175e03c99f0f5e3
verdict: pass
blockers: 0
critical_findings: 0
requirements: 8/8
scenarios: 11/11 full
test_command: cargo test --workspace
test_exit_code: 0
build_command: cargo build --workspace
build_exit_code: 0
```

**Post-verify update (2026-07-12)**: all 3 WARNING findings from the FINAL
verify pass below (W-1, W-2, W-3) have been fixed prior to archive. See
"Warnings Resolved" at the end of this report for the exact fix per finding.
The narrative body below is left as originally written (the as-verified
state) for audit trail; the resolution section is authoritative for current
status.

## Verification Report

**Change**: core-018-production-reference-service
**Scope**: Final, whole-change verification across all 3 chained PRs (Phases 1-10, 31 tasks)
**Mode**: Strict TDD

### Completeness
| Metric | Value |
|--------|-------|
| Tasks total | 31 |
| Tasks complete | 31 |
| Tasks incomplete | 0 |

### Build & Tests Execution
**Build**: PASSED — `cargo build --workspace`, exit 0, clean.

**Tests**: PASSED — `cargo test --workspace`, exit 0, 974 tests passed / 0 failed / 0 measured (across lib + integration + doc tests). No `FAILED` lines anywhere in output. New/PR-specific binaries independently re-run and confirmed:

| Test binary | Result |
|---|---|
| `register_user_guard_chain.rs` | 3/3 |
| `register_user_partial_failure.rs` | 1/1 |
| `register_user_observability.rs` | 3/3 |
| `http_route.rs` | 2/2 |
| `e2e_register.rs` | 2/2 |
| `user_entity.rs` (PR2) | 3/3 |
| `tenant_org_entity.rs` (PR2) | 2/2 |
| `pipeline.rs` (pre-existing) | 4/4, unaffected by `build_runtime` signature change |

**Clippy**: `cargo clippy --workspace --all-targets` — zero warnings attributable to `crates/transport`, `crates/testkit`, or `examples/reference-app`. All emitted warnings are pre-existing debt in `service-sdk-macros`, `persistent-entity`, `service-sdk`, `security-jwt` (confirmed by file-path grep against clippy output, not just trusted from the apply report).

**Doc**: `cargo doc --workspace --no-deps` — exit 0. All broken-intra-doc-link warnings are pre-existing, in `persistent-entity`, `security-sdk`, `security-jwt`, `domain` — none in `reference-app`/`transport`/`testkit`.

**Coverage**: not available — no coverage tool detected in this workspace.

### Spec Compliance Matrix

**reference-service**

| Requirement | Scenario | Test | Result |
|---|---|---|---|
| PersistentEntity Contracts | Registering a user | `user_entity.rs > register_on_unregistered_produces_exactly_one_user_registered_event` | COMPLIANT |
| PersistentEntity Contracts | Ensuring a tenant org exists (incl. idempotent re-`Ensure`) | `tenant_org_entity.rs > ensure_on_absent_produces_organization_ensured_and_transitions_to_present`, `ensure_on_present_is_idempotent_and_produces_no_events` | COMPLIANT |
| Authorization/Tenant-Scoping | Unauthorized principal denied | `register_user_guard_chain.rs > unauthorized_principal_is_denied_and_no_entity_write_occurs` | COMPLIANT |
| Authorization/Tenant-Scoping | Cross-tenant request denied | `register_user_guard_chain.rs > cross_tenant_request_is_denied_and_no_entity_write_occurs` | COMPLIANT |
| Happy Path | Successful registration | `register_user_guard_chain.rs > successful_registration_returns_ok_output`, `http_route.rs > valid_bearer_jwt_and_body_returns_201`, `e2e_register.rs > ...registers_both_entities_end_to_end` | COMPLIANT |
| Non-Atomic Dual Write | TenantOrganization succeeds, User write fails | `register_user_partial_failure.rs > user_write_failure_leaves_org_persisted_as_a_benign_reusable_orphan` | COMPLIANT |
| Observability | Success and failure are observed | `register_user_observability.rs` (3 tests: success, partial-failure, guard-denial-for-free) | COMPLIANT |

**http-transport**

| Requirement | Scenario | Test | Result |
|---|---|---|---|
| HTTP Route Reaches RegisterUser | Request reaches the guarded operation | `http_route.rs`, `e2e_register.rs` | COMPLIANT |
| Security Context Extraction | Missing/invalid credentials rejected pre-invocation | `security_extractor.rs` (PR1), `http_route.rs > missing_authorization_header_returns_401...`, `e2e_register.rs > ...without_jwt_returns_401...` | COMPLIANT |
| Security Context Extraction | Valid credentials produce a SecurityContext | `security_extractor.rs` (PR1) | COMPLIANT |
| Success/Error Response Contract | Outcomes map to appropriate responses | `http_route.rs` (201 only), generic mapper table test `error.rs` (PR1) | **PARTIAL** — see finding W-1 |

**Compliance summary**: 10/11 scenarios fully compliant, 1/11 partial (W-1).

### Correctness (Static Evidence)

| Requirement | Status | Notes |
|---|---|---|
| `RegisterUser` uses CORE-025 canonical `with_service`/`resolve`, not `Injectable` (AD-7) | Implemented | `service.rs:108` `#[service(version="1.0.0")]`; `lib.rs:182` `.with_service::<RegisterUserTag>(...)`; `routes.rs:34` `state.runtime.resolve::<RegisterUserTag>()`. No `Injectable` impl exists for `RegisterUserImpl`. |
| Non-atomic dual write, org-first, no compensation (AD-5) | Implemented | `service.rs:156-203` — org `Ensure` sent and awaited first; on `User`-write `Err`, returns unmodified, no delete/rollback code anywhere. |
| Partial-failure trigger is real, non-vacuous | Implemented | `domain/user.rs:119-121` rejects empty/whitespace email as genuine validation (not a test-only hook); `register_user_partial_failure.rs` uses a tenant-scoped principal (not the default unscoped one) so the guard actually passes and the entity-write path is truly exercised, then proves the org survives as a **benign, reusable orphan** via a follow-up `Ensure` returning `NoEvents`, not just "org still exists". |
| HMAC signing-key length fix | Implemented, justified | See finding W-2. |
| Non-goals: saga/compensation/outbox | Confirmed absent | `rg -i "saga|compensat|outbox"` across `crates/transport` + `examples/reference-app` → 2 matches, both doc comments in `service.rs` describing the non-goal. |
| Non-goals: gRPC/tonic | Confirmed absent | No `tonic`/`grpc` reference in any `Cargo.toml` or `Cargo.lock` in the workspace. |
| Non-goals: no production Observability adapter | Confirmed absent | Only `Observability` implementors in the workspace: `NoopObservability` (`crates/infrastructure`), `RecordingObservability`/`PanickingObservability` test doubles (`service-sdk/test_support.rs`, `service-sdk/tests/common`, `register_user_observability.rs`). `build_runtime` wires `RegisterUserImpl::new(.., .., None)` — the real server has **no** observability sink at all; only tests attach one via `FixtureBuilder::with_observability`. |
| `testkit::with_observability` doesn't break existing consumers | Confirmed | Full `-p ego-testkit` test suite (visible in the `cargo test --workspace` run) passes, including all `fixtures.rs` unit tests exercising `with_service`/`resolve`/config pass-throughs unrelated to observability. |
| E2E test is a real server, not a mock | Confirmed | `e2e_register.rs` binds a real ephemeral `TcpListener`, spawns real `ego_transport::serve(...)`, opens a raw `TcpStream`, writes a literal HTTP/1.1 request, reads the raw response — no in-process/mock client. |

### Coherence (Design)

| Decision | Followed? | Notes |
|---|---|---|
| AD-1: Runtime in `State<Arc<Runtime>>`, resolve per-request | Yes | `routes.rs:32-35` |
| AD-2: `ego-transport` mechanism-only | Yes | Concrete route/handler lives in `reference-app/src/routes.rs`; transport exports only `AppState`/`AuthenticatedContext`/`TransportError`/`serve` |
| AD-3: Security via `FromRequestParts`, reusing `BearerExtractor` | Yes | Previously verified in PR1; unchanged |
| AD-4: Two independent `EntityRuntime`s, no shared event enum | Yes | `service.rs:117-121`, `lib.rs:176-177` |
| AD-5: Org-first, non-atomic, no compensation | Yes | See Correctness table above |
| AD-6: Aggregate shapes (`UserRegistered`, `OrganizationEnsured`, `Absent\|Present{name}`) | Yes | `domain/user.rs`, `domain/tenant_org.rs` match exactly |
| AD-7: Server lifecycle outside `RuntimeBuilder`, teardown order | Yes | `bin/server.rs:27-32` — `serve()` returns, then `rt.shutdown()` |

### Issues Found

**CRITICAL**: None.

**WARNING** (all 3 fixed prior to archive — see "Warnings Resolved" below):
- **W-1 — HTTP-level "Outcomes map to appropriate responses" scenario is only 1/3 proven at the HTTP layer.** `http_route.rs`/`e2e_register.rs` prove the 201 (success) and 401 (pre-invocation, extractor-level) outcomes through the real route, but neither the 403 (`RegisterUserError::Security` → `TransportError::Forbidden`, e.g. a cross-tenant call reaching the guard through the real HTTP route) nor the 500 (`RegisterUserError::EntityWrite` → `TransportError::Internal`, e.g. the empty-email partial-failure trigger through the real HTTP route) branch of `routes.rs::map_register_error` has any HTTP-level covering test. The underlying business behavior for both is separately proven at the service layer (`register_user_guard_chain.rs`, `register_user_partial_failure.rs`), and `map_register_error` is a trivial 2-arm match mirroring the already-proven generic mapper (`error.rs`) — so this is a coverage gap in the transport-mapping wiring specifically, not evidence of a behavioral defect. Recommend 2 additional `http_route.rs` cases (cross-tenant → 403, empty-email → 500) before or shortly after archive; not a blocking defect.
- **W-2 — HMAC signing-key comment states "51-byte" but the literal is 50 bytes.** `lib.rs`'s `DEV_SIGNING_KEY` doc comment says "Lengthened to a 51-byte... const"; `wc -c`/Python `len()` on the literal `b"reference-app-development-signing-key-not-for-prod"` returns 50. Functionally inconsequential — both figures clear `Hs256AuthenticationProvider`'s 32-byte NIST SP 800-107 floor — but the comment is factually off by one byte. Confirmed via `rg` that the previous 25-byte literal (`b"reference-app-signing-key"`) is referenced nowhere else in the workspace except this file's own doc comment describing the fix — no other test/example depended on the old key, so the fix is isolated and in-scope, triggered directly by this PR's new HTTP layer (no prior code path ever called `authenticate()`).
- **W-3 (cosmetic, carried) — `tasks.md`'s traceability table (line 240) still cites the pre-PR3 scenario name "Associating a user with a tenant org"**, while `specs/reference-service/spec.md` (fixed in this PR per PR2's recommendation) now titles it "Ensuring a tenant org exists". This is an internal cross-reference in `tasks.md`'s own notes, not user-facing spec text — the spec.md fix itself was independently re-read and confirmed correct (Absent/Present{name}, idempotent language, "reflects the new membership" reworded to "is `Present`"). No action required before archive; optional tidy-up.

**SUGGESTION**: None beyond the above.

### TDD Compliance
| Check | Result | Details |
|-------|--------|---------|
| TDD Evidence reported | Substantively yes (narrative, not tabular) | `apply-progress` describes real RED evidence (temporarily removing `service.rs` from the module tree to force genuine E0432/E0433 compile failures before restoring it) rather than a literal RED/GREEN/TRIANGULATE/SAFETY-NET table. `tasks.md` itself encodes RED/GREEN per task, which was cross-checked against live file existence and current passing state. Same standard applied consistently with the PR1/PR2 verify passes for this same change. |
| All tasks have tests | 31/31 code tasks correspond to an existing test file or a `[Verify only]` phase (Phase 10) | |
| RED confirmed (tests exist) | 20/20 new test files verified present and read in full | |
| GREEN confirmed (tests pass) | 20/20 new/modified test files re-run independently, all green | |
| Triangulation adequate | Adequate | guard chain (3 cases), observability (3 cases), entity tests (2-3 cases each); partial-failure intentionally single-case (the "benign orphan" proof), matching its own single spec scenario |
| Safety Net for modified files | Confirmed | `pipeline.rs` (4/4) unaffected by `build_runtime`'s additive signature change; PR2's `user_entity.rs`/`tenant_org_entity.rs` still pass unmodified except the one new, deliberate email-validation test |

**TDD Compliance**: 5/6 checks fully formal, 1/6 (evidence format) accepted as substantively equivalent — no fabricated or missing evidence found.

### Test Layer Distribution
| Layer | Tests | Files | Tools |
|-------|-------|-------|-------|
| Unit | ~9 | `user.rs`, `tenant_org.rs`, `error.rs` table tests | rustc/cargo test |
| Integration (in-process, `oneshot`/direct call) | ~12 | `register_user_guard_chain.rs`, `register_user_partial_failure.rs`, `register_user_observability.rs`, `http_route.rs` | `tower::ServiceExt`, `ego-testkit` |
| E2E (real socket) | 2 | `e2e_register.rs` | real `TcpListener`/`TcpStream`, real Hs256 JWT |
| **Total (this change, PR2+PR3)** | **~23** | 11 files | |

### Assertion Quality
No tautologies, ghost loops, or mock-heavy tests found across the new/modified test files read in full (`user_entity.rs`, `tenant_org_entity.rs`, `register_user_guard_chain.rs`, `register_user_partial_failure.rs`, `register_user_observability.rs`, `http_route.rs`, `e2e_register.rs`). Every assertion follows a real production-code call (entity command, guarded proxy invocation, real HTTP round-trip) and asserts a specific, non-trivial expected value (specific status codes, specific event names, `NoEvents` vs `Events` discrimination, specific output fields).

**Assertion quality**: All assertions verify real behavior.

### Quality Metrics
**Linter**: No new warnings attributable to `crates/transport`, `crates/testkit`, `examples/reference-app`.
**Type Checker**: N/A (Rust — covered by `cargo build`).

### Verdict
**PASS WITH WARNINGS** (as originally verified) — **all 3 warnings now fixed, see below.**
0 CRITICAL, 3 WARNING (1 real HTTP-coverage gap in error-mapping branches, 1 trivial off-by-one doc comment, 1 cosmetic stale cross-reference in tasks.md), 0 SUGGESTION. CORE-018 is functionally complete, all 31 tasks verified against live source, all tests pass on independent re-run, non-goals hold across the whole change, and the CORE-025 canonical service path is genuinely used. Ready for `sdd-archive`; W-1 is worth a follow-up test addition but is not archive-blocking since the underlying behavior is proven at the service layer and the mapping code is trivial and pattern-consistent with the already-proven generic mapper.

### Warnings Resolved (post-verify, pre-archive)

All 3 WARNING findings above have been fixed. Independently re-verified:
`cargo test -p reference-app` (all binaries green, `http_route.rs` now 4/4),
`cargo test --workspace` (976 passed / 0 failed, up from 974 — the 2 new
tests below), `cargo build --workspace` (exit 0), `cargo clippy -p
reference-app --all-targets` (zero warnings attributable to
`examples/reference-app`).

| Finding | Fix | File |
|---|---|---|
| W-1 | Added `cross_tenant_request_returns_403` and `empty_email_partial_failure_returns_500` to `http_route.rs`, driving `map_register_error`'s `Security`→403 and `EntityWrite`→500 arms through the real `tower::oneshot` HTTP route (same triggers as `register_user_guard_chain.rs`'s cross-tenant case and `register_user_partial_failure.rs`'s empty-email case, respectively, now proven at the HTTP layer too). RED-first sanity check performed: temporarily stubbed `map_register_error` to always return `BadRequest`, confirmed both new tests fail non-vacuously (400 vs expected 403/500), then reverted and confirmed GREEN against the real mapper. `http_route.rs` is now 4/4; "Outcomes map to appropriate responses" is now COMPLIANT (was PARTIAL). | `examples/reference-app/tests/http_route.rs` |
| W-2 | Reworded the `DEV_SIGNING_KEY` doc comment from the hardcoded "Lengthened to a 51-byte..." (which was off by one byte vs the actual 50-byte literal) to "Lengthened to well above that 32-byte floor" — accurate and no longer a byte count that can silently drift if the literal changes. | `examples/reference-app/src/lib.rs` |
| W-3 | Updated both stale occurrences of "Associating a user with a tenant org" (Phase 5's TASK-014 note and the traceability table) to "Ensuring a tenant org exists", matching `specs/reference-service/spec.md`'s already-corrected scenario title. | `openspec/changes/core-018-production-reference-service/tasks.md` (lines 125, 240) |

**Current verdict: PASS, 0 CRITICAL, 0 WARNING, 0 SUGGESTION.** Ready for `sdd-archive`.
