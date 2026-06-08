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
///
/// # Advisory-Only Semantics
/// - Output is strictly advisory — `suggest_activation` is NOT a command
/// - Policy MUST NOT influence execution directly or indirectly
/// - Execution authority belongs exclusively to CORE-006
pub trait SchedulingPolicy: Send + Sync {
    /// Given the current scheduler state and a set of pending entities,
    /// suggest which entity to activate next.
    ///
    /// # Allowed State Fields
    /// Policy MAY read from `state`:
    /// - `total_events_consumed` (aggregate event counter)
    /// - `last_suggestion` (previous activation suggestion)
    ///
    /// Policy MUST NOT read from:
    /// - `replay_buffer` (diagnostic-only, I4)
    /// - `detected_gaps` (gap tracking, not decision-relevant)
    /// - `last_sequence_id` (per-actor scoped, resets on entity switch)
    /// - `state_hash` (integrity hash, not decision-relevant)
    fn suggest_activation(
        &self,
        state: &SchedulerState,
        pending_entities: &BTreeSet<EntityTriple>,
    ) -> Option<EntityTriple>;
}
```

---

## Built-in Implementations

### RoundRobin

Selects entities in round-robin order. The cursor is derived from `state.total_events_consumed` — every consumed event advances the cursor, regardless of which entity emitted it. Deterministic — cursor advances based on consumed event count, not wall-clock time.

**Fairness model**: Event-driven. Under skewed event distributions (one entity emits many more events than others), high-event-rate entities occupy more cursor positions and are suggested more frequently. This is deterministic and predictable — not a fairness defect. Suggestions are advisory-only (I3); the consumer may accept or ignore any suggestion. For entity-driven or weighted fairness, implement a custom `SchedulingPolicy`.

```rust
pub struct RoundRobin;

impl SchedulingPolicy for RoundRobin {
    fn suggest_activation(
        &self,
        state: &SchedulerState,
        pending_entities: &BTreeSet<EntityTriple>,
    ) -> Option<EntityTriple> {
        // BTreeSet provides deterministic iteration order — no sorting needed.
        // Index by state.total_events_consumed % pending_entities.len()
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
