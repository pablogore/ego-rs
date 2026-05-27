## Why

ego-rs governs WHAT interaction means, HOW interaction becomes exposed, HOW participants interact, HOW behavior executes, and HOW read knowledge materializes. However, persistence semantics — how durable truth is preserved, restored, replayed, and trustworthily governed — remain constitutionally ungoverned. Without dedicated persistence governance, durable state semantics become ambiguous, replay trustworthiness degrades, restoration behavior becomes inconsistent, and lineage trustworthiness drifts into implementation-defined territory.

## What Changes

- Introduce a new **Persistence Model** constitution defining how durable truth is preserved, restored, replayed, and trustworthily governed across ego-rs
- Define governed semantics for: durable state, persistence lifecycle, replay-safe persistence, snapshots, restoration, consistency, deterministic persistence, observability, failure, lineage trustworthiness, ownership boundaries, and cross-spec governance
- Establish explicit, non-overlapping authority boundaries between Persistence Model and Projection Model, Behavior Model, Runtime Abstraction, and Architecture Governance
- Create delta specs clarifying authority boundaries for `projection-model`, `runtime-abstraction`, and `architecture-governance`

## Capabilities

### New Capabilities
- `persistence-model`: Constitutional governance for how durable truth is preserved, restored, and replayed — persistence semantics, durable state semantics, persistence lifecycle semantics, replay-safe persistence, snapshot semantics, restoration semantics, persistence consistency expectations, deterministic persistence behavior, persistence failure semantics, persistence observability semantics, lineage trustworthiness, governance enforcement, persistence ownership and boundaries, and cross-spec governance

### Modified Capabilities
- `projection-model`: Read materialization remains governed by Projection Model while durable truth preservation is governed by Persistence Model — authority boundary clarification only, no requirement changes
- `runtime-abstraction`: Runtime execution implementation remains governed by Runtime Abstraction while persistence semantics are governed by Persistence Model — authority boundary clarification only, no requirement changes
- `architecture-governance`: Persistence behavior across architectural boundaries SHALL comply with both Architecture Governance and Persistence Model governance

## Impact

- New spec: `specs/persistence-model/spec.md`
- Delta specs: `specs/projection-model/spec.md` (authority boundary amendment), `specs/runtime-abstraction/spec.md` (authority boundary amendment), `specs/architecture-governance/spec.md` (governance scope amendment)
- No code changes — constitutional specification only
- No breaking changes — additive governance layer
