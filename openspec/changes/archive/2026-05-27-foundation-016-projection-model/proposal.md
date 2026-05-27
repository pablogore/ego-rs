## Why

ego-rs governs service meaning, transport exposure, participant interaction, and behavior execution. However, projection behavior — how behavior outcomes become materialized, synchronized, replayed, and exposed as governed read knowledge — remains constitutionally ungoverned. Without dedicated projection governance, read-side materialization drifts into implicit semantics, replay trustworthiness degrades, consistency expectations become ambiguous, and observable read semantics lack constitutional grounding.

## What Changes

- Introduce a new **Projection Model** constitution defining how behavior becomes materialized as read knowledge across ego-rs
- Define governed semantics for: read-side materialization, projection lifecycle, replay-safe projections, projection consistency, deterministic projection behavior, projection observability, and projection failure behavior
- Establish explicit, non-overlapping authority boundaries between Projection Model and Behavior Model, Runtime Abstraction, and Architecture Governance
- Create delta specs clarifying authority boundaries for `behavior-model`, `runtime-abstraction`, and `architecture-governance`

## Capabilities

### New Capabilities
- `projection-model`: Constitutional governance for how behavior becomes materialized as read knowledge — read-side materialization semantics, projection lifecycle semantics, replay-safe projections, projection consistency expectations, deterministic projection behavior, projection observability semantics, projection failure semantics, governance enforcement, projection ownership and boundaries, and cross-spec governance

### Modified Capabilities
- `behavior-model`: Behavior execution remains governed by Behavior Model while read materialization semantics are governed by Projection Model — authority boundary clarification only, no requirement changes
- `runtime-abstraction`: Runtime execution implementation remains governed by Runtime Abstraction while projection semantics are governed by Projection Model — authority boundary clarification only, no requirement changes
- `architecture-governance`: Projection behavior across architectural boundaries SHALL comply with both Architecture Governance and Projection Model governance

## Impact

- New spec: `specs/projection-model/spec.md`
- Delta specs: `specs/behavior-model/spec.md` (authority boundary amendment), `specs/runtime-abstraction/spec.md` (authority boundary amendment), `specs/architecture-governance/spec.md` (governance scope amendment)
- No code changes — constitutional specification only
- No breaking changes — additive governance layer
