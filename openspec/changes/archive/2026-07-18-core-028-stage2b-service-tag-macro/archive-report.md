# Archive Report: CORE-028 Stage 2B — Service→Tag Macro Link

**Archive Date**: 2026-07-18  
**Change**: core-028-stage2b-service-tag-macro  
**Status**: Archived, all gates passed, SDD cycle complete

## Executive Summary

CORE-028 Stage 2B (Service→Tag macro link, `.service::<S>()`) has been fully implemented, verified, and archived. The change enables macro-annotated service structs to register with a single generic parameter (eliminating the ceremony of supplying a coercion closure and explicit Tag type), while preserving permanent support for hand-rolled `Injectable` structs via the renamed `service_with_tag` method. Two chained PRs merged to `develop`; one CRITICAL review finding was identified and fixed; all verification passed with warnings noted as process/documentation lag only.

## Shipped Work

### Implementation (PRs #192 & #193)

- **PR #192** (`opsx/core-028-stage2b-pr1-service-tag-trait`, merged commit `2606ef01561de7bda22c2a8482d06b575e88518d`):
  - Added `HasServiceTag` trait to `crates/service-sdk/src/runtime/resolvable.rs` with associated `Tag` and `into_service` method
  - Extended `ServiceArgs` in macros crate to accept optional `impl_of` argument (comma-separated `key = value` parsing)
  - Generated `HasServiceTag` impl in `expand_service_struct` when `impl_of` present
  - Added guard in `expand_service_trait` to reject `impl_of` on trait annotations (commit `ffb443d`)
  - Created trybuild compile-fail and compile-pass fixtures validating the macro codegen
  - All Phase 1 & 2 tasks (traits, macros, unit tests, codegen) completed and green

- **PR #193** (`opsx/core-028-stage2b-pr2-appbuilder-service`, merged commit `2663d9d06c5d8d4bcb6cf85cd0e8f7c06c2be11c`):
  - Renamed existing `AppBuilder::service<S, Tag>(closure)` → `service_with_tag<S, Tag>(closure)` (permanent, non-deprecated)
  - Added new `AppBuilder::service<S>()` (single-generic form, no closure)
  - Migrated 4 in-tree call sites in `app_composition.rs` to the new form or renamed form as appropriate
  - Updated unit tests and scenarios; verified all 7 `app_composition.rs` tests pass
  - All Phase 3 & 4 tasks (AppBuilder wiring, call-site migration, docs check) completed and green

### Verification (verify-report.md)

**Verdict**: PASS WITH WARNINGS (0 CRITICAL, 2 WARNING, 0 SUGGESTION in final state)

- All 20 implementation tasks across Phases 1-4 marked complete and spot-checked against diff
- `cargo test -p ego-service-sdk-macros -p ego-service-sdk`: all green (171 unit tests + 7 integration tests + 2 trybuild)
- `cargo test --workspace`: green, no regressions
- `cargo clippy`: no new warnings
- All spec scenarios pass (bare service unaffected, macro-linked single-param registration, unlinked fails at compile, renamed form still works)

**Warnings** (process/documentation, not code defects):
1. Apply-progress artifact was saved before commit `ffb443d` (the task 2.8 gap-fix) was added; `tasks.md` is the canonical source
2. PR2 branch not rebased onto PR1's updated tip; GitHub diffs will recompute, merge test confirms no conflicts

### Review (receipt.json)

**Status**: Approved  
**Lens**: review-reliability (Medium risk, Standard tier)  
**Findings**:
- **RELIABILITY-001** (CRITICAL, deterministic): `#[service(impl_of=Trait)]` on trait annotation silently discarded → Fixed in commit `ffb443d` + scoped validator approved → Status: `fixed`
- **RELIABILITY-002** (WARNING): Path-qualified `impl_of` tested at string/AST level only, not cross-module → Status: `info` (non-blocking)

No open severe findings remain at archive time.

### Spec Compliance

Delta specs written for `application-composition` and `service-sdk` capabilities:

