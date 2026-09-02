# Archive Report: PROD-013 — Production Composition Hardening

**Date**: 2026-09-02  
**Change**: PROD-013 (2026-08-25 initial date)  
**Status**: ARCHIVED  
**All Tasks**: 48/48 complete (verified in tasks.md)  
**Verification**: PASS (per verify-report.md)  

## Executive Summary

PROD-013 successfully hardened production composition in ego-rs by introducing a `Profile` enum (Dev/Production) that gates event store, snapshot store, and effect store durability requirements at bootstrap. All 48 tasks across 7 work units are complete, all gates pass, and delta specs have been merged into the main specification suite. The change is now archived.

## Task Completion Gate

**Status**: PASS

All 48 implementation tasks are marked complete in the persisted artifacts:
- Phase 1 (1.1–1.7): Profile enum, shared predicate, durability signal
- Phase 2 (2.1–2.10): EntityRuntimeBuilder gates, try_build(), compatibility
- Phase 3 (3.1–3.8): Effect-store gate on RuntimeBuilder/AppBuilder
- Phase 4 (4.1–4.10): Reference app EntityEventStores wiring
- Phase 5 (5.1–5.3): Postgres block_in_place fix, test flavor migration
- Phase 6 (6.1–6.2): AD-10 regression guards
- Phase 7 (7.1–7.2): Documentation of persistence completeness rule and PROD-005 boundary
- Phase 8 (8.1–8.2 final gates): Cargo test and clippy pass (8.3 deferred by design)
- Phase 9 (9.1–9.3 final reconciliation): WU8 durability-capability check verification

## Spec Sync Decisions and Rationale

### 1. `production-composition-hardening/spec.md`

**Decision**: SKIP MERGE (already incorporated by PROD-014A)

**Rationale**: A more recent change, PROD-014A (archived 2026-09-02 at `openspec/changes/archive/2026-09-02-prod-014a-read-side-persistence-composition/`), manually pre-merged PROD-013's entire `production-composition-hardening` delta spec content into the main spec at `openspec/specs/production-composition-hardening/spec.md` and then layered its own read-side durable progress gate on top (the fourth governed capability). 

**Evidence**: The main spec now reads:
- "defines `Profile::Production` as an explicit opt-in gate that rejects bootstrap — with an actionable error — when any of the **four** composition-root-observable persistent capabilities (**event store, snapshot store, effect store, read-side durable progress**) lacks an explicitly configured durable implementation."

The opening header states: "Incorporates PROD-013 base spec and PROD-014A delta (read-side durable progress as a fourth governed capability)."

All three PROD-013 requirements (Explicit Profile Declaration, Event Store Gate, Snapshot Store Gate) plus the Effect Store Gate and Reference App Declaration requirements are already present verbatim or near-verbatim in the main spec, along with the identical scenario sets. Appending PROD-013's delta would duplicate that content exactly.

**Action**: No merge performed; delta remains in archive as historical record.

### 2. `application-composition/spec.md`

**Decision**: MERGE DELTA (3 new requirements added)

**Rationale**: No prior change had incorporated PROD-013's delta for application-composition. The main spec at `openspec/specs/application-composition/spec.md` describes the `App`/`AppBuilder` composition API (CORE-028 Stage 1) but lacked the PROD-013 requirements around Profile declaration and effect store gating at the composition root.

**Merged Requirements**:
1. **Profile Declaration At The Composition Root** — `RuntimeBuilder` and `AppBuilder` MUST accept `Profile` declaration (Dev default / Production)
2. **Effect Store Gate Under Production, Conditional On A Registered Executor, Surfaced Through CompositionError** — Under Production, when an executor is registered, composition MUST reject bootstrap without an effect store, naming the capability and the fix
3. **Reference App Propagates Its Profile From EntityEventStores, Guarded By A Regression Check** — `build_runtime_with` MUST propagate the profile from `EntityEventStores` rather than hardcoding it

**Action**: Appended to main spec after the existing "Duplicate Effect Retention Store Registration" requirement, maintaining spec structure and scenario conventions.

### 3. `persistent-entity/spec.md`

**Decision**: MERGE DELTA (3 new functional requirements added)

**Rationale**: No prior change had incorporated PROD-013's delta for persistent-entity. The main spec at `openspec/specs/persistent-entity/spec.md` defines entity activation authority and linearizability (FR-001 through FR-018) but lacked the PROD-013 requirements around Profile gating on the EntityRuntimeBuilder.

