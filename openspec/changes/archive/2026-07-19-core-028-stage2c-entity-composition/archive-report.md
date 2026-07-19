# Archive Report: CORE-028 Stage 2C — Entity Composition (`.entity::<E>()`)

**Archive Date**: 2026-07-19  
**Change**: core-028-stage2c-entity-composition  
**Status**: Archived, all gates passed, SDD cycle complete

## Executive Summary

CORE-028 Stage 2C (Entity composition, `.entity::<E>()` registration on `AppBuilder`/`RuntimeBuilder`) has been fully implemented, verified, and archived. The change enables application developers to register host-constructed `EntityRuntime<E>` instances with fail-closed duplicate detection, keyed by aggregate type (not event type), making entity dependencies satisfiable through the same DI resolution path projections use. One PR merged to `develop` (commit `24216a9`); one post-PR review round fixed 2 issues (branch rebase, AD-1 identity test strengthening); all verification passed with zero CRITICAL/WARNING findings in final state.

## Shipped Work

### Implementation (PR #196)

- **PR #196** (`opsx/archive-core-028-stage2-slice-2c-entity-composition`, merged commit `24216a9`):
  - Added `DuplicateEntity` error type and `EntityRuntimeRef<E>` wrapper to `crates/service-sdk/src/di/mod.rs`
  - Implemented `RuntimeBuilder::with_entity<E>()` with fail-closed duplicate detection and aggregate-keyed TypeId
  - Added `DependencyTable` entity registration wiring and `resolve_entity<E>()` accessor
  - Flipped `check_dependency(DepKey::Entity)` from always-Err stub to real presence check
  - Implemented `AppBuilder::entity::<E>()` facade and `App::resolve_entity::<E>()` accessor
  - Added reference-app registration and dual-proof consumption tests (Phase 6)
  - Merged delta specs into living `openspec/specs/{application-composition,service-sdk}/spec.md`
  - All 37 tasks (Phases 1-7) completed and verified with `cargo test --workspace` (0 failures)
  - Post-merge review round (branch divergence rebase, AD-1 identity test `ptr_eq` strengthening) applied before closure

### Verification (verify-report.md)

**Verdict**: PASS (0 CRITICAL, 0 WARNING, 0 SUGGESTION in final state)

