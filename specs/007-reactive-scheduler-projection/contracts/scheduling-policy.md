# Contract: SchedulingPolicy

> The `SchedulingPolicy` trait defines the contract for deterministic activation suggestion.
> Implementations are pure functions with no side effects.

---

## Trait Definition

```rust
/// A pure function that selects the next entity to activate.
///
/// # Determinism
/// - Identical inputs MUST produce identical outputs
/// - No side effects, no I/O, no wall-clock dependency
/// - MUST complete within bounded time
///
/// # Invariants
/// - `pending_entities` is the set of entities eligible for activation
/// - The returned entity MUST be a member of `pending_entities` (if `pending_entities` is non-empty)
/// - Returns `None` if and only if `pending_entities` is empty
pub trait SchedulingPolicy: Send + Sync {
    /// Given the current scheduler state and a set of pending entities,
    /// suggest which entity to activate next.
    fn suggest_activation(
        &self,
        state: &SchedulerState,
        pending_entities: &HashSet<EntityTriple>,
    ) -> Option<EntityTriple>;
}
```

---

## Built-in Implementations

### RoundRobin

Selects entities in round-robin order. The cursor is stored in `SchedulerState` (projected from event count). Deterministic — cursor advances based on consumed event count, not wall-clock time.

```rust
pub struct RoundRobin;

impl SchedulingPolicy for RoundRobin {
    fn suggest_activation(
        &self,
        state: &SchedulerState,
        pending_entities: &HashSet<EntityTriple>,
    ) -> Option<EntityTriple> {
        // Sort pending_entities deterministically, then index by
        // state.total_events_consumed % pending_entities.len()
    }
}
```

---

## Contract Tests

Any `SchedulingPolicy` implementation MUST pass:

| Test | Input | Expected |
|------|-------|----------|
| Empty set → None | `pending_entities = {}` | Returns `None` |
| Deterministic | Same `(state, pending_entities)` twice | Same output both times |
| Bounded time | Any valid input | Completes within bounded time |
| No side effects | Any valid input | State unchanged after call |
| Returns member | `pending_entities = {A, B, C}` | Output ∈ {A, B, C} |

---

## Extension Points

Custom policies may be provided by users implementing the `SchedulingPolicy` trait. Common examples:
- **FIFO**: Activate the entity that has been waiting longest (requires tracking enqueue time in state)
- **Priority**: Activate based on entity priority metadata (requires priority field in EntityTriple)
- **Weighted**: Activate based on entity-specific weights in scheduler state