| Spec | Change | Details |
|------|--------|---------|
| `openspec/specs/application-composition/spec.md` | Modified | Enhanced "Service Registration Follows Injectable Contract" requirement with two registration forms (macro-linked primary, explicit-Tag permanent); updated non-goals |
| `openspec/specs/service-sdk/spec.md` | Added | New "Optional Struct-Macro Trait-Link Argument" requirement describing `impl_of` argument and `HasServiceTag` trait |

Both merged into main specs during archive phase.

## Artifacts

### Proposal & Design
- `proposal.md`: scope, intent, architecture decisions (AD-1 through AD-4), rollback plan
- `design.md`: technical approach, `HasServiceTag` trait shape, method signatures, `ServiceArgs` parsing, testing strategy

### Implementation Tracking
- `tasks.md`: 4 phases, 20 tasks (all checked)
  - Phase 1: Marker trait (1.1-1.2)
  - Phase 2: Macro codegen (2.1-2.8, including 2.8 added during implementation)
  - Phase 3: AppBuilder wiring (3.1-3.6)
  - Phase 4: Docs (4.1-4.3, all no-op)

### Verification & Review
- `verify-report.md`: full execution evidence, test results, spec matrix, design coherence, scope creep check
- `reviews/policy.md`: risk classification (Medium), lens selected (review-reliability)
- `reviews/receipt.json`: approved status, bound content (PR1 `ffb443d`, PR2 `3aa70f1`, base `cc31086`), no open severe findings
- `reviews/ledger.json`: findings ledger (RELIABILITY-001 fixed, RELIABILITY-002 info)
- `reviews/transaction.json`: review mode (openspec-mirror), correction applied (1 work unit), scoped validator approved
- `reviews/gate-context.json`: expected heads, genesis (review/start), gate instruction
- `reviews/chain-bundle.json`: review chain (start → findings → correction → scoped validation → approved)

### Delta Specs
- `specs/application-composition/spec.md`: requirement modification with 6 scenarios
- `specs/service-sdk/spec.md`: new requirement with 3 scenarios

All artifacts now in `openspec/changes/archive/2026-07-18-core-028-stage2b-service-tag-macro/`

## Engram Traceability

Archive report persisted with topic_key `sdd/core-028-stage2b-service-tag-macro/archive-report`. All prior artifacts preserved for full lineage:

| Artifact | Engram ID |
|----------|-----------|
| Proposal | 1281 |
| Design | 1283 |
| Spec (delta) | 1282 |
| Apply-progress | 1285 |
| Verify-report | 1288 |

## Roadmap Continuity

**Stage 2 Completion**: Stage 2A (projection registration, archived via PR #191) + Stage 2B (service macro link, archived here) = Stage 2 complete.

**Stage 2C**: Entity composition (`.entity::<E>()`) — **BLOCKED by CORE-006** (entity type contract awaiting Phase 1 of parallel workstream). No changes to blocking constraint; 2C remains dependent on CORE-006 completion.

**Next Steps**: Coordinate with CORE-006 team on entity type contract; once stable, 2C can proceed as a standalone Stage 2 slice following the same chained-PR pattern (Phase 1 trait/codegen, Phase 2 AppBuilder binding + migration).

## Rollback Boundary

The change is purely additive macro argument + method rename, with no runtime, DI-resolution, or stored-data changes. Rollback (if needed before 1.0): revert the macro argument, restore original `.service::<S, Tag>(closure)` name on AppBuilder, delete new trait, revert call sites. No deployment-time data migration required.

## Sign-Off

All SDD phase gates passed:
- ✅ Proposal reviewed and approved
- ✅ Spec written and delta specs prepared
- ✅ Design finalized with rationale
- ✅ Tasks tracked and all completed
- ✅ Implementation applied (PR #192 + PR #193 merged to develop)
- ✅ Verification passed with documented warnings (process, not code defects)
- ✅ Review approved (1 CRITICAL fixed, 1 WARNING info-level, no open severe)
- ✅ Archive complete, specs merged, artifacts moved to archive

**SDD cycle for core-028-stage2b-service-tag-macro: CLOSED**
