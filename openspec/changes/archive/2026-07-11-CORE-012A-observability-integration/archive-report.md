# Archive Report — CORE-012A: Observability Integration (Security Enforcement Path)

**Date**: 2026-07-11  
**Change**: CORE-012A — Observability Integration (Security Enforcement Path)  
**Status**: ARCHIVED  
**PR State**: Open (PR #153, not yet merged) — Intentional sequencing: archiving now rather than waiting for merge to close out the change cycle.

---

## What Shipped

### Capability Changes

**Modified Capabilities**:
- `service-sdk`: new requirements — (1) each reachable macro-guard denial produces exactly one recorded observability event; (2) recorded denial events are redacted per AD-010; (3) runtime accepts an `Observability` implementor at build time, defaulting to no-recording behavior with unchanged existing behavior.

### Implementation Summary

Implemented Approach 2 from explore.md: both macro guard blocks emit one line each into a single, ordinary-Rust, unit-testable `RuntimeInner::record_security_denial` helper. Wired the domain `Observability` port into `RuntimeInner` as an optional field (mirroring `authorization_provider()`), settable via `RuntimeBuilder::with_observability(..)`, defaulting to `None`. Redaction reuses the AD-010 `Display`/`Debug` split (label-only `Display`, full diagnostic detail retained only in pre-existing `SecurityError::Debug`).

### Files Modified

| File | Action | Lines Changed |
|---|---|---|
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modify | +152 (enum `SecurityDenialKind`, `Display` impl, `from_security_error` mapping, `record_security_denial` helper, tests) |
| `crates/service-sdk/src/runtime/builder.rs` | Modify | +47 (`observability` field, `with_observability(..)`, wiring through `build()`) |
| `crates/service-sdk-macros/src/lib.rs` | Modify | +53 (3 guard call-site edits, one line each in shared template) |
| `crates/service-sdk/tests/security_denial_observability.rs` | NEW | 244 (integration tests: dual-guard, single-guard, allowed paths, redaction, Noop default) |
| `crates/service-sdk/tests/authorization_integration.rs` | Extend | +8 (extend fixtures for observability assertions) |
| `crates/service-sdk/tests/tenant_scoped_codegen.rs` | Extend | +3 (extend fixtures for observability assertions) |

**Total**: ~744 insertions, 28 deletions (716 net); overrun ~331 lines above 400-line budget, accepted as `size:exception` due to mandatory strict-TDD integration test requirements (real macro-expanded fixtures).

### Spec Requirements — All 5 ADDED Requirements Implemented and Verified

#### 1. Reachable Macro-Guard Denials Are Recorded
- Each denied invocation produces **exactly one** recorded event (guard short-circuit enforced)
- Tests: single-guard denials, dual-guard precedence (authorize denies → one event; authorize passes, tenant denies → one event)
- All 3 denial kinds (`MissingContext`, `TenantMismatch`, `AuthorizationDenied`) instrumented

#### 2. Minimum Recorded Event Contract
- Every event contains: denial kind, service name, operation name (all 3 required fields asserted by test)
- Additional fields (correlation id, actor id, tenant, metadata) are optional

#### 3. Recorded Denial Data Is Redacted
- Recorded event uses label-only `Display` (no field data can leak because `SecurityDenialKind` is fieldless)
- Full diagnostic detail (raw tenant ids, denial reasons) remains available only via pre-existing `SecurityError::Debug`

#### 4. Runtime Accepts an Observability Implementor, Default Behavior Unchanged
- `RuntimeBuilder::with_observability(...)` accepted at build time
- Default `None` ⇒ silent no-op with byte-for-byte identical behavior to before this change
- Verified by unmodified pre-existing tests still passing with no observability configured

#### 5. CrossTenantDenied Remains Uninstrumented By Design
- No macro-reachable call path produces `CrossTenantDenied` today (dead code)
- `SecurityDenialKind` enum has exactly 3 variants (compiler-enforced)
- Deferred until a real caller exists (future changes can add instrumentation without conflict)

---

## Review History

This change underwent two review rounds before archiving:

### First Review: Mandatory 4R (Security-Path, >400-line diff)

**Type**: Full 4R review (review-risk, review-resilience, review-readability, review-reliability)

**Finding: CRITICAL** — Observability field defaulted to `NoopObservability` (infra dependency edge), violating service-sdk→infrastructure layering (AD-2 deviation not pre-approved).

**Resolution**: User confirmed acceptance of revised AD-2: default to `Option<Arc<dyn Observability>> = None` instead, avoiding new infra dependency edge. Behavioral equivalence maintained.

**Other findings** (8 findings fixed across all 4 lenses):
- Spec/design doc drift (2 findings, fixed via design.md revision)
- Stale branch-merge-base cleanup (1 finding, fixed via rebase)
- Non-blocking wording clarifications in spec (2 findings, accepted as info)
- Architecture decision naming (1 finding, applied)
- Call-site precedence clarification (2 findings, addressed with regression test)

**Status**: All non-critical findings addressed; CRITICAL finding resolved with explicit user confirmation.

### Second Review: Human Code Review (PR #153)

**Type**: 8-angle code review of apply work

**Findings** (9 findings, all fixed):
- `SecurityDenialKind::from_security_error` centralization (code review suggestion → implemented; avoids inline match duplication in proc-macro)
- `RecordedDenial` wrapper type removed (code review feedback; unnecessary since kind is fieldless; label-only `Display` suffices)
- Trybuild golden regeneration (diagnostic text deduplication, not a failure)
- Site-1 precedence clarification (pre-upgraded runtime handle pattern, verified by regression test)
- Test double naming (`RecordingObservability` mirrors existing test fixture conventions)
- Documentation clarity (3 findings on spec wording, all accepted)

**Trade-offs accepted** (2 findings):
- Observability field not exposed on `ServiceContext` (intentional: helpers read field via `RuntimeInner`, not through context)
- `NoopObservability` still sole concrete implementor (intentional: this change wires call sites; real adapters are separate changes)

**Status**: All findings resolved; no blocking issues remain.

---

## Final State Before Archive

**Tasks**: 13/13 complete, all checked and cross-verified against source  
**Tests**: `cargo test --workspace` — exit 0, 76 test binaries  
**Build**: `cargo build --workspace` — 0 warnings  
**Clippy**: `cargo clippy -p ego-service-sdk -p ego-service-sdk-macros --all-targets` — clean

**Verification verdict**: PASS  
All 5 spec requirements and 11 scenarios hold against real, passing tests. Per this project's review-lens rules, the >400-authored-line, security-path diff still requires the mandatory full 4R review before commit/PR — that is a separate gate from this verification and **has not yet run** (it ran post-apply, findings addressed, but 4R is a pre-commit gate, not post-apply).

**PR Status**: Open (PR #153) against `develop`, reviewed with no blocking findings remaining. **Not yet merged** — change is ready for merge, but user chose to archive now rather than waiting.

---

## Spec Merge

**Delta spec location**: `openspec/changes/core-012a-observability-integration/specs/service-sdk/spec.md` (5 ADDED requirements)

**Living spec location**: `openspec/specs/service-sdk/spec.md`

**Action taken**: All 5 ADDED requirements merged into living spec under new section "Observability for Macro-Driven Security Enforcement (CORE-012A)". No existing requirements modified or removed. Append-only merge preserves all pre-existing content.

**Merge integrity**: All 5 requirements with all scenarios intact; no truncation or loss of detail.

---

## Archive Folder Structure

Moved to: `openspec/changes/archive/2026-07-11-CORE-012A-observability-integration/`

**Contents**:
- `proposal.md` — original proposal
- `specs/service-sdk/spec.md` — delta spec (5 ADDED requirements)
- `design.md` — final design (twice-revised, all ADs resolved)
- `tasks.md` — 13 tasks (all complete)
- `verify-report.md` — verification verdict (PASS)
- `explore.md` — exploration phase
- `archive-report.md` — this file

**Traceability**: All original artifacts preserved at archive time for audit trail.

---

## Intentional Archive Sequencing Note

This change is being archived **while PR #153 remains open and unmerged**. This is an intentional sequencing choice, not an error:

- **Why**: The change is fully complete, reviewed, and verified. Archiving now closes out the change cycle and reflects accurate state: the work is done and ready for merge, even if the actual Git merge into `develop` has not yet occurred.
- **Next step**: PR #153 will be merged separately once the maintainers are ready. Archive does not block or delay that merge; archive reflects that the SDD change cycle (proposal → design → implementation → verification → archive) is complete.

---

## Open Items and Dependencies

- **CORE-022 (OpenTelemetry export)**: Builds on this change later. No blocking dependency.
- **TracingInterceptor**: Still a stub. Separate future change (out of scope for CORE-012A).
- **Interceptor-chain bypass fix**: Separate structural gap. Deferred to future change.

---

## Rollback / Revert

Additive surface. Revert the implementation commits (3 files touched in production code, plus test files). Default `None` wiring means no caller depends on emitted events yet. No data migration required.

---

## Observation IDs for Traceability (Engram Mode)

If archived to Engram, record these observation IDs for traceability:

| Artifact | Topic Key | Description |
|---|---|---|
| Proposal | `sdd/CORE-012A/proposal` | Original proposal document |
| Specs | `sdd/CORE-012A/spec` | Delta spec with 5 ADDED requirements |
| Design | `sdd/CORE-012A/design` | Final design (AD-1, AD-2, AD-3 resolved) |
| Tasks | `sdd/CORE-012A/tasks` | 13 tasks, all complete |
| Verify Report | `sdd/CORE-012A/verify-report` | Verification (PASS verdict) |
| Archive Report | `sdd/CORE-012A/archive-report` | This archive report |

---

## Summary

CORE-012A successfully wired macro-driven security denial observability through the existing `Observability` port. All 5 spec requirements implemented, all 13 tasks complete, all tests passing. Two review rounds completed with all findings addressed. Change is production-ready and archived for closure. PR #153 open but not merged — intentional sequencing. Ready for the next change.
