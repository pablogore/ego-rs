# SPEC DECISION MATRIX — Keep / Fix / Rework / Rollback / Archive

---

## CANONICAL SPECS

### SPEC-01: `specs/project-constitution`

| Field | Value |
|-------|-------|
| **Decision** | KEEP |
| **Why** | Core principles: determinism, fail-closed, explicit state, OpenSpec-driven, hexagonal, 95% coverage |
| **Blast radius** | All other specs depend on it |
| **Risk** | None. Well-written, stable. |
| **Ready for implementation** | Yes — constraining rules, not implementation |

### SPEC-02: `specs/architecture-governance`

| Field | Value |
|-------|-------|
| **Decision** | KEEP |
| **Why** | Hexagonal layer rules. Layer enforcement script exists (`verify-layers.sh`?). Essential for build integrity. |
| **Blast radius** | All crate dependency declarations |
| **Risk** | None. Standard hexagonal rules. |
| **Ready for implementation** | Yes — already enforced by workspace structure |

### SPEC-03: `specs/testing-governance`

| Field | Value |
|-------|-------|
| **Decision** | KEEP |
| **Why** | Mock-first, no-real-infra, 95% coverage. Necessary for quality. |
| **Blast radius** | Test code only |
| **Risk** | Coverage enforcement is aspirational (no CI). But spec is directionally correct. |
| **Ready for implementation** | Partially — 95% CI enforcement not configured yet. Spec is fine. |

### SPEC-04: `specs/runtime-abstraction` (simplified, 69 lines)

| Field | Value |
|-------|-------|
| **Decision** | KEEP (current simplified version) |
| **Why** | Captures Determinism Axiom, lifecycle states (Pending→Running→Completed/Failed/Cancelled/TimedOut), execution boundaries, fail-closed, concurrency model, testing contract. All essential. |
| **Blast radius** | All CORE specs reference it |
| **Risk** | Low. Well-scoped. Removed premature SPI governance. |
| **Ready for implementation** | Yes — implementable as-is |

**Action:** Roll forward. Do NOT revert to 432-line version.

---

## ACTIVE CHANGES

### CORE-001: Deterministic Runtime Slice

| Field | Value |
|-------|-------|
| **Decision** | KEEP |
| **Why** | Only change with real working code (`types.rs`). Simplified spec (37 lines) is clean. |
| **Issues** | 5 empty stubs remain (executor, projection, validation, persistence, observability). Not in workspace. |
| **Blast radius** | Foundational — all runtime depends on it |
| **Risk** | Low. Empty stubs don't break anything. Workspace integration is the first implementation step. |
| **Ready for implementation** | Spec is ready. Tasks exist. Needs workspace integration + stub implementation. |
| **Action** | KEEP. Implement CORE-001. Remove stubs that won't be implemented now. |

### CORE-002: Actor Primitive

| Field | Value |
|-------|-------|
| **Decision** | KEEP WITH FIXES |
| **Why** | Central abstraction. `Actor` trait, `ActorId`, `actor_id!` are minimal and clean. Domain/runtime split is correct. |
| **Issues** | **No code exists.** `crates/domain/src/actor/` doesn't exist. Doc comment in domain lib.rs references it but code doesn't. |
| **Blast radius** | All actor-based specs (CORE-003, 004, 007) depend on it |
| **Risk** | Medium. Actor is the central primitive. If the trait is wrong, downstream changes cascade. However, the spec is minimal enough to change safely. |
| **Ready for implementation** | Partially. Spec is clean. Needs actual code. The `type Message;` trait is trivially implementable. |
| **Action** | KEEP spec. Implement `crates/domain/src/actor/` module. Simple code: Actor trait (1 line), ActorId newtype, actor_id! macro, lifecycle enum, supervision enum. ~80 lines of code. |

### CORE-003: Runtime Actor Execution

| Field | Value |
|-------|-------|
| **Decision** | KEEP |
| **Why** | Unifies mailbox, dispatch, supervision into one atomic spec. Clean separation from CORE-002 (domain contract). |
| **Issues** | No code exists. Depends on CORE-002 actor types. |
| **Blast radius** | Medium — depends on CORE-002, foundation for CORE-007 |
| **Risk** | Low. Well-scoped. Implementation can be incremental. |
| **Ready for implementation** | Spec is ready. Needs CORE-002 types first. |
| **Action** | KEEP. Implement after CORE-002. |

### CORE-004: Persistence SPI

