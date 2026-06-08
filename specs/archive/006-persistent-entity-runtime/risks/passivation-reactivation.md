# Implementation Risk: PASSIVATED → RECOVERING Auto-Reactivation

**Feature**: `006-persistent-entity-runtime`
**Created**: 2026-06-07
**Status**: Documented
**Risk Class**: Implementation Safety / Concurrency

## Clarified Separation

Per the spec clarification (Session 2026-06-07): **Semantic guarantees are strict and observable; implementation guards are internal, optional in mechanism but mandatory in outcome.**

- Single-actor invariant is **strict at the observable level**: exactly one task processes entity commands at any time.
- Reactivation is **single-flight**: at most one activation process may be in-flight per entity. Duplicate spawn attempts MUST NOT occur — concurrent activation triggers MUST coalesce.
- Stale sender handle detection is a **purely internal implementation detail**, not part of the semantic model.
- The guard mechanism (CAS, per-entity lock, single-flight) is **implementation-defined** — only the outcome (single actor, exactly-once activation per transition window) is mandatory.

## Context

When a command is sent to a PASSIVATED entity, the runtime must detect a stale sender handle and automatically:
1. Look up the entity in the Passivation Registry
2. Spawn a new Tokio task
3. Transition the entity to RECOVERING
4. Deliver the command transparently

This behavior is fully specified at the semantic level, but introduces two implementation-level races.

---

## Risk 1: Stale Sender Handle Detection Race

**Description**: Between the moment a sender handle is checked and the moment it is used, the entity may transition from PASSIVATED → RECOVERING (or ACTIVE). A stale handle may still be present locally while the entity has already been reactivated elsewhere.

**Impact**:
- Duplicate task spawn attempts
- Redundant recovery initialization
- Potential double-processing if not guarded

**Severity**: Medium — violates single-actor invariant if triggered.

---

## Risk 2: Registry Lookup vs Task Spawn Race

**Description**: The sequence of (1) lookup entity in Passivation Registry, (2) decide to spawn new task, (3) spawn Tokio task is not atomic. Concurrent requests may all observe PASSIVATED state, all attempt to spawn, and race to create multiple actors for the same entity.

**Impact**:
- Violation of single-actor-per-entity invariant
- Duplicated mailbox/task creation

**Severity**: High — directly breaks a core architectural invariant of the spec.

---

## Required Implementation Guard

The specification mandates single-flight reactivation (exactly one activation process per entity at any time). Implementations MUST enforce this through one of:

- **Atomic check-and-spawn**: Atomically mark the registry entry as pending-reactivation before spawning; fail fast if already pending.
- **Per-entity spawn lock**: A lightweight mutex per entity triple guards the activation path, serializing concurrent activation requests.
- **Single-flight pattern**: Coalesce concurrent activation requests — the first acquires, rest await the same result.
- **Channel-based ownership**: The registry issues sender handles only for entities with an active task. For PASSIVATED entities, the handle is created atomically with the new task and issued only after both registry update and task spawn are complete.

### Mitigation Strategy Comparison

| Strategy | Description | Tradeoff |
|----------|-------------|----------|
| Compare-and-swap (CAS) on registry entry | Atomically transition the registry entry from PASSIVATED → RECOVERING; only the winner spawns the task | Requires atomically-addressable state in registry |
| Per-entity spawn lock | A lightweight mutex per entity triple guards the activation path | Memory overhead for lock table; potential contention under burst |
| Single-flight pattern | Coalesce concurrent activation requests: first acquires, rest await the same result | Clean abstraction; requires dedup key per entity |
| Channel-based ownership | The Passivation Registry does not hand out sender handles to PASSIVATED entities; the channel is created atomically with the new task and issued only after both registry update and task spawn are complete | Simplifies the client path; shifts complexity to registry lifecycle |

---

## Non-Impact Declaration

- This is NOT a functional requirement gap in CORE-006
- This does NOT change runtime semantics defined in the spec
- This does NOT affect correctness model (strict event sourcing, deterministic replay)
- This is purely an implementation safety concern for concurrent activation edge cases

## References

- Spec section: `§5 Passivation Interaction (Irreversible Drain)` — includes "Reactivation safety (single-flight)" subsection
- Spec section: `§1 Execution Model: Actor Per Entity`
- Spec FR-023: PASSIVATED auto-reactivation semantics
- Spec FR-007: Actor Per Entity model (single-actor invariant)
- Spec Clarifications (Session 2026-06-07): PASSIVATED reactivation semantics vs implementation guards, single-flight model
