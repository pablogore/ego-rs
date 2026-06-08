# CORE-007 SchedulingPolicy

## Contract

```rust
pub trait SchedulingPolicy: Send + Sync {
    /// Given the current scheduler state and the set of entities awaiting
    /// activation, returns the entity that should be activated next.
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
| RoundRobin | Cycle through pending entities | Yes | Must store cursor in state |
| FIFO | Activate in arrival order | Yes | Requires arrival timestamp in event |
| PriorityWeighted | Highest priority first | Yes | Must be based on event-derived metrics |
| Random | Random selection | No | Prohibited — non-deterministic |

## Default Policy

RoundRobin with a cursor stored in SchedulerState. The cursor advances deterministically based on event count, not on wall-clock time.