| Field | Value |
|-------|-------|
| **Decision** | KEEP |
| **Why** | EventStore + SnapshotStore traits. Essential for stateful actors. Hexagonal — keeps backends pluggable. |
| **Issues** | No code exists. The spec on disk is the ORIGINAL 503-line version (I need to verify whether the UNTRACKED version is simplified or not). |
| **Blast radius** | Low — persistence is a port, not a core |
| **Risk** | Low. Traits are straightforward. |
| **Ready for implementation** | Depends on CORE-002 (actors). Spec revision may be needed if it's the old 503-line version. |
| **Action** | KEEP. But verify the actual spec content of the untracked file. If it's still 503 lines, simplify to ~100 lines. |

### CORE-005: Observability SPI

| Field | Value |
|-------|-------|
| **Decision** | KEEP |
| **Why** | Built-in observability from start. Port-only contract keeps vendors neutral. |
| **Issues** | No code. Depends on CORE-001 runtime. |
| **Blast radius** | Low — observability is a port |
| **Risk** | Low |
| **Ready for implementation** | Depends on CORE-001. Spec is reasonable. |
| **Action** | KEEP. Defer implementation until CORE-001 is stable. |

### CORE-007: Cluster Model

| Field | Value |
|-------|-------|
| **Decision** | KEEP (as deferred) |
| **Why** | Distributed coordination is needed eventually. Core-007 was correctly deferred by the audit ("Phase 4, not MVP"). |
| **Issues** | Exists as active change but should be deferred. The spec might still contain premature detail. |
| **Blast radius** | Low — deferred |
| **Risk** | Low — intentional deferral |
| **Ready for implementation** | NOT ready — depends on CORE-003, 004, 006. Correctly deferred. |
| **Action** | KEEP but move to pending/deferred status. Do not archive — cluster model has value. |

---

## ARCHIVED ITEMS

### Pre-existing Archives (keep)

| Entry | Decision | Why |
|-------|----------|-----|
| foundation-001 (workspace) | KEEP | Done. Workspace structure based on this. |
| project-governance | KEEP | Original governance reference. |
| spec-000 (constitution) | KEEP | Original constitution, historical reference. |
| foundation-002 (contracts) | KEEP | Completed contracts. |
| foundation-003 (runtime, May 26) | KEEP | Original runtime abstraction, historical reference. |

### Cleanup-created Archives (keep)

| Entry | Decision | Why |
|-------|----------|-----|
| foundation-008 through 020 (7 entries) | KEEP | Constitutional ownership chain. Correctly archived — zero code. |
| core-002-fail-closed-runtime-governance | KEEP | Governance before runtime existed. Correctly archived. |
| fail-closed-semantic-loop-correction | KEEP | Spec-ception. Correctly archived. |

### DUPLICATE — needs resolution

| Entry | Decision | Why |
|-------|----------|-----|
| foundation-003 (May 27 copy) | DELETE | Duplicate of May 26 archive + canonical spec. Two copies of same thing. |

**Action:** Remove `archive/2026-05-27-foundation-003-runtime-abstraction/`. The May 26 copy already captures this content. The canonical spec is the simplified 69-line version.

---

## FILES TO REMOVE / CLEAN UP

| File | Action | Why |
|------|--------|-----|
| `crates/domain/src/governance/governance_context.rs` (staged) | `git rm --cached` | Empty staged file, already deleted from working tree. Index conflict. |
| `core/runtime-slice/src/executor.rs` | Implement or remove | Empty stub, not in lib.rs |
| `core/runtime-slice/src/projection.rs` | Implement or remove | Empty stub, not in lib.rs |
| `core/runtime-slice/src/validation.rs` | Implement or remove | Empty stub, not in lib.rs |
| `core/runtime-slice/src/persistence.rs` | Remove | Persistence belongs in CORE-004, not in runtime-slice |
| `core/runtime-slice/src/observability.rs` | Remove | Observability belongs in CORE-005, not in runtime-slice |

---

## SUMMARY

| Verdict | Count | Items |
|---------|-------|-------|
| KEEP | 10 | project-constitution, architecture-governance, testing-governance, runtime-abstraction (simplified), CORE-001, CORE-003, CORE-004, CORE-005, CORE-007 (deferred), all pre-existing archives |
| KEEP WITH FIXES | 1 | CORE-002 (needs actual code module) |
| REMOVE/ROLLBACK | 1 | Duplicate foundation-003 archive entry |
| FIX | 2 | Index conflict (governance_context.rs), empty stubs in runtime-slice |
