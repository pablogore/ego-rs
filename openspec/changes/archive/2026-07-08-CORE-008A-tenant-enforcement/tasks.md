# Tasks: CORE-008A — Canonical Tenant Model & Runtime Enforcement

## Completion Summary

All 34 tasks (TASK-001 through TASK-034) across 6 phases have been completed and merged to develop (commit 11830a5). This archive captures the final state after full implementation and verification.

## Review Workload Forecast (Original)

| Field | Value |
|-------|-------|
| Estimated changed lines | ~1300-1650 (completed) |
| Crates touched | `service-sdk`, `service-sdk-macros`, `security-sdk` + `openspec/specs/service-sdk/spec.md` |
| 400-line budget risk | High (addressed via chained PRs) |
| Chained PRs delivered | Yes (PR1–PR6, #133–#138) |
| Delivery strategy | feature-branch-chain |

## PR Chain Executed (Completed)

**Tracker branch**: `opsx/core-008a-tenant-enforcement` → merged to develop after PR 6 completion (commit 11830a5).

| PR | Branch | Tasks | Status |
|----|--------|-------|--------|
| 1 | `opsx/core-008a-tenant-enforcement-01-errors-canonical-type` | TASK-001–005 | Merged |
| 2 | `opsx/core-008a-tenant-enforcement-02-enforcement-path` | TASK-006–010 | Merged |
| 3 | `opsx/core-008a-tenant-enforcement-03-macro-tenant-scoped` | TASK-011–014 | Merged |
| 4 | `opsx/core-008a-tenant-enforcement-04-cross-tenant-issuance` | TASK-015–019 | Merged |
| 5 | `opsx/core-008a-tenant-enforcement-05-concurrency-deprecation-tests` | TASK-020–024 | Merged |
| 6 | `opsx/core-008a-tenant-enforcement-06-adopt-markers-acceptance-spec` | TASK-025–034 | Merged |

All merge gates passed; all phases green.

---

## Phase-by-Phase Completion Status

### Phase 1: Errors + Canonical Type
- [x] TASK-001: `SecurityError::TenantMismatch`, `SecurityError::CrossTenantDenied` added with redacted `Display`
- [x] TASK-002: Error variants integrated
- [x] TASK-003: `TenantResolver::resolve` 5-branch policy unit-tested
- [x] TASK-004: New `crates/service-sdk/src/runtime/tenant.rs` created with `CanonicalTenant`, `TenantEnforcementMode`, `TenantResolver`
- [x] TASK-005: Compile-fail test proving external code cannot construct `CanonicalTenant` directly

### Phase 2: Enforcement Path
- [x] TASK-006: `ServiceContext` canonical tenant fields and accessors tested
- [x] TASK-007: `resolved_tenant` field, `canonical_tenant()` accessor, `tenant_hint()` accessors, deprecated `tenant_id()`/`has_tenant()` implemented
- [x] TASK-008: `RuntimeInner::enforce_tenant(&mut ServiceContext) -> Result<(), SecurityError>` implementation tested
- [x] TASK-009: `enforce_tenant` implemented with resolver integration
- [x] TASK-009B: Macro unmarked-path signature fix (critical for workspace compilation)
- [x] TASK-010: `RuntimeBuilder::with_tenant_enforcement_mode(TenantEnforcementMode)` added

### Phase 3: Macro `#[tenant_scoped]`
- [x] TASK-011: Macro expansion tests for `#[tenant_scoped]` attribute
- [x] TASK-012: `#[tenant_scoped]` proc-macro attribute implemented
- [x] TASK-013: Regression tests confirm zero behavior change for unmarked operations
- [x] TASK-014: `tenant_scoped_lint.rs` automated detection runs in CI, zero violations workspace-wide

### Phase 4: Cross-Tenant Issuance
- [x] TASK-015: `issue_cross_tenant_permit` authorization-gating and destination-scoping tested
- [x] TASK-016: `CrossTenantPermit` dropped `Copy`, gained `{destination, issued_to}` fields
- [x] TASK-017: `issue_cross_tenant_permit` implemented with `AuthorizationProvider` capability check
- [x] TASK-018: All 4 call sites migrated to new signature
- [x] TASK-019: `with_cross_tenant_access` destination-scoped via `is_cross_tenant_allowed_for`

### Phase 5: Concurrency Tests
- [x] TASK-020: Two concurrent operations carrying different tenant hints tested
- [x] TASK-021: Retried calls under tenant resolution tested (idempotency)
- [x] TASK-022: `ServiceContext` clone behavior under tenant resolution tested
- [x] TASK-023: `CrossTenantPermit` non-reusability across destinations tested
- [x] TASK-024: Deprecated `tenant_id()`/`has_tenant()` compatibility tested

### Phase 6: Adoption + Acceptance Suite
- [x] TASK-025: New `crates/service-sdk/tests/tenant_enforcement_contract.rs` FR/NFR acceptance suite
- [x] TASK-026: Test service `TenantContractService` with `#[tenant_scoped]` operation implementation
- [x] TASK-027: FR-002/003/004 scenarios (Principal-derived, mismatch, neither authenticated nor internal)
- [x] TASK-028: FR-005/006 scenarios (permit denial, authorized cross-tenant)
  - Completed in two parts: the permit-denial scenario and the FR-005 test
    scaffolding landed at original archive (2026-07-08); FR-006's actual
    cross-tenant-grant consumption in `TenantResolver::resolve()` was only
    wired up by the follow-up PR #143 ("FR-006 cross tenant grant"),
    merged 2026-07-09, commit `ffbfbdd`. See `archive-report.md` for the
    full timeline.
- [x] TASK-029: FR-007 structural test (transport-independent runtime)
- [x] TASK-030: FR-008 scenario (single canonical tenant convergence)
- [x] TASK-031: FR-009/010/014 scenarios (fallible enforcement, no parallel authority, immutability)
- [x] TASK-032: FR-011/012/NFR-003 scenarios (canonical presence, distinguishable errors)
- [x] TASK-033: `openspec/specs/service-sdk/spec.md` delta applied (lines 76-92, INV-003 437-443)
- [x] TASK-034: Full workspace verification, migration inventory sweeps, rg confirmations passed

---

## Implementation Deviations (Completed & Flagged)

| Item | Deviation | Resolution |
|------|-----------|-----------|
| `CanonicalTenant` shape | Opaque wrapper instead of plain public enum | Required to enforce AD-003: only `TenantResolver` can create (Rust visibility rules); mirrors `CrossTenantPermit` pattern |
| `TenantId::to_string()` | Needed `as_str().to_string()` (no Display impl) | Applied without behavior change |
| `tenant_scoped_lint` false negatives | Identifier-name heuristic only (direct references) | Accepted per AD-007; indirect paths flagged as future secure-by-default flip |

---

## Success Criteria (Archive View)

- [x] Spec defines contracts without inheriting implementation details
- [x] Every finding (1–10) maps to at least one requirement
- [x] All product decisions (D1–D6) appear as requirements or ADs
- [x] `design.md` contains Transport-independent Tenant Resolution AD and answers to all 9 Open Questions
- [x] `openspec/specs/service-sdk/spec.md:76` and INV-003 describe enforced (not aspirational) behavior
- [x] Rejection paths have dedicated test coverage
- [x] Authorized cross-tenant path has dedicated positive-path coverage
- [x] `ServiceContext` no longer a parallel writable authority on authenticated path
- [x] Tenant enforcement failure aborts before operation body execution
- [x] All tests passing, workspace clean, verification gates passed

---

## Artifacts & References

**Key Implementation Files:**
- `crates/service-sdk/src/runtime/tenant.rs` — `CanonicalTenant`, `TenantResolver`, `TenantEnforcementMode`
- `crates/service-sdk/src/context/mod.rs` — `canonical_tenant()`, `tenant_hint()`, resolved_tenant field
- `crates/service-sdk-macros/src/lib.rs` — `#[tenant_scoped]` attribute macro, fallible `enforce_tenant` emission
- `crates/security-sdk/src/error/mod.rs` — `TenantMismatch`, `CrossTenantDenied` error variants
- `openspec/specs/service-sdk/spec.md` — Lines 76-92, INV-003 at 437-443 (delta applied)

**Test Coverage:**
- `crates/service-sdk/tests/tenant_enforcement_contract.rs` — 11 FR/NFR acceptance tests
- `crates/service-sdk/tests/tenant_scoped_lint.rs` — Automated detection + structural test (FR-007)
- `crates/service-sdk/tests/tenant_enforcement_concurrency.rs` — Concurrency/idempotency/clone tests
- All inline unit tests in `runtime/tenant.rs`, `context/mod.rs`, `runtime_builder.rs`, `permit.rs`

**Verification Report:**
- Engram topic: `sdd/core-008a-tenant-enforcement/verify-report` (0 CRITICAL, 2 disclosed WARNINGs, 2 SUGGESTIONs)
- Apply Progress: Engram topic: `sdd/core-008a-tenant-enforcement/apply-progress` (all 6 phases complete)

---

## Delivery Timeline

- Proposal: 2026-07-07 (Product decisions D1–D6 locked)
- Specification: Concurrent with proposal (14 FR + 3 NFR + delta)
- Design: Concurrent with spec (13 ADs, implementation choices resolved)
- Tasks: Concurrent with design (34 tasks, 6 phases, chaining strategy)
- Apply: 2026-07-07 through 2026-07-08 (PR1–PR6 merged in sequence)
- Verify: 2026-07-08 (commit 11830a5, all gates passed)
- Archive: 2026-07-07 (this document; change folder moved to archive, spec delta merged to main spec)

---

## Notes for Future Reference

1. **Automatic detection covers direct references only** — operations touching tenant data indirectly (e.g., through repository filters) may not be flagged. The secure-by-default flip (opt-out `#[system]`) is the long-term fix.
2. **Deprecated accessors remain functional during migration** — `tenant_id()`/`has_tenant()` are deprecated but working aliases of `tenant_hint()`. Their removal is a scoped follow-up.
3. **`RuntimeExecutionContext` intentionally deferred** — convergence of this fifth tenant carrier is orthogonal to this change and scoped separately.
4. **Per-call `TenantId` re-allocation** — `TenantResolver::resolve` re-validates and re-allocates `TenantId`/`String` from `Principal.tenant_id` on every call. This is noted as a perf optimization opportunity (ego-rs#139) but not fixed here due to cross-crate implications.

All work is complete. The change is merged to develop and ready for production use.
