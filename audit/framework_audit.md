# Framework Audit — Constitution Cleanup Report

## Executive Summary

**Before:** 26 active changes, 15+ spec layers, "constitutional ownership chain" of 9 interdependent models with zero code, ~120 spec files, 10 stub files, 1 change with real code.

**After:** 6 active changes, 4 canonical specs, ~25 spec files, 3 stubs, all changes framework-first and implementable.

---

## Classification Decisions

### KEEP

| Spec | Reason |
|------|--------|
| `specs/project-constitution` | Constitutional rules — single source of truth. Determinism, fail-closed, explicit state, OpenSpec-driven, hexagonal, 95% coverage. |
| `specs/architecture-governance` | Hexagonal layer rules enforced by `verify-layers.sh`. |
| `specs/testing-governance` | Testing standards (mocks-only, no-real-infra, 95% coverage). |
| `specs/runtime-abstraction` | Foundation for execution semantics. Simplified from 432→69 lines. |
| `changes/core-001-deterministic-runtime-slice` | Only change with pre-existing code (types.rs). Needs workspace integration. |
| `changes/core-002-actor-primitive` | THE central abstraction. Actor trait, ActorId, lifecycle states. Implemented with tests. |
| `changes/core-003-runtime-actor-execution` | Runtime mechanics: ActorSystem, mailbox, dispatch, supervision. Unified from separate mailbox/dispatch/supervision specs. |
| `changes/core-004-persistence-spi` | Persistence is foundational for stateful actors. Simplified from 503→64 lines. |
| `changes/core-005-observability-spi` | Built-in observability from start. Port-only contract. Simplified from 299→48 lines. |
| `archive/foundation-001` | Done. Workspace structure implemented. |
| `archive/spec-000` | Done. Original constitution. |
| `archive/project-governance` | Done. Governance specs in place. |
| `archive/foundation-002` | Done. Contracts spec. |

### REWORK

| Spec | Required Changes |
|------|-----------------|
| `specs/runtime-abstraction` | **DONE.** Stripped from 432 lines to 69 lines. Removed SPI ports, governance tiers, compliance verification, forbidden patterns. Kept: lifecycle states, Determinism Axiom, execution boundaries, fail-closed. |
| `changes/core-002-actor-primitive` | **DONE.** Reduced from 342→113 spec lines, 577→24 task lines. Implemented: Actor trait, ActorId, actor_id! macro, lifecycle states, supervision strategy. |
| `changes/core-004-persistence-spi` | **DONE.** Reduced from 503→64 lines. SPI trait only. Governance/compliance removed. |
| `changes/core-005-observability-spi` | **DONE.** Reduced from 299→48 lines. Port definition only. |
| `changes/core-007-cluster-model` | Defer to Phase 4. Not needed for MVP. |

### REMOVE (from codebase)

| Target | Reason |
|--------|--------|
| `crates/domain/src/governance/` | Dead code. 0-byte stub, not wired into lib.rs. Governance belongs in Phase 13, not Phase 0. |
| `core/runtime-slice/src/main.rs` | Empty. Runtime slice is a library crate. |
| `core/runtime-slice/src/example.rs` | Empty. Write real examples in `examples/`. |

### MERGE

| Sources → Target | Reason |
|-----------------|--------|
| `changes/foundation-009-determinism-constitution` → `specs/project-constitution` | Determinism is a core principle. Already covered in constitution. |
| `changes/foundation-011-dependency-governance` → enforced by `layers.toml` | Don't spec it twice. Already enforced. |

### ARCHIVED (15 changes)

| Archived Spec | Reason |
|---------------|--------|
| `foundation-003-runtime-abstraction` (active copy) | Duplicate of canonical `specs/runtime-abstraction`. Also duplicate of archived `foundation-003`. |
| `foundation-008-examples-constitution` | Meta-governance ("mandatory examples policy") with no examples. Write examples first. |
| `foundation-009-determinism-constitution` | Merged into constitution. |
| `foundation-010-canonical-contracts-constitution` | Meta-governance. Contracts already governed by archive/foundation-002. |
| `foundation-011-dependency-governance-constitution` | Already enforced by layers.toml + verify-layers.sh. |
| `foundation-012` through `foundation-020` (9 specs) | "Constitutional ownership chain" — 9 interconnected models (service-contract, transport-binding, interaction, behavior, projection, persistence-model, placement, lifecycle, runtime-execution) with zero code. Pure bureaucracy. |
| `core-002-fail-closed-runtime-governance` | Governance before the framework exists. Keep module design but defer to post-MVP. |
| `fail-closed-semantic-loop-correction` | Spec-ception. A governance correction to a governance spec with no code. |