- All 37 implementation tasks across Phases 1-7 marked complete and executed in strict TDD (RED→GREEN per task)
- Implementation footprint: ~550 changed lines across 7 code files (di/mod.rs 70, app/mod.rs 104, app/error.rs 23, runtime/builder.rs 234, runtime/runtime_builder.rs 90, reference-app lib.rs 11, pipeline.rs 18)
- `cargo test -p ego-service-sdk`: 183 unit + integration tests, 0 failures
- `cargo test --workspace`: 0 failures across all crates
- All spec requirements verified satisfied:
  - AD-1 aggregate-keyed DI (TypeId keyed by `E`, never `E::Event`)
  - AD-2 host-constructed (framework constructs nothing, runtime passed by caller)
  - AD-3 `EntityRuntimeRef<E>` distinct from `EntityRef<E>` (persistent-entity's per-request handle)
  - AD-4 fail-closed duplicate (no replace hatch, first untouched)
  - AD-5 thin AppBuilder facade (clone-then-call + pending_error)
  - AD-6 zero persistent-entity lifecycle changes
  - AD-7 dual consumption proofs (NeedsEntity Injectable fixture + reference-app registration)
  - AD-8 `App::resolve_entity<E>()` accessor
  - AD-9 zero service-sdk-macros changes (macro trait-link deferred)
- Shared-event-type scenario verified (`two_aggregates_sharing_an_event_type_register_and_resolve_without_collision` test confirms both aggregates resolve independently despite shared `TestEvent`)
- `RegisteredDependencies` struct (code-quality fix during review) confirmed as signature-shape only, not scope deviation

### Review (4R + Post-PR Round)

**Initial 4R Review Status**: Approved with 5 WARNINGs found and fixed  
- 4 WARNINGs: code-quality/documentation lag (non-blocking)
- 1 WARNING: `RegisteredDependencies` struct shape change (confirmed as intentional fix, no AD deviation)

**Post-Merge Review Round**: 2 issues identified and fixed
1. Branch divergence rebase (PR branch not rebased onto upstream, GitHub diffs recomputed cleanly, merge test confirmed no conflicts)
2. AD-1 identity test strengthened (replaced assertion with `ptr_eq` to prove both aggregates share distinct runtime instances)

No open severe findings remain at archive time.

### Spec Compliance

Delta specs merged into main specs during apply phase (task 7.4):

| Spec | Change | Details |
|------|--------|---------|
| `openspec/specs/application-composition/spec.md` | Added | "Entity Runtime Registration Facade" requirement with 3 scenarios |
| `openspec/specs/application-composition/spec.md` | Added | "Duplicate Entity Registration Through AppBuilder Fails Closed" requirement with 1 scenario |
| `openspec/specs/application-composition/spec.md` | Modified | Non-Goals retired stale CORE-006-deferral bullet |
| `openspec/specs/service-sdk/spec.md` | Added | "Entity Runtime Registration Completes The Resolution Contract" requirement with 3 scenarios |
| `openspec/specs/service-sdk/spec.md` | Added | "Entity Identity Is Keyed By The Aggregate Type, Not Its Event Type" requirement with 2 scenarios |
| `openspec/specs/service-sdk/spec.md` | Added | "Duplicate Entity Registration Fails Closed" requirement with 2 scenarios |
| `openspec/specs/service-sdk/spec.md` | Added | "A Declared Entity Dependency Is Satisfiable At Build" requirement with 2 scenarios |
| `openspec/specs/service-sdk/spec.md` | Added | "App Exposes An Entity Resolution Accessor" requirement with 1 scenario |
| `openspec/specs/service-sdk/spec.md` | Modified | Non-Goals retired CORE-006-deferral bullet in Stage 2C scope |

Both specs merged into main specs **during apply phase, not during archive** — consistent with project convention (stage 2A/2B precedent).

## Artifacts

### Proposal & Design
- `proposal.md`: scope, intent, architecture decisions (AD-1 through AD-9), success criteria, rollback plan. Notes: CORE-006 deferral status corrected (CORE-006 and CORE-006A are shipped/archived; prior blocking status was stale). Out-of-scope: RegisterUserImpl migration off `.service_instance()`, EntityRuntime construction/lifecycle ownership, macro field-recognition for EntityRuntimeRef.
- `design.md`: technical approach, `EntityRuntimeRef<E>` wrapper shape, method signatures, bound stack for `E` and `E::Event`, testing strategy. 9 ADs with full rationale and code references.

### Implementation Tracking
- `tasks.md`: 7 phases, 37 tasks (all checked)
  - Phase 1: Error + Type (1.1-1.6)
  - Phase 2: `RuntimeBuilder::with_entity` (2.1-2.6)
  - Phase 3: `DependencyTable` + `resolve_entity` (3.1-3.8)
  - Phase 4: `Injectable` integration proof (4.1-4.4)
  - Phase 5: `AppBuilder::entity()` + `App::resolve_entity()` (5.1-5.5)
  - Phase 6: Reference-app reachability proof (6.1-6.4)
  - Phase 7: Wiring + verification (7.1-7.4, including spec merge)

### Verification & Review
- `verify-report.md`: full execution evidence, test results, spec matrix, design coherence, AD verification, scope creep check, RegisteredDependencies analysis
- 4R review and post-PR review details documented in initial review receipt and post-merge corrections

### Delta Specs
- `specs/application-composition/spec.md`: 2 new requirements + modified non-goals (5 scenarios)
- `specs/service-sdk/spec.md`: 5 new requirements + modified non-goals (11 scenarios)

All artifacts now in `openspec/changes/archive/2026-07-19-core-028-stage2c-entity-composition/`

## Engram Traceability

Archive report persisted with topic_key `sdd/core-028-stage2c-entity-composition/archive-report`. All prior artifacts preserved for full lineage:

| Artifact | Engram ID |
|----------|-----------|
| Proposal | 1294 |
| Design | 1296 |
| Spec (delta) | 1295 |
| Tasks | 1297 |
| Verify-report | 1299 |

## Roadmap Continuity

**Stage 2 Completion**: Stage 2A (projection registration, archived via PR #191) + Stage 2B (service macro link, archived via PR #194) + Stage 2C (entity composition, archived here via PR #196) = Stage 2 complete.

**Stage 2C Unblocking**: Proposal noted CORE-006 (persistent-entity runtime) and CORE-006A were already shipped/archived by this change date (June 22 archives). No blocking constraint remains.

**Stage 3 and Beyond**: No Stage 3 currently planned; Stage 2 is the final stage of CORE-028 application-composition roadmap per design.md scope.

**Next Steps**: CORE-028 is complete. Monitor for any Stage 3 feature requests or refinement issues against the unified application-composition API.

## Rollback Boundary

The change adds three production types (`DuplicateEntity`, `EntityRuntimeRef<E>`, new DI table field), one runtime check (`check_dependency` now presence-checks instead of always-Err), and two AppBuilder/App methods. Rollback (if needed before 1.0): delete the three types, restore always-Err check, remove `.entity()` and `resolve_entity` methods, drop `entities` map from DependencyTable, revert reference-app registration. No deployment-time data migration required.

## Sign-Off

All SDD phase gates passed:
- ✅ Proposal reviewed and approved
- ✅ Spec written and delta specs prepared
- ✅ Design finalized with 9 ADs and rationale
- ✅ Tasks tracked and all 37 completed
- ✅ Implementation applied (PR #196 merged to develop, post-merge review fixes applied)
- ✅ Verification passed (PASS, all 9 ADs verified, all scenarios tested, shared-event-type scenario explicitly proven)
- ✅ Review approved (5 WARNINGs found and fixed; post-PR round fixed 2 issues; zero CRITICAL/WARNING in final state)
- ✅ Archive complete, specs merged (task 7.4), artifacts moved to archive

**SDD cycle for core-028-stage2c-entity-composition: CLOSED**
