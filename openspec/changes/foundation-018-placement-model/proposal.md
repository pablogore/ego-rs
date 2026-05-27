## Why

ego-rs governs WHAT interaction means, HOW interaction becomes exposed, HOW participants interact, HOW behavior executes, HOW read knowledge materializes, and HOW durable truth is preserved. However, execution ownership semantics — how execution ownership exists, moves, localizes, and remains governable in space — remain constitutionally ungoverned. Without dedicated placement governance, ownership semantics drift into runtime implementations, locality becomes ambiguous, mobility is implementation-defined, and the constitutional ownership chain remains incomplete.

## What Changes

- Introduce a new **Placement Model** constitution defining how execution ownership exists, moves, localizes, and remains governable in space across ego-rs
- Define governed semantics for: ownership, locality, execution location abstraction, mobility, placement lifecycle, consistency, deterministic placement, failure, replay-safe placement, observability, ownership boundaries, and cross-spec governance
- Establish explicit, non-overlapping authority boundaries between Placement Model and Behavior Model, Projection Model, Persistence Model, Runtime Abstraction, and Architecture Governance
- Create delta specs clarifying authority boundaries for `behavior-model`, `projection-model`, `persistence-model`, `runtime-abstraction`, and `architecture-governance`

## Capabilities

### New Capabilities
- `placement-model`: Constitutional governance for how execution ownership exists in space — placement semantics, ownership semantics, locality semantics, execution location abstraction, mobility semantics, placement lifecycle semantics, placement consistency expectations, deterministic placement behavior, placement failure semantics, replay-safe placement semantics, placement observability semantics, placement ownership boundary, governance enforcement, constitutional ownership chain, and cross-spec governance

### Modified Capabilities
- `behavior-model`: Behavior execution semantics remain governed by Behavior Model while ownership-in-space semantics are governed by Placement Model — authority boundary clarification only
- `projection-model`: Read materialization semantics remain governed by Projection Model while ownership-in-space semantics are governed by Placement Model — authority boundary clarification only
- `persistence-model`: Durable truth semantics remain governed by Persistence Model while ownership-in-space semantics are governed by Placement Model — authority boundary clarification only
- `runtime-abstraction`: Runtime execution implementation remains governed by Runtime Abstraction while ownership-in-space semantics are governed by Placement Model — authority boundary clarification only
- `architecture-governance`: Placement behavior across architectural boundaries SHALL comply with both Architecture Governance and Placement Model governance

## Impact

- New spec: `specs/placement-model/spec.md`
- Delta specs: `specs/behavior-model/spec.md`, `specs/projection-model/spec.md`, `specs/persistence-model/spec.md`, `specs/runtime-abstraction/spec.md`, `specs/architecture-governance/spec.md`
- No code changes — constitutional specification only
- No breaking changes — additive governance layer completing the ownership chain
