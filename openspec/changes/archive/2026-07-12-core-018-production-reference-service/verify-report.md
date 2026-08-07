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

---

## Addendum — 2026-08-08: verification downgraded to PARTIAL

**Verdict above is superseded. Current status: PARTIAL.**

This is an addendum rather than an edit. Everything above was true when written and
independently re-verified at the time; the record stands. What changed is the world,
not the report — the evidence was subsequently deleted from the repository.

`examples/reference-app/tests/e2e_register.rs` was removed. It violated CC-R11 (No
Infrastructure Dependency) and UT-R2 (No Real Infrastructure) by binding a real
socket, and `scripts/detect-integration-tests.sh` was failing on the workspace as a
whole. The coverage is tracked for reconstruction in a separate Testcontainers
workspace: **issue #275**, scope in `docs/integration-test-backlog.md`.

### What is still verified

`e2e_register.rs` was **co-evidence**, never sole evidence, for all three rows that
cite it. Each retains in-process coverage:

| Requirement | Surviving evidence |
|---|---|
| Happy Path — successful registration | `register_user_guard_chain.rs > successful_registration_returns_ok_output`, `http_route.rs > valid_bearer_jwt_and_body_returns_201` |
| HTTP Route Reaches RegisterUser | `http_route.rs` (4/4, via `tower::oneshot`) |
| Security Context Extraction — missing/invalid credentials rejected | `security_extractor.rs`, `http_route.rs > missing_authorization_header_returns_401…` |

### What is no longer verified

The proposal's named success criterion:

> "A real HTTP request against a running axum server completes registration
> end-to-end."

`http_route.rs` drives the router through `tower::oneshot` — in-process, no socket,
no HTTP client, no server task. That proves the route and its error mapping; it does
**not** prove the criterion above, and treating it as equivalent would be the exact
substitution `e2e_register.rs` existed to prevent. The 401-without-JWT and
register-both-entities paths over a real `axum::serve()` are unproven today.

**Report this change as: implemented and contractually tested; real HTTP transport
not verified.** Restoring PASS requires #275, not a re-reading of the tests that
remain.
