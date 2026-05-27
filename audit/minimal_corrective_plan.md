# MINIMAL CORRECTIVE PLAN

> Principle: KEEP 80%, FIX 20%. No mass rewrite. No redesign.

---

## STEP 1: Commit the Cleanup

**Status:** CRITICAL — nothing is committed.

**Change:** Stage and commit all working tree changes:
1. Stage untracked files: `openspec/changes/archive/*` (except dedup), `openspec/changes/core-002*` through `core-007*`, `audit/*`
2. Stage deletions of old foundation specs
3. Stage modifications to core-001 spec/tasks and canonical specs
4. Commit with message describing the cleanup

**Before committing, complete Steps 2-5 below.**

**Why:** The repository is in an inconsistent intermediate state. Without a commit, all cleanup work can be lost.

**Rollback:** `git stash` or `git checkout -- .`

---

## STEP 2: Fix Index Conflict

**Input:** `crates/domain/src/governance/governance_context.rs`

**Change:** `git rm --cached crates/domain/src/governance/governance_context.rs`

**Why:** An empty file is staged in the index while also showing as deleted from working tree. This is a conflicted state that prevents clean staging. The governance directory was correctly removed; the staged empty file is an artifact.

**Expected result:** Clean working tree. No governance files in index or working tree.

**Rollback:** `git reset HEAD -- crates/domain/src/governance/governance_context.rs` (if commit hasn't happened yet)

---

## STEP 3: Remove Duplicate Archive Entry

**Input:** `openspec/changes/archive/2026-05-27-foundation-003-runtime-abstraction/`

**Change:** Delete this directory. The May 26 entry (`2026-05-26-foundation-003-runtime-abstraction/`) already contains the original foundation-003 content. This is a duplicate.

**Why:** The cleanup created a second archive copy of foundation-003. The original was already archived as `2026-05-26-foundation-003-runtime-abstraction/`. The May 27 copy is identical content. Archive should have one entry per historical spec.

**Expected result:** Clean archive — each spec has exactly one archive entry.

**Rollback:** Restore from the May 26 copy (if needed, recover from git history).

---

## STEP 4: Remove Empty Stubs from runtime-slice

**Input:** 5 empty stub files in `core/runtime-slice/src/`

**Change:**
| File | Action | Why |
|------|--------|-----|
| `executor.rs` | KEEP (empty) but add `pub mod executor;` to lib.rs | Required by CORE-001 tasks |
| `projection.rs` | KEEP (empty) but add `pub mod projection;` to lib.rs | Required by CORE-001 tasks |
| `validation.rs` | KEEP (empty) but add `pub mod validation;` to lib.rs | Required by CORE-001 tasks |
| `persistence.rs` | DELETE | Persistence is CORE-004 concern, not runtime-slice |
| `observability.rs` | DELETE | Observability is CORE-005 concern, not runtime-slice |

**Alternative (preferred):** Remove ALL empty stubs, let CORE-001 implementation create them when actually needed. Empty stubs with no module declaration are dead code.

**Why:** Empty stubs that aren't declared in lib.rs are dead code — they compile to nothing but clutter the tree. They mislead about implementation status. Either wire them in or remove them.

**Expected result:** No empty undeclared files in runtime-slice/src/.

**Rollback:** `git checkout -- core/runtime-slice/src/<file>`

---

## STEP 5: Fix Actor Module Doc Reference

**Input:** `crates/domain/src/lib.rs`

**Change:** Remove the doc-comment table row referencing `actor` module:

```
-| `actor` | `Actor` trait, `ActorId`, `actor_id!` macro (CORE-002) |
```

**Or (better):** Implement the actor module. CORE-002 tasks define ~80 lines of code:
- `Actor` trait (1 line: `type Message;`)
- `ActorId` newtype (~15 lines)
- `actor_id!` macro (~8 lines)
- `ActorLifecycleState` enum (~6 lines)
- `SupervisionStrategy` enum (~4 lines)

**Why:** The doc comment claims a module exists that doesn't. This is misleading. Either implement it or remove the reference.

**Expected result:** `crates/domain/src/lib.rs` doc comment accurately reflects the module structure.

**Rollback:** `git checkout -- crates/domain/src/lib.rs`

---

## STEP 6: Fix Audit File Inaccuracies

**Input:** `audit/framework_audit.md`, `audit/migration_plan.md`

**Changes:**

### framework_audit.md
- Change "5 active changes" → "6 active changes"
- Add `core-003-runtime-actor-execution` to the KEEP table
- Change "432→50 lines" → "432→69 lines" for runtime-abstraction

### migration_plan.md
- Change STEP 0-3 status from "DONE ✓" to "DONE (UNCOMMITTED) ✓" to avoid misleading future readers
- Update STEP 1 mapping to match actual disk naming:
  - foundation-004-actor-model → core-002-actor-primitive ✓
  - foundation-005-persistence-spi → core-004-persistence-spi (was core-003)
  - foundation-007-observability-spi → core-005-observability-spi (was core-004)
  - foundation-006-cluster-model → core-007-cluster-model ✓
  - ADD: NEW → core-003-runtime-actor-execution (was not a rename)

**Why:** Audit files will be committed as part of the cleanup. Inaccuracies in them will mislead everyone who reads them in the future.

**Expected result:** Audit files accurately describe what was done, with correct counts and mappings.

**Rollback:** `git checkout -- audit/<file>`

---

## EXECUTION ORDER

```
STEP 2   Fix index conflict          (git rm --cached)
STEP 3   Remove duplicate archive    (rm -rf)
STEP 4   Clean empty stubs           (rm + lib.rs edit)
STEP 5   Fix actor doc reference     (edit lib.rs)
STEP 6   Fix audit inaccuracies      (edit audit files)
STEP 1   Commit everything           (git add + git commit)
```

---

## WHAT NOT TO TOUCH

| Item | Reason |
|------|--------|
| Original 432-line runtime-abstraction spec (committed) | Don't rewrite history. The old committed version is preserved in git. |
| Any foundation-003 through foundation-020 content | Archived correctly. Don't re-review. |
| CORE-001 through CORE-007 spec content | Correctly simplified. Don't re-simplify. |
| architecture-governance, project-constitution, testing-governance | Correctly kept. Don't modify. |
| workspace Cargo.toml members | Leave as-is until CORE-001 implementation adds runtime-slice. |
| audit files beyond inaccuracy fixes | Content is directionally correct. Don't rewrite. |
| codebase crate content (command/event/query/hello) | Working examples. Don't touch. |

---

## SUMMARY OF CHANGES

| # | Action | Scope | Risk |
|---|--------|-------|------|
| 1 | Commit cleanup | Entire tree | Low |
| 2 | Fix index conflict | 1 file | None |
| 3 | Remove duplicate archive | 1 directory | None |
| 4 | Clean empty stubs | 5 files in runtime-slice | Low |
| 5 | Fix actor doc reference | 1 file in domain lib.rs | Low |
| 6 | Fix audit inaccuracies | 2 audit files | None |

**Total blast radius:** 6 small changes, ~10 files. No redesign. No rewrite. No overreaction.
