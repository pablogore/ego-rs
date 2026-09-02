# Archive Report: PROD-014B — PostgreSQL Durable Read-Side Stores

**Change**: `prod-014b-postgresql-durable-read-side-stores`
**Archived to**: `openspec/changes/archive/2026-09-02-prod-014b-postgresql-durable-read-side-stores/`
**Archive Date**: 2026-09-02
**Status**: Complete — All 24 tasks verified complete, 13 spec requirements traced to merged code with passing tests, all gates green.

## Verification Summary

Per the `sdd-verify` run against develop@df10396 (PR3's merge commit):

- **Tasks**: 24/24 complete (`[x]`)
- **Spec Requirements**: 13/13 traced to merged code with passing tests
- **Build Gates**: Clean (zero warnings with clippy -D warnings)
- **Test Gates**: Zero failures (cargo test --workspace)
- **Conformance Suite**: 65/65 PostgreSQL conformance tests passing (integration-tests)
- **Design Fidelity**: AD-1 through AD-10 confirmed, EC-2 confirmed
- **Wording Constraint**: No exactly-once implication detected anywhere; PROD-014C and F-2 correctly named as distinct follow-ups
- **Scope Boundary**: Confirmed no SPI change, no retention/atomic-claiming/multi-replica code, in-memory/fake-durable pairs preserved

## Specs Merged

### Domain: `read-side`

**Capability: `read-side` (MODIFIED)**
- **Action**: ADDED 3 new requirements to existing capability
  1. Durable Dedup Bookkeeping Does Not Imply Exactly-Once Handler Execution
  2. Prevention of Double Handler Execution Rests on an Explicit, Unenforced Single-Writer Adoption Constraint
  3. The Concurrency Gap Has a Named, Distinct Follow-Up

**Capability: `read-side-durable-progress` (NEW)**
- **Action**: ADDED entire capability with 10 requirements
  1. Offset Survives a Process Restart
  2. Absent Offset Reads Are Tenant-Isolated
  3. Repeated Dedup Marks Converge to One Record
  4. Dedup Identity Is Tenant-Independent
  5. Offset Writes Are Last-Write-Wins
  6. Both Progress Stores Report Themselves As Durable
  7. Tenant Is a Required Part of Offset Identity
  8. Dedup Storage Growth Is Unbounded In This Capability
  9. The Reference Application's Production Path Uses the Durable Pair
  10. The Single-Writer Adoption Constraint Is Documented at the Adapter Level

**Merged Spec Files**:
- `openspec/specs/read-side/spec.md` — 429 lines (was 214, now includes both CORE-026 + PROD-014B)
- `openspec/specs/read-side/spec.es.md` — 429 lines (NEW bilingual companion)

## Archive Contents

The change folder has been moved to `openspec/changes/archive/2026-09-02-prod-014b-postgresql-durable-read-side-stores/` with 9 files:

- `proposal.md` — Original proposal (English)
- `proposal.es.md` — Original proposal (Spanish companion)
- `spec.md` — Delta spec (English) — archived for reference; merged content is now in `openspec/specs/read-side/spec.md`
- `spec.es.md` — Delta spec (Spanish) — archived for reference; merged content is now in `openspec/specs/read-side/spec.es.md`
- `design.md` — Design decisions (AD-1 through AD-10, EC-2)
- `design.es.md` — Design decisions (Spanish companion)
- `explore.md` — Exploration notes
- `tasks.md` — Task checklist — 24/24 tasks marked complete
- `tasks.es.md` — Task checklist (Spanish companion)

## Traceability

**3-PR Stacked Chain (all merged into develop)**:
- PR1 (#382, merge commit `7030272`) — Schema foundation (migrations 013, 014 + registration)
- PR2 (#383, merge commit `063ca3f`) — Durable adapters + error mapping + conformance tests
- PR3 (#384, merge commit `df10396`) — Production adoption (reference-app wiring + docs)

**13 Spec Requirements → Implementing Code**:
All requirements traced in `tasks.md`'s Traceability Audit section. Every requirement has ≥1 covering task, every task maps to merged code lines.

**Non-Goals Scope Boundary**:
Zero findings: no dedup retention, no atomic claiming, no multi-replica code, no backend other than PostgreSQL. All reserved for PROD-014C or backlog (F-2).

## SDD Cycle Status

- **Proposal**: ✓ Completed and archived
- **Spec**: ✓ Completed and merged into main specs; delta archived
- **Design**: ✓ Completed and archived
- **Tasks**: ✓ 24/24 complete and marked in persisted artifact
- **Implementation**: ✓ 3 PRs shipped and merged into develop
- **Verification**: ✓ PASS — all 24 tasks, 13 spec requirements, all gates green
- **Archive**: ✓ Completed — change folder moved, specs merged, untracked docs committed

## Notes

**Bilingual Artifacts**: Per project convention, both English (spec.md) and Spanish (spec.es.md) versions were merged into main specs. Delta specs remain in archive for traceability.

**Untracked Docs Committed**: All 7 untracked openspec docs (design.md, design.es.md, explore.md, proposal.md, proposal.es.md, spec.md, spec.es.md) were staged as part of this archive operation before the change folder was moved.

**Task Completion Gate**: Persisted tasks.md shows no unchecked implementation tasks. Archive proceeds under full verification authority.

## Final State

The change has transitioned from active development to archived — the SDD cycle is complete. The durable PostgreSQL read-side progress pair is now integrated into the reference application's production composition path, with all adoption constraints documented at the adapter level and follow-up concurrency work (PROD-014C) explicitly named and reserved.
