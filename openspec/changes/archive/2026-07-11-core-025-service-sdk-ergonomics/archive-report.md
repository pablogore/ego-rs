# Archive Report: CORE-025 — Service SDK Developer Ergonomics

**Archived**: 2026-07-11  
**Change**: Service SDK Developer Ergonomics (CORE-025)  
**Status**: COMPLETE — All 22 tasks delivered, 946 tests passing, verify-report PASS WITH WARNINGS (0 CRITICAL; 1 accepted structural scenario, plus pre-existing clippy/doc debt unrelated to this change — see Warnings below)

---

## Executive Summary

CORE-025 successfully completed the wiring of existing-but-dormant `ServiceRegistry` / `Resolvable` / `Injectable` machinery into the public `RuntimeBuilder` and `Runtime` surface, eliminating hand-rolled proxy construction and enabling fail-fast dependency validation at `build()`/`try_build()` time. The change is fully backward-compatible (hand-rolled `{Trait}Ref::new()` remains untouched) and preserves all security/tenant enforcement invariants through byte-identical proxy generation.

**Delta specs merged into living specs:**
- `openspec/specs/service-sdk/spec.md` — 5 new requirements added (Canonical Service Registration, Canonical Service Resolution, Fail-Fast Dependency Validation, Diagnosable Dependency Error, `{Trait}Ref::new` Escape Hatch)
- `openspec/specs/testkit/spec.md` — 1 new requirement added (TestKit Trait-Proxy Registration and Resolution Use the Canonical Production Path)

**All artifacts preserved in archive:**
- proposal.md — problem statement and scope
- design.md — architectural decisions (AD-1 through AD-7)
- explore.md — Phase 1 ergonomics audit with evidence-based findings
- tasks.md — 22 complete tasks across 10 phases, all marked `[x]`
- specs/service-sdk/spec.md — delta spec, now merged to living spec
- specs/testkit/spec.md — delta spec, now merged to living spec

---

## Verification Summary

**Build & Tests:**
- `cargo build --workspace` — PASS (0 errors, 0 warnings on scoped ego-service-sdk clippy)
- `cargo test --workspace` — PASS (946 tests, 0 failed)

**Verification Report:**
- 22/22 tasks complete (100%)
- 6/6 requirements compliant (100%)
- 15/16 scenarios fully compliant (93.75%)
- 1/16 scenario PARTIAL — "Registration and resolution can never disagree on version" accepted as a structural guarantee (neither `with_service` nor `resolve` accepts a version param), not a dedicated runtime test

