# FORENSIC INVENTORY — Actual Repository State

> Generated: 2026-05-27
> Method: git status + working tree verification
> Note: All cleanup work is UNCOMMITTED unless stated otherwise.

---

## CANONICAL SPECS (`openspec/specs/`)

| Spec | Status | Actual File | Lines | Change Detected | Claimed Change | Match |
|------|--------|-------------|-------|-----------------|----------------|-------|
| architecture-governance | KEPT | `openspec/specs/architecture-governance/spec.md` | 44 | Uncommitted - doc comment format only. Content unchanged from initial commit. | "Kept" | YES |
| project-constitution | KEPT | `openspec/specs/project-constitution/spec.md` | 111 | Uncommitted - unchanged from initial commit. | "Kept" | YES |
| runtime-abstraction | SIMPLIFIED | `openspec/specs/runtime-abstraction/spec.md` | 69 (was 432) | **UNCOMMITTED.** 432→69 line reduction. Removed: SPI ports, governance tiers, capability model, compliance verification. | "Simplified 432→50 lines" | PARTIAL (69 vs 50 claimed) |
| testing-governance | KEPT | `openspec/specs/testing-governance/spec.md` | 55 | Uncommitted - unchanged from initial commit. | "Kept" | YES |

---

## ACTIVE CHANGES (`openspec/changes/`)

### CORE-001 (Deterministic Runtime Slice)

| File | Status | Lines | Change Detected | Match |
|------|--------|-------|-----------------|-------|
| `proposal.md` | UNCHANGED | 26 | Same as committed. | ✓ |
| `design.md` | UNCHANGED | 39 | Same as committed. | ✓ |
| `tasks.md` | MODIFIED (UNCOMMITTED) | →29 | Reduced from 42. Removed ownership-chain verifications. | ✓ |
| `specs/deterministic-runtime-slice/spec.md` | SIMPLIFIED (UNCOMMITTED) | 37 (was 165) | Removed FOUNDATION mutation checks, ownership chain, redundancy. | ✓ |
| `specs/replay-validation/spec.md` | UNCHANGED | 59 | Same as committed. | ✓ |
| `specs/semantic-observability-runtime/spec.md` | UNCHANGED | 23 | Same as committed. | ✓ |

### CORE-002 (Actor Primitive)

| File | Status | Lines | Change Detected | Match |
|------|--------|-------|-----------------|-------|
| ALL FILES | NEW (UNTRACKED) | Various | Created by cleanup as replacement for `foundation-004-actor-model`. | PARTIAL |
| `specs/actor-model/spec.md` | NEW | ~85 lines (estimated from reading) | Simplified from original 342. No governance tiers. | PARTIAL |
| `tasks.md` | NEW | ~22 items | Reduced from 577-line original. | YES |

**Key issue: `crates/domain/src/actor/` does NOT exist.** CORE-002 spec and doc comments reference it, but no code exists.

### CORE-003 (Runtime Actor Execution)

| File | Status | Lines | Change Detected | Match |
|------|--------|-------|-----------------|-------|
| ALL FILES | NEW (UNTRACKED) | Various | Created by cleanup. NOT a renamed foundation spec — this is NEW content unifying mailbox+dispatch+supervision. | N/A |

### CORE-004 (Persistence SPI)

| File | Status | Lines | Change Detected | Match |
|------|--------|-------|-----------------|-------|
| ALL FILES | NEW (UNTRACKED) | Various | Created by cleanup as replacement for `foundation-005-persistence-spi`. | PARTIAL |

### CORE-005 (Observability SPI)

| File | Status | Lines | Change Detected | Match |
|------|--------|-------|-----------------|-------|
| ALL FILES | NEW (UNTRACKED) | Various | Created by cleanup as replacement for `foundation-007-observability-spi`. | PARTIAL |

### CORE-007 (Cluster Model)

| File | Status | Lines | Change Detected | Match |
|------|--------|-------|-----------------|-------|
| ALL FILES | NEW (UNTRACKED) | Various | Created by cleanup as replacement for `foundation-006-cluster-model`. "Deferred to Phase 4." | PARTIAL |

### FOUNDATION-003 through FOUNDATION-020 (Deleted)

| Status | Count | Detail |
|--------|-------|--------|
| DELETED (UNSTAGED) | 17 dirs | `foundation-003-runtime-abstraction` through `foundation-020-runtime-execution-model` |
| DELETED (UNSTAGED) | 1 dir | `fail-closed-semantic-loop-correction` |
| Total deleted files | 144 | All unstaged, uncommitted |

---

## ARCHIVE (`openspec/changes/archive/`)

### Pre-existing (committed before cleanup)

| Entry | Committed? | Note |
|-------|-----------|------|
| `2026-05-25-foundation-001-workspace-monorepo-structure` | YES | Original initial foundation |
| `2026-05-25-project-governance` | YES | Original governance |
| `2026-05-25-spec-000-project-constitution-objetivo` | YES | Original constitution |
| `2026-05-26-foundation-002-canonical-contracts` | YES | Original contracts |
| `2026-05-26-foundation-003-runtime-abstraction` | YES | Original runtime abstraction (DUPLICATE of active) |

### Created by cleanup (UNTRACKED)

