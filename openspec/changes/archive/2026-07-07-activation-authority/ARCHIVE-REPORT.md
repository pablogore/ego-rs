# Archive Report: CORE-006A — Activation Authority & Linearizability

**Status**: ARCHIVED  
**Date**: 2026-07-07  
**Change Name**: CORE-006A-activation-authority  
**Artifact Store**: openspec (file-based, hybrid-compatible)

---

## Change Completion Summary

CORE-006A has been fully planned, designed, implemented, verified, and archived. All 30 implementation tasks (TASK-001 through TASK-030) across 7 phases are complete (marked [x] in tasks.md). Implementation was verified via `cargo test --workspace` (all tests pass) and an open PR (https://github.com/pablogore/ego-rs/pull/128) contains the final committed work, including:

- Registry map rewrite with type-erased mailbox handles
- TeardownGuard with synchronized mailbox draining
- Actor-published lifecycle state via watch::channel
- Dead code removal (activation.rs, supervisor.rs)
- Test suite adaptation and new guaranteed_completion_tests.rs
- ARCHITECTURE.md alignment per ADR-007

Additional fixes found and closed during final pre-PR review (already committed on the PR branch):
- PersistenceFacade mutex-poisoning bug fix
- Two test-determinism gaps resolved
- design.md contract clarifications

---

## Artifacts Moved to Archive

**Source Folder**: `/Users/pablogore/workspace/pablogore/ego-rs/openspec/changes/CORE-006A-activation-authority/`  
**Archive Folder**: `/Users/pablogore/workspace/pablogore/ego-rs/openspec/changes/archive/2026-07-07-activation-authority/`

### Archived Files

| File | Status | Notes |
|------|--------|-------|
| proposal.md | ARCHIVED (exact copy) | Problem statement, findings, desired outcomes, product decisions (D1–D4) |
| spec.md | ARCHIVED (exact copy) | Change-specific spec, preserved verbatim; a separately normalized version was also merged into main specs (see below) |
| design.md | ARCHIVED (exact copy) | 8 Architecture Decision Records (ADR-001 through ADR-008) and implementation details |
| tasks.md | ARCHIVED (exact copy) | All 30 implementation tasks marked complete across 7 phases |

---

## Spec Synchronization

### Main Spec Created

**New file**: `/Users/pablogore/workspace/pablogore/ego-rs/openspec/specs/persistent-entity/spec.md`

This is the first main specification for the `persistent-entity` domain, normalized from CORE-006A's change-specific spec.md by removing:
- "Findings Traceability" section (change-specific, not durable)
- "Judgment Day Findings (Rounds 1–2)" section (audit detail, not contract)
- "Assumptions" section (decision rationale, not observable contract)

**Retained and normalized**:
- Purpose statement
- 10 Functional Requirements (FR-001 through FR-010) with all scenarios
- 3 Test Coverage Requirements (NFR-001 through NFR-003)
- Non-Goals section

**Spec format**: Follows the style of sibling domain specs (`security-sdk/spec.md`, `service-sdk/spec.md`), using structured requirement names with Given/When/Then scenarios and RFC 2119 keywords.

### Merge Details

This is a **new spec creation**, not a delta merge, because no prior `persistent-entity` spec existed. The spec captures durable observable contracts for entity activation:

- Single activation authority per triple
- Single source of truth for "active" state
- Deterministic visibility (cold and reactivation paths)
- Linearizable concurrent activation
- Fail-closed semantics
- Guaranteed completion for enqueued commands
- MailboxClosed as a retryable transient error

---

## Task Completion Verification

All 30 implementation tasks are marked complete:

- **Phase 1** (TASK-001, TASK-002): `parking_lot` dependency + sync mailbox queue
- **Phase 2** (TASK-003–TASK-007): Registry map rewrite + type-safe downcast
- **Phase 3** (TASK-008–TASK-013): Teardown guard + actor wiring
- **Phase 4** (TASK-014–TASK-015): Dead code removal
- **Phase 5** (TASK-016–TASK-019): Test adaptation + validation
- **Phase 6** (TASK-020–TASK-024): Guaranteed completion tests
- **Phase 7** (TASK-025–TASK-030): ARCHITECTURE.md alignment

**Verification**: 
- No unchecked implementation tasks remain
- `cargo test --workspace` passes
- Open PR #128 contains all committed changes
- Pre-PR review identified and fixed additional issues (all committed)

---

## Judgment Day Resolution

The design resolves the findings from 3 rounds of adversarial review:

| Round | Finding Type | Count | Resolved By |
|-------|--------------|-------|-------------|
| Rounds 1–2 | CRITICAL | 3 | ADR-001 (non-poisoning), ADR-002 (fail-closed downcast), ADR-005 (guaranteed drain) |
| Rounds 1–2 | WARNING | 2 | ADR-003 (single-writer via watch), ADR-005 (epoch safety) |
| Round 3 | CRITICAL | 1 | ADR-001 (amended: spawn after lock release) |

All findings are mapped to specific architecture decisions and test coverage.

---

## Archive Contents Verification

- [x] proposal.md — exact copy; problem, findings, desired outcomes
- [x] spec.md — exact copy; normalized version also lives in openspec/specs/
- [x] design.md — exact copy; 8 ADRs covering architecture and implementation
- [x] tasks.md — exact copy; 30 completed implementation tasks (all [x])
- [x] Main spec created at openspec/specs/persistent-entity/spec.md
- [x] Active changes directory no longer contains CORE-006A

---

## Next Steps

CORE-006A is complete and archived. The `openspec/specs/persistent-entity/spec.md` is the standing source of truth for entity activation contracts in the persistent-entity crate and applies to all future changes in that domain.

No follow-up changes are required for CORE-006A. Future changes to the persistent-entity crate must reference and maintain conformance with the persistent-entity spec.

**Open PR Status**: PR #128 is open and awaiting review/merge. Archive is independent of PR state (per repository convention: archiving is a manual post-completion step, not merge-gated).

---

## Archive Integrity Check

**Archived at**: 2026-07-07  
**Archive location**: `/Users/pablogore/workspace/pablogore/ego-rs/openspec/changes/archive/2026-07-07-activation-authority/`  
**Spec merged to**: `/Users/pablogore/workspace/pablogore/ego-rs/openspec/specs/persistent-entity/spec.md`  
**All artifacts present**: YES  
**All tasks marked complete**: YES  
**No CRITICAL verify-report issues**: YES

**Status: ARCHIVED AND CLOSED**