**Warnings (all explicitly accepted, out-of-scope):**
1. Version-agreement scenario — "Registration and resolution can never disagree on version" accepted as an API-shape guarantee, no dedicated test needed.
2. `cargo clippy -p ego-service-sdk --all-targets` (plain): 4 pre-existing warnings, 0 new — `clippy::collapsible_match` in `crates/service-sdk-macros/src/lib.rs:677` (CORE-012A, unrelated), `clippy::derivable_impls` in `config_provider.rs:46` (unrelated), `clippy::too_many_arguments` in `runtime_builder.rs:234` (CORE-012A's `new_with_logger`, unrelated), `clippy::bool_assert_comparison` in `tenant_enforcement_contract.rs:157` (unrelated test file). Left untouched per CORE-021 closure convention (no inline pre-existing debt fixes).
3. `cargo doc --workspace --no-deps`: 1 pre-existing warning — unresolved intra-doc link in `runtime/builder.rs:32` (predates CORE-025 by 2 days, commit `3c0c057`), unrelated.

---

## Traceability: Engram Observations

| Artifact | Observation ID | Type | Note |
|---|---|---|---|
| Proposal Decision | #1187 | decision | CORE-025 proposal: 3 final adjustments before design.md |
| Spec Document | #1189 | decision | sdd/core-025-service-sdk-ergonomics/spec — delta specs (service-sdk, testkit) |
| Design Document | #1188 | decision | CORE-025 design.md: validate() standing invariant + resolve() clarifications |
| Tasks Breakdown | #1190 | decision | sdd/core-025-service-sdk-ergonomics/tasks — 22 tasks across 10 phases |
| Verify Report | #1194 | architecture | sdd/core-025-service-sdk-ergonomics/verify-report — PASS WITH WARNINGS (0 CRITICAL, 2 WARNING) |
| Apply Progress | #1191 | architecture | sdd/core-025-service-sdk-ergonomics/apply-progress — TDD cycle evidence reconstructed (per-task RED/GREEN table) |
| Archive Report | (this document) | architecture | sdd/core-025-service-sdk-ergonomics/archive-report — closure record with spec merges and traceability |

---

## Specs Merged into Living Specifications

### openspec/specs/service-sdk/spec.md

**Requirements Added (5):**
1. **Canonical Service Registration** — `RuntimeBuilder::with_service::<Tag>(Arc<Tag::Service>)` with 3 scenarios
2. **Canonical Service Resolution Yields the Concrete Generated Proxy** — `Runtime::resolve::<Tag>()` with 3 scenarios
3. **Fail-Fast Dependency Validation at `try_build()`** — `with_injectable` + `try_build()` with 4 scenarios
4. **Diagnosable Dependency Error** — `RuntimeError::DependencyNotFound { type_name, service_name }` with 2 scenarios
5. **`{Trait}Ref::new` Escape Hatch Remains Supported** — backward-compatibility guarantee with 1 scenario

**Total new scenarios:** 13 (all requirements-level + scenario-level acceptance criteria preserved)

**Merge method:** Additive insertion before the "Tenant Enforcement & Cross-Tenant Access (CORE-008A)" section, preserving all existing requirements unchanged.

### openspec/specs/testkit/spec.md

**Requirements Added (1):**
1. **TestKit Trait-Proxy Registration and Resolution Use the Canonical Production Path** — `FixtureBuilder::with_service` + `ServiceTestFixture::resolve` with 3 scenarios

**Total new scenarios:** 3

**Merge method:** Additive insertion before the "Out of Scope" section, preserving all existing requirements unchanged.

---

## Change Summary by Phase

| Phase | Tasks | Mechanism | Status |
|---|---|---|---|
| 1 | TASK-001 to TASK-004 | `RuntimeError::DependencyNotFound` struct variant + Display/Error impls + call-site fixes | ✓ Complete |
| 2 | TASK-005 to TASK-008 | `DepKey` type-name field + macro codegen + snapshot regeneration | ✓ Complete |
| 3 | TASK-009, TASK-010 | `Injectable::validate()` + `RuntimeInner::check_dependency()` | ✓ Complete |
| 4 | TASK-011, TASK-012 | `Resolvable::Service` assoc type + macro codegen | ✓ Complete |
| 5 | TASK-013, TASK-014 | `RuntimeBuilder::with_service()` + `Runtime::resolve()` | ✓ Complete |
| 6 | TASK-015, TASK-016 | `with_injectable()` + `try_build()` + CORE-018b regression check | ✓ Complete |
| 7 | TASK-017, TASK-018 | TestKit `with_service()` + `resolve()` pass-throughs | ✓ Complete |
| 8 | TASK-019 | Minimal end-to-end example (`hello_service.rs`) | ✓ Complete |
| 9 | TASK-020 | Acceptance walkthrough test (5 scenarios) | ✓ Complete |
| 10 | TASK-021, TASK-022 | `cargo doc` check + COOKBOOK.md exclusion note | ✓ Complete |

---

## Known Limitations (Recorded, Not Regressions)

**AD-7 (Architectural Limitation):** The SDK cannot model a service that is both an `Injectable` struct AND a resolvable trait proxy in the same change. This is a present boundary of expressiveness, not a deferral. If a real service needs both paths, it requires a dedicated follow-up design (post-build registration or construct-then-register bridge).

**F-08 (Deferred to Follow-up):** Aggregating all missing dependencies in a single error report (instead of stopping at the first) is deferred to CORE-025b. This change implements fail-fast at `try_build()` with first-failure semantics only.

---

## No Regressions

- **CORE-018b requirement preserved:** `RuntimeBuilder::build()` remains infallible and unchanged for all existing scenarios (logger wiring, teardown ordering, security-provider installation).
- **Guard order preserved:** Tenant enforcement, authorization, and interceptor execution order verified identical through generated proxy (byte-identical `{Trait}Ref`).
- **No global state introduced:** Registry lives instance-scoped inside `RuntimeInner`; no statics, no task-local state, no ambient context changes.
- **Hand-rolled path untouched:** Existing `{Trait}Ref::new(inner, chain, weak)` constructor remains callable and produces behavior-identical proxies.

---

## Archive Folder Structure

```
openspec/changes/archive/2026-07-11-core-025-service-sdk-ergonomics/
├── proposal.md
├── design.md
├── explore.md
├── tasks.md
├── verify-report.md
└── specs/
    ├── service-sdk/spec.md
    └── testkit/spec.md
```

All artifacts are immutable historical snapshots. Future changes to service-sdk or testkit specifications must flow through the living `openspec/specs/` files, not this archive.

---

## Next Steps

1. ✓ **Specs merged** — new requirements now part of living service-sdk and testkit specifications
2. ✓ **Change archived** — immutable closure record established
3. Follow-up changes (out of scope for CORE-025):
   - **F-04**: Delete unused `ServiceFactory` trait (independent cleanup)
   - **F-05**: Rewrite `COOKBOOK.md` (sequence after code lands to avoid staleness)
   - **F-08**: Aggregate multiple missing dependencies in one error (CORE-025b or micro-change)
   - **F-10**: Rename `runtime_builder.rs` if beneficial (cosmetic, no rush)

---

## Change Closed

**Archive Date:** 2026-07-11  
**Verify Status:** PASS WITH WARNINGS (0 CRITICAL, 2 accepted)  
**SDD Cycle:** COMPLETE  

This change is ready for the next cycle. The Service SDK developer experience is measurably improved: one registration call (`with_service`), one resolution call (`resolve`), fail-fast bootstrap detection, and named errors — all backward-compatible with the hand-rolled escape hatch preserved.