| Entry | Count | Note |
|-------|-------|------|
| `2026-05-27-core-002-fail-closed-runtime-governance` | 1 dir | Governance spec (archived, not active) |
| `2026-05-27-fail-closed-semantic-loop-correction` | 1 dir | Loop governance correction |
| `2026-05-27-foundation-003-runtime-abstraction` | 1 dir | SECOND archive copy of foundation-003 |
| `2026-05-27-foundation-008-examples-constitution` | 1 dir | Meta-governance |
| `2026-05-27-foundation-009-determinism-constitution` | 1 dir | Constitution addition |
| `2026-05-27-foundation-010-canonical-contracts-constitution` | 1 dir | Meta-governance |
| `2026-05-27-foundation-011-dependency-governance-constitution` | 1 dir | Meta-governance |
| `2026-05-27-foundation-012-service-contract-model` | 1 dir | Ownership chain |
| `2026-05-27-foundation-013-transport-binding-model` | 1 dir | Ownership chain |
| `2026-05-27-foundation-014-interaction-model` | 1 dir | Ownership chain |
| `2026-05-27-foundation-015-behavior-model` | 1 dir | Ownership chain |
| `2026-05-27-foundation-016-projection-model` | 1 dir | Ownership chain |
| `2026-05-27-foundation-017-persistence-model` | 1 dir | Ownership chain |
| `2026-05-27-foundation-018-placement-model` | 1 dir | Ownership chain |
| `2026-05-27-foundation-019-lifecycle-model` | 1 dir | Ownership chain |
| `2026-05-27-foundation-020-runtime-execution-model` | 1 dir | Ownership chain |

**Note:** `foundation-003` has TWO archive entries (May 26 committed + May 27 untracked).

---

## AUDIT FILES (UNTRACKED)

| File | Status | Real? | Content |
|------|--------|-------|---------|
| `audit/framework_audit.md` | UNTRACKED | YES | Constitution cleanup report. Contains some inaccuracies. |
| `audit/gap_analysis.md` | UNTRACKED | YES | Gap analysis. Generally accurate. |
| `audit/migration_plan.md` | UNTRACKED | YES | Migration steps. Claims DONE for uncommitted work. |
| `audit/new_framework_roadmap.md` | UNTRACKED | YES | Future roadmap. Well-structured. |
| `audit/technical_review.md` | UNTRACKED | YES | Technical review. Accurate findings. |

---

## SOURCE CODE STATE

### Core Runtime Slice (`core/runtime-slice/`)

| File | Status | Lines | Note |
|------|--------|-------|------|
| `Cargo.toml` | COMMITTED | 8 | Package: runtime-slice. NOT in workspace. |
| `src/lib.rs` | MODIFIED (UNCOMMITTED) | 31 | Doc comments updated. Only `pub mod types;`. |
| `src/types.rs` | MODIFIED (UNCOMMITTED) | ~90 | Expanded with detailed types. |
| `src/executor.rs` | EMPTY STUB | 0 | Not declared in lib.rs. Dead code. |
| `src/projection.rs` | EMPTY STUB | 0 | Not declared in lib.rs. Dead code. |
| `src/validation.rs` | EMPTY STUB | 0 | Not declared in lib.rs. Dead code. |
| `src/persistence.rs` | EMPTY STUB | 0 | Not declared in lib.rs. Dead code. |
| `src/observability.rs` | EMPTY STUB | 0 | Not declared in lib.rs. Dead code. |
| `src/example.rs` | DELETED (UNSTAGED) | - | Was empty. Removed by cleanup. |
| `src/main.rs` | DELETED (UNSTAGED) | - | Was empty. Removed by cleanup. |

### Domain Crate (`crates/domain/`)

| Item | Status | Note |
|------|--------|------|
| `src/actor/` | **DOES NOT EXIST** | CORE-002 actor module never created. |
| `src/governance/` | **DOES NOT EXIST** (working tree) | Governance directory removed by cleanup. |
| `src/governance/governance_context.rs` | **STAGED (empty)** | 0-byte file is in index. Not committed. Conflict state. |
| `src/command.rs` | MODIFIED (UNCOMMITTED) | Doc comments updated. |
| `src/event.rs` | MODIFIED (UNCOMMITTED) | Doc comments updated. |
| `src/hello.rs` | MODIFIED (UNCOMMITTED) | Doc comments updated. |
| `src/lib.rs` | MODIFIED (UNCOMMITTED) | Doc comment references `actor` module but code doesn't. |
| `src/query.rs` | MODIFIED (UNCOMMITTED) | Doc comments updated. |

### Workspace

| Item | Status | Note |
|------|--------|------|
| `Cargo.toml` members | COMMITTED | `crates/domain`, `crates/application`, `crates/infrastructure`, `crates/transport`. |
| `core/runtime-slice` | **NOT in workspace** | Cannot be used by any crate. CRITICAL issue. |

---

## SUMMARY

| Metric | Value |
|--------|-------|
| Total commits in history | 7 |
| Committed cleanup work | 0 (NONE committed) |
| Unstaged deletions | 144 files (old changes) |
| Unstaged modifications | 13 files (simplifications) |
| Untracked additions | 151 files (new archive + core changes + audit) |
| Staged (index) conflicts | 1 file (governance_context.rs - staged empty AND deleted) |
| Empty stub files remaining | 5 (executor, projection, validation, persistence, observability) |
| Claimed vs actual active changes | 5 claimed / 6 actual |
| Actor module code | DOES NOT EXIST |
| Runtime slice in workspace? | NO |
