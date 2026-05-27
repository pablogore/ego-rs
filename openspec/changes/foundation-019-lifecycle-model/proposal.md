## Why

ego-rs governs WHAT interaction means, HOW interaction becomes exposed, HOW participants interact, HOW behavior executes, HOW read knowledge materializes, HOW durable truth is preserved, and HOW execution ownership exists in space. However, lifecycle evolution semantics — how governed things activate, suspend, recover, restore, and transition through lifecycle — remain constitutionally ungoverned. Without dedicated lifecycle governance, activation semantics become runtime-defined, suspension semantics drift into implementation detail, restoration trustworthiness degrades, replay-safe lifecycle transitions become implicit, and the constitutional ownership chain remains incomplete.

## What Changes

- Introduce a new **Lifecycle Model** constitution defining how governed things evolve through lifecycle across ego-rs
- Define governed semantics for: lifecycle semantics, activation semantics, suspension semantics, recovery semantics, restoration semantics, lifecycle transition semantics, lifecycle consistency expectations, deterministic lifecycle behavior, lifecycle failure semantics, replay-safe lifecycle semantics, lifecycle observability semantics, lifecycle ownership boundaries, and cross-spec governance
- Establish explicit, non-overlapping authority boundaries between Lifecycle Model and Behavior Model, Projection Model, Persistence Model, Placement Model, Runtime Abstraction, and Architecture Governance
- Create delta specs clarifying authority boundaries for `behavior-model`, `projection-model`, `persistence-model`, `placement-model`, `runtime-abstraction`, and `architecture-governance`

## Capabilities

### New Capabilities
- `lifecycle-model`: Constitutional governance for how governed things evolve through lifecycle — lifecycle semantics, activation semantics, suspension semantics, recovery semantics, restoration semantics, lifecycle transition semantics, lifecycle consistency expectations, deterministic lifecycle behavior, lifecycle failure semantics, replay-safe lifecycle semantics, lifecycle observability semantics, lifecycle ownership boundary, governance enforcement, constitutional ownership chain, and cross-spec governance

### Modified Capabilities
- `behavior-model`: Behavior execution semantics remain governed by Behavior Model while lifecycle evolution semantics are governed by Lifecycle Model — authority boundary clarification only
- `projection-model`: Read materialization semantics remain governed by Projection Model while lifecycle evolution semantics are governed by Lifecycle Model — authority boundary clarification only
- `persistence-model`: Durable truth semantics remain governed by Persistence Model while lifecycle evolution semantics are governed by Lifecycle Model — authority boundary clarification only
- `placement-model`: Ownership-in-space semantics remain governed by Placement Model while lifecycle evolution semantics are governed by Lifecycle Model — authority boundary clarification only
- `runtime-abstraction`: Runtime execution implementation remains governed by Runtime Abstraction while lifecycle evolution semantics are governed by Lifecycle Model — authority boundary clarification only
- `architecture-governance`: Lifecycle behavior across architectural boundaries SHALL comply with both Architecture Governance and Lifecycle Model governance

## Impact

- New spec: `specs/lifecycle-model/spec.md`
- Delta specs: `specs/behavior-model/spec.md`, `specs/projection-model/spec.md`, `specs/persistence-model/spec.md`, `specs/placement-model/spec.md`, `specs/runtime-abstraction/spec.md`, `specs/architecture-governance/spec.md`
- No code changes — constitutional specification only
- No breaking changes — additive governance layer completing the lifecycle dimension of the ownership chain
