# Effect Type Contract

**Status**: Draft | **Spec**: [spec.md](../spec.md) | **Data Model**: [data-model.md](../data-model.md)

## Purpose

Domain-owned value type contract describing execution outcomes. Effects are returned from execution handlers and interpreted by runtime crates.

## Contract

```rust
/// Describes an execution outcome.
///
/// Effects are value types returned from execution handlers.
/// The runtime interprets the effect and executes the described
/// outcomes. Handlers do not execute effects directly.
///
/// Generic parameters are model-agnostic:
///   E = event type, R = reply type, S = state type
pub enum Effect<E, R, S> {
    /// No side effects.
    NoEffect,
    /// A state mutation carrying the new state.
    StateMutation(S),
    /// Event emission carrying event payloads.
    EventEmission(Vec<E>),
    /// A reply carrying the response value.
    Reply(R),
    /// Multiple effects composed together.
    Composed(Vec<Effect<E, R, S>>),
}
```

## Implementer Requirements

1. **Runtime implementations** MUST:
   - Provide an interpreter that matches all Effect variants exhaustively
   - Execute the described outcomes for each variant
   - Handle `Composed` by processing child effects in order

2. **Runtime implementations** MAY:
   - Add additional execution guarantees (ordering, transactional boundaries, idempotency)
   - Reject effects the runtime cannot interpret with a clear error during effect interpretation

3. **Entities (execution models)**:
   - Define concrete E, R, S types appropriate to the model
   - Construct Effect variants from handler logic
   - Return Effect from handler functions

## Derives

Effect MUST derive: `Debug, Clone, PartialEq, Eq, Hash`

## Testability

Effects are testable by value assertion:
- Construct expected Effect values in tests
- Assert handler returns exactly the expected Effect
- No infrastructure, no runtime, no IO required