**Merged Requirements**:
1. **FR-019 — EntityRuntimeBuilder Gates In-Memory Fallback By Profile** — EntityRuntimeBuilder MUST accept Profile declaration; under Production, missing event/snapshot stores MUST reject (not silently fall back)
2. **FR-020 — Partial Event/Snapshot Configuration Under Production Is Covered By The Per-Capability Gates** — Partial configuration is rejected via per-capability gates, not a separate check
3. **FR-021 — Existing EntityRuntimeBuilder Call Sites Are Unaffected** — All 67 existing call sites MUST compile and pass unmodified

**Action**: Appended to main spec after FR-018, before "Test Coverage Requirements (NFR)", maintaining requirement naming and numbering conventions.

## Archive Contents Verification

- [ ] proposal.md ✅ (28,996 bytes)
- [ ] proposal.es.md ✅ (32,343 bytes)
- [ ] design.md ✅ (50,805 bytes)
- [ ] design.es.md ✅ (53,821 bytes)
- [ ] explore.md ✅ (15,788 bytes)
- [ ] tasks.md ✅ (21,890 bytes, 48/48 tasks complete)
- [ ] tasks.es.md ✅ (24,183 bytes)
- [ ] verify-report.md ✅ (27,292 bytes, status PASS)
- [ ] verify-report.es.md ✅ (29,526 bytes)
- [ ] specs/ directory ✅ (3 subdirectories)
  - production-composition-hardening/ ✅
  - application-composition/ ✅
  - persistent-entity/ ✅
- [ ] archive-report.md ✅ (this file)

## Merge Impact Summary

| Domain | Action | Details |
|--------|--------|---------|
| production-composition-hardening | Skipped | Already fully incorporated by PROD-014A (4 requirements total: 3 from PROD-013 + 1 from PROD-014A) |
| application-composition | Merged | 3 requirements added (Profile Declaration, Effect Store Gate, Reference App Profile Propagation) |
| persistent-entity | Merged | 3 requirements added (FR-019/FR-020/FR-021: Profile gating on EntityRuntimeBuilder) |

## Source of Truth Updated

The following main specs now reflect PROD-013's final requirements:
- `openspec/specs/application-composition/spec.md` — 3 new requirements appended
- `openspec/specs/persistent-entity/spec.md` — 3 new requirements appended (FR-019, FR-020, FR-021)
- `openspec/specs/production-composition-hardening/spec.md` — pre-merged by PROD-014A (no action needed)

## Final-State Authority

Per the SDD archive contract, the following rank sources from highest to lowest authority:

1. **Native review authority**: Not applicable; PROD-013 was implemented and verified under the prior orchestration model (not under receipt-driven review).
2. **Persisted tasks artifact**: `tasks.md` shows 48/48 tasks checked; verify-report.md shows PASS with all gates confirmed green. This is the authoritative completion state.
3. **Explicit final-state facts in launch prompt**: User confirmed "All 48/48 tasks... checked (the final two — `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` — were just run clean, zero failures/zero warnings, and committed)."
4. **verify-report and apply-progress snapshots**: verify-report.md (dated 2026-08-31, re-verified through WU8 on same date) records PASS; any intermediate snapshots are now superseded by the final archive state.

**Conclusion**: All work is complete and verified at archive time. No unresolved gaps or post-verification rework remain.

## Key Archive Decisions

1. **Production Composition Hardening spec**: Skipped merge to avoid duplication of requirements already incorporated by PROD-014A. The main spec is now the single source of truth for all four governed capabilities (event, snapshot, effect, read-side durable progress).

2. **Application Composition spec**: Merged 3 PROD-013 requirements covering Profile declaration, effect store gating, and reference app profile propagation. These complete the composition-root contract established by CORE-028 Stage 1.

3. **Persistent Entity spec**: Merged 3 new functional requirements (FR-019, FR-020, FR-021) covering Profile-gated fallbacks, partial configuration handling, and backward compatibility of the 67 existing call sites.

4. **Bilingual artifacts**: PROD-013 includes `.es.md` companions for proposal, design, tasks, and verify-report, consistent with the session-wide bilingual artifact convention. These are preserved in the archive for historical reference.

## Mechanical Copy Verification

Archive move performed via `git mv` with byte-identity verification:
- Source snapshot created before move: `/tmp/sdd-archive.XXXXXX/source`
- Target archived directory: `openspec/changes/archive/2026-09-02-prod-013-production-composition-hardening/`
- Diff readback: empty (zero differences)
- Source removal verified: ✅ (source no longer exists)

---

**Archive Completed**: 2026-09-02 (ISO format: YYYY-MM-DD)  
**SDD Cycle Status**: CLOSED  
**Change Ready for Deployment**: Yes
