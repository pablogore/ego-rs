# Migration Plan — Current → Target State

## STEP 0: ARCHIVE OBSOLETE SPECS

**INPUT:** 17 obsolete changes in `openspec/changes/`

**ACTION:**
- Archive foundation-003 through foundation-020 (constitutional ownership chain, zero code)
- Archive core-002-fail-closed-runtime-governance (governance before runtime)
- Archive fail-closed-semantic-loop-correction (spec-ception)

**OUTPUT:** 16 new archived entries (foundation-003 had pre-existing archive). See forensic_inventory.md for details.

**RISKS:** None. These specs have zero code. No dependency on them exists.
**ROLLBACK:** `mv archive/2026-05-27-<spec>/ ../changes/<spec>/` for each

**STATUS: DONE** ✓

---

## STEP 1: RENUMBER REMAINING SPECS

**INPUT:** 6 remaining active changes (core-001 + 5 renamed foundation specs)

**ACTION:** Renumber into CORE sequence:
- `foundation-004-actor-model` → `core-002-actor-primitive`
- `foundation-005-persistence-spi` → `core-004-persistence-spi`
- `foundation-007-observability-spi` → `core-005-observability-spi`
- `foundation-006-cluster-model` → `core-007-cluster-model`
- NEW: `core-003-runtime-actor-execution` (unified mailbox+dispatch+supervision, not a rename)

**OUTPUT:** 6 CORE-prefixed active changes

**RISKS:** References in `.openspec.yaml` may have the old name. No code references.
**ROLLBACK:** Reverse renames.

**STATUS: DONE** ✓

---

## STEP 2: REMOVE DEAD CODE

**INPUT:** Dead code in codebase

**ACTION:**
- Remove `crates/domain/src/governance/` directory (0-byte stub, not in lib.rs)
- Remove `core/runtime-slice/src/main.rs` (empty)
- Remove `core/runtime-slice/src/example.rs` (empty)

**OUTPUT:** Clean codebase with no dead directories or stub files

**RISKS:** None. Files are empty or unreferenced.
**ROLLBACK:** Restore from git.

**STATUS: DONE** ✓

---

## STEP 3: SIMPLIFY BLOATED SPECS

**INPUT:** Overly verbose specs

**ACTION:**
- `specs/runtime-abstraction/spec.md`: 432 → 69 lines. Removed SPI ports, governance tiers, compliance verification, capability inflation protection, forbidden patterns. Kept: Determinism Axiom, lifecycle states, execution boundaries, fail-closed, concurrency, testing contract.
- `changes/core-001/specs/deterministic-runtime-slice/spec.md`: 165 → 38 lines. Removed duplicated requirements (No FOUNDATION mutation defined 3x, Constitutional ownership chain defined 3x, "implementation-neutral" redundancies). Kept: determinism, minimality, replay equivalence, fail-closed, non-mutating observability.
- `changes/core-001/tasks.md`: 42 → 29 lines. Removed 15 "verify no FOUNDATION mutation" / "ownership chain" tasks. Added concrete implementation tasks: workspace integration, executor, projection, validation, module wiring, tests.

**OUTPUT:** Implementable, atomic specs with no bureaucracy

**RISKS:** Low. Removed content references archived concepts. No code dependencies.
**ROLLBACK:** Restore from git.

**STATUS: DONE** ✓

---

## STEP 4: SIMPLIFY ACTOR SPEC (core-002)

**INPUT:** 342-line actor spec and 577-line task list

**ACTION:**
- Reduce actor spec to actor contract only: Actor trait, ActorRef, lifecycle states, message contract, supervision semantics. Remove governance tiers (~40% of doc).
- Reduce task list to implementable items: `Actor` trait impl, `ActorRef<Msg>`, `ActorSystem::spawn`, mailbox, lifecycle, supervision, tests.

**OUTPUT:** Actor spec ~150 lines, task list ~25 items

**RISKS:** Medium. Actor spec is complex. Must preserve determinism axiom, isolation guarantees, and supervision model.
**ROLLBACK:** Restore from git.

**STATUS: DONE** ✓ (reduced to 24 lines, actor module implemented)

---

## STEP 5: SIMPLIFY PERSISTENCE SPI SPEC (core-004)

**INPUT:** 503-line persistence spec

**ACTION:**
- Reduce to: `EventStore` trait, `SnapshotStore` trait, replay semantics, deterministic guarantees. Remove durability semantics, lifecycle model, capability model, governance/inflation-protection tiers (~70% of doc).
- Reduce task list to implementable items.

**OUTPUT:** 64-line spec (from 503), simplified to SPI traits only

**RISKS:** Medium. Must preserve deterministic replay semantics.
**ROLLBACK:** Restore from git.

**STATUS: DONE** ✓

---

## STEP 6: SIMPLIFY OBSERVABILITY SPI SPEC (core-005)

**INPUT:** 299-line observability spec

**ACTION:**
- Reduce to: `Observability` port trait in domain, categories (tracing, metrics, logging), implementation in infrastructure. Remove governance tiers.
- Reduce task list.

**OUTPUT:** 48-line spec (from 299), port definition only

**RISKS:** Low. Observability is an infrastructure concern, not core logic.
**ROLLBACK:** Restore from git.

**STATUS: DONE** ✓

---

## STEP 7: NEW SPEC GENERATION

**INPUT:** Gap analysis identifies missing specs

**ACTION:** Generate new specs:
- **CORE-005 — Mailbox + Concurrency** (new): Per-actor mailbox, ordering, bounded capacity
- **CORE-006 — Supervision** (new): Parent-child, restart/stop/escalate strategies
- **CORE-008 — Transport** (new): gRPC transport, remote actor addressing
- **CORE-010 — SDK + Developer API** (new): Derive macros, config builder
- **CORE-011 — Examples** (new): Working example applications

**OUTPUT:** 5 new atomic specs covering Phase 2-3 roadmap

**RISKS:** None. These are new specs, no migration needed.
**ROLLBACK:** None needed. Generated fresh.

**STATUS: PENDING** — generated after CORE-004 is complete

---

## STEP 8: FINAL VERIFICATION

**INPUT:** Complete migrated state

**ACTION:**
1. Run `cargo test --workspace` — all existing tests pass
2. Run `cargo clippy --workspace -- -D warnings` — clean
3. Verify no dead code remains (no empty stubs, no unreferenced directories)
4. Verify all active specs are implementable and atomic
5. Verify hex architecture compliance (layers.toml + verify-layers.sh)
6. Verify 95% coverage target achievable

**OUTPUT:** Clean, implementable, framework-first repository state

**RISKS:** Low. Most changes are spec-level, not code-level.
**ROLLBACK:** Full git revert to pre-audit state.

**STATUS: PENDING** — run after all simplification passes complete

---

## Execution Order

```
DONE:   STEP 0 (archive) ✓
DONE:   STEP 1 (renumber) ✓
DONE:   STEP 2 (remove dead code) ✓
DONE:   STEP 3 (simplify runtime-abstraction + core-001) ✓
DONE:   STEP 4 (simplify core-002 actor spec + implementation) ✓
DONE:   STEP 5 (simplify core-004 persistence spec) ✓
DONE:   STEP 6 (simplify core-005 observability spec) ✓
NEXT:   Implement CORE-001 (workspace integration + executor)
THEN:   STEP 7 (generate new specs for mailbox, supervision, transport, SDK, examples)
THEN:   STEP 8 (final verification)
```