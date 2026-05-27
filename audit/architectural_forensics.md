# ARCHITECTURAL FORENSICS — Real Resulting Architecture

---

## BOUNDARY ANALYSIS

### Domain Layer (`crates/domain/`)

| Concern | Current State | Assessment |
|---------|--------------|------------|
| Core contracts (Command, Query, Event) | ✓ Clean, minimal traits | GOOD |
| Actor contract | ✗ Missing entirely. `pub mod actor;` doesn't exist. Doc-only. | BROKEN |
| Governance types | ✗ Removed (correctly). Empty file staged in index. | FIXING |
| Runtime types | ✓ Absent from domain (good). | GOOD |
| Infrastructure types | ✓ Absent from domain (good). | GOOD |

### Runtime Layer (`core/runtime-slice/`)

| Concern | Current State | Assessment |
|---------|--------------|------------|
| Domain types in runtime crate | ⚠ `RuntimeSliceId`, `ExecutionContext`, `DeterministicInput` are domain-level concepts living in a runtime crate | LEAKAGE |
| Executor | ✗ Empty stub, not in lib.rs | DEAD CODE |
| Projection | ✗ Empty stub, not in lib.rs | DEAD CODE |
| Validation | ✗ Empty stub, not in lib.rs | DEAD CODE |
| Persistence | ✗ Empty stub, not in lib.rs | DEAD CODE |
| Observability | ✗ Empty stub, not in lib.rs | DEAD CODE |
| Workspace membership | ✗ NOT a workspace member | BROKEN |

### Application Layer (`crates/application/`)

| Concern | Current State | Assessment |
|---------|--------------|------------|
| CQRS ports | ✓ CommandHandler, QueryHandler traits | GOOD |
| HelloHandler example | ✓ Working reference implementation | GOOD |
| Contract tests | ✗ Empty stubs | DEAD CODE |

### Infrastructure/Transport

| Concern | Current State | Assessment |
|---------|--------------|------------|
| `crates/infrastructure/` | ✓ Empty lib.rs (placeholder) | GOOD (placeholder) |
| `crates/transport/` | ✓ Empty lib.rs (placeholder) | GOOD (placeholder) |

---

## OVERENGINEERING REMOVAL QUALITY

### What was correctly removed:

| Item | Severity | Reason |
|------|----------|--------|
| Governance tiers in specs | HIGH | Premature — framework has no runtime to govern |
| Constitutional ownership chain (9 models) | HIGH | Zero code, pure bureaucracy |
| Spec-ception (loop governance correction) | HIGH | Fixing a spec that references a spec with zero code |
| Compliance verification mechanisms | MEDIUM | Premature — nothing to verify |
| Capability inflation protection | MEDIUM | Premature optimization of speculation |
| Examples constitution (meta-governance) | MEDIUM | Govern examples after writing them |
| Determinism constitution (separate doc) | LOW | Correctly merged into project-constitution |

### What was kept correctly:

| Item | Reason |
|------|--------|
| Project constitution | Core principles: determinism, fail-closed, explicit state |
| Architecture governance | Hexagonal layer rules |
| Testing governance | Mock-first, no-real-infra, 95% coverage |
| Runtime abstraction (simplified) | Execution semantics without runtime coupling |
| CORE-001 runtime slice | Only change with real code (types.rs) |

### What might have been over-simplified:

| Item | Risk | Assessment |
|------|------|------------|
| runtime-abstraction at 69 lines | Low | Still captures determinism axiom, lifecycle, boundaries, fail-closed. Adequate. |
| core-001 at 37 lines | Low | Captures deterministic execution, minimality, replay equivalence. Adequate. |
| core-002 actor spec | Low | Covers Actor trait, ActorId, lifecycle, supervision semantics. Adequate. |

**Assessment:** Overengineering removal was APPROPRIATE. No critical primitives were lost.

---

## LEAKAGE DETECTION

### Issue LEAK-01: Domain types in runtime crate
**Severity:** MEDIUM

**Evidence:** `core/runtime-slice/src/types.rs` defines:
- `RuntimeSliceId` — a domain identity type
- `ExecutionContext` — a domain concept
- `DeterministicInput` — a domain concept

These should live in `crates/domain/` or be eliminated in favor of simpler primitives.

**Why:** The runtime-slice was created as an independent crate before the domain/runtime boundary was established. It has not been integrated into the workspace.

**Recommendation:** During CORE-001 implementation, either:
- (A) Move these types to `crates/domain/` and have runtime-slice depend on domain, OR
- (B) Simplify: remove `RuntimeSliceId` and `ExecutionContext` in favor of simpler primitives in the runtime crate itself, and explicitly label them as runtime concepts.

### Issue LEAK-02: Actor module referenced but missing
**Severity:** MEDIUM

**Evidence:** `crates/domain/src/lib.rs` doc comment says module `actor` exists with `Actor` trait, `ActorId`, `actor_id!` macro. But `pub mod actor;` is NOT in the code, and `src/actor/` directory doesn't exist.

**Why:** CORE-002 spec was written but not implemented. The doc comment was prematurely updated.

**Recommendation:** Either implement the actor module (CORE-002) or remove the doc comment reference. Both are acceptable — the doc comment should match reality.

### Issue LEAK-03: Runtime slice not in workspace
**Severity:** HIGH

**Evidence:** `core/runtime-slice/` is not listed in `Cargo.toml` workspace members. No crate can depend on it.

**Why:** The crate was created before workspace integration was completed. The cleanup removed dead stubs but did not perform the integration.

**Recommendation:** Add to workspace as part of CORE-001. This is already a task in core-001/tasks.md.

---

## CORRECT PRESERVATIONS

| Pattern | Status | Why Good |
|---------|--------|----------|
| Domain contracts without runtime deps | ✓ Clean | `crates/domain/` has zero tokio/infra deps |
| Hexagonal layering | ✓ Clean | Layer boundaries clear in lib.rs docs |
| CQRS separation | ✓ Clean | Command/Event/Query as distinct traits |
| Determinism as first principle | ✓ Strong | Embedded in multiple specs and types.rs |
| Fail-closed as invariant | ✓ Strong | Present in runtime-abstraction, project-constitution, core-001 |
| Specs are atomic | ✓ Improved | One concern per change |
| No governance theater | ✓ Removed | Governance tiers, ownership chains archived |

---

## ARCHITECTURE QUALITY SCORE

| Dimension | Score (1-10) | Note |
|-----------|-------------|------|
| Domain purity | 7 | Clean CQRS, but actor module missing |
| Runtime separation | 5 | Specs are clean, but runtime-slice has domain types |
| Spec realism | 8 | Cleaner, implementable specs |
| Code completeness | 2 | Only types.rs has real code |
| Workspace integrity | 4 | runtime-slice not integrated |
| Boundary enforcement | 6 | Good in spec, mixed in code |
| Overengineering removal | 9 | Most bureaucracy removed |
| Claim-truth alignment | 3 | Claims overstated, nothing committed |

**Overall:** The cleanup SUCCESSFULLY simplified the spec tree and removed governance bureaucracy. However, the WORK IS UNCOMMITTED, claims are overstated, the actor module is missing, runtime-slice has domain types, and 5 empty stubs remain as dead code. The spec improvements are real but fragile (uncommitted).
