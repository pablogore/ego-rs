# CORE-007 SchedulingPolicy

## Contract

```rust
pub trait SchedulingPolicy: Send + Sync {
    /// Given the current scheduler state and the set of entities awaiting
    /// activation, returns the entity that should be activated next.
    ///
    /// `pending` is an **unordered set** of entity identities. Policies
    /// MUST NOT treat it as a globally ordered queue. Entity selection
    /// is based on entity identity, not on cross-entity event ordering.
    ///
    /// # Determinism
    ///
    /// - MUST be a pure function (no side effects)
    /// - MUST produce identical output for identical (state, pending) inputs
    /// - MUST NOT depend on wall-clock time, random numbers, or external state
    ///
    /// # Bounded Execution
    ///
    /// - MUST complete in O(pending) time or better
    /// - MUST NOT block or yield
    ///
    /// # No Cross-Entity Ordering
    ///
    /// - MUST NOT compare `sequence_id` values across entity boundaries
    /// - Cross-entity ordering semantics, if needed, exist outside this policy
    fn suggest_activation(
        &self,
        state: &SchedulerState,
        pending: &HashSet<EntityTriple>,
    ) -> Option<EntityTriple>;
}
```

## Policy Examples

| Policy | Strategy | Deterministic | Notes |
|--------|----------|---------------|-------|
| RoundRobin | Cycle through pending entities | Yes | Must store cursor in state. Selects by entity identity only — does NOT use cross-entity sequence_id |
| FIFO | Activate in entity selection order | Yes | Selects entity based on identity order in pending set, not on cross-entity event sequence. Prohibited from using cross-entity `sequence_id` comparisons |
| PriorityWeighted | Highest priority first | Yes | Must be based on event-derived metrics |
| Random | Random selection | No | Prohibited — non-deterministic |

**HARD INVARIANT**: All policies operate over the `pending_entities` set as an unordered entity identity set. Policies MUST NOT treat `pending_entities` as a globally ordered queue. Cross-entity `sequence_id` comparison is prohibited in all policy implementations (see spec Section 14.2 and Section 17).

## Default Policy

RoundRobin with a cursor stored in SchedulerState. The cursor advances deterministically based on entity identity order, not on cross-entity event ordering or wall-clock time.
