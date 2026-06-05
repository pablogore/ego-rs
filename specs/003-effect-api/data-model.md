# Data Model: Effect API

**Date**: 2026-06-04 | **Spec**: [spec.md](spec.md) | **Research**: [research.md](research.md)

## Overview

The Effect data model defines value types that describe execution outcomes. These types live in `ego-domain` and are runtime-neutral. Effects are data, not behavior — they describe what the handler wants to happen, not how to execute it.

### Ownership Boundaries

Effect API **owns**: Effect enum hierarchy, composition logic, handler return type contract.
Effect API does **NOT own**: interpretation/execution, ExecutionContext, event store integration, reply transport.

## Entities

### Effect\<E, R, S\> (enum)

The canonical effect type describing an execution outcome.

| Variant | Inner Type | Description |
|---------|------------|-------------|
| `NoEffect` | — | No side effects. The handler completed without producing outcomes. |
| `StateMutation(S)` | S | A state change. Carries the new state value. |
| `EventEmission(Vec<E>)` | `Vec<E>` | One or more events to persist. Carries a vector of event payloads. |
| `Reply(R)` | R | A reply to send back to the caller. Carries the reply value. |
| `Composed(Vec<Effect<E, R, S>>)` | `Vec<Effect<E, R, S>>` | Multiple effects composed together. Contains a list of child effects. Recursive — children may themselves be Composed. |

**Validation**: 
- `Composed` MUST contain at least one child effect. Empty compositions are logic errors.
- `EventEmission` MUST NOT contain an empty Vec (no-op should use `NoEffect`).

**Derives**: Debug, Clone, PartialEq, Eq, Hash.

**State Transitions**: None. Effects are immutable after construction.

### Generic Type Parameters

| Parameter | Purpose | Constraints |
|-----------|---------|-------------|
| `E` | Event type | None — may be any type the execution model defines |
| `R` | Reply type | None — may be any type the execution model defines |
| `S` | State type | None — may be any type the execution model defines |

### Handler Return Type

Execution handlers return `Effect<E, R, S>` synchronously. The return type is generic and model-agnostic:

```rust
// Type alias (convenience, not required)
type HandlerResult<E, R, S> = Effect<E, R, S>;

// Handler signature pattern
fn handle(input: Input) -> Effect<E, R, S> { ... }
```

### Relationship Diagram

```text
ExecutionHandler
    │
    │ returns
    ▼
Effect<E, R, S> (enum)
    ├── NoEffect
    ├── StateMutation<S>
    ├── EventEmission<E>
    ├── Reply<R>
    └── Composed<E, R, S>
            └── children: Vec<Effect<E, R, S>>
```

## Cross-References

- **Spec Requirements**: FR-001 through FR-011
- **Research Decisions**: AD-001 (enum), AD-002 (generics), AD-003 (composition)
