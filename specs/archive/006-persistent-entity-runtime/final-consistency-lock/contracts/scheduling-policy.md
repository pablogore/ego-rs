# Contract: SchedulingPolicy

**Feature**: `006-persistent-entity-runtime/final-consistency-lock`
**Created**: 2026-06-07

The SchedulingPolicy trait defines the Scheduler's decision logic. It is stateless — all state is held externally by the Scheduler struct. Policy implementations are deterministic: same inputs → same selection output.

## Trait Definition

```rust
/// Stateless scheduling policy contract.
///
/// The policy makes the "who goes next" decision for entity activation.
/// It receives the current pending set and available budget slots,
/// and returns the next entity to activate (or None).
///
/// All methods are deterministic — they do not depend on wall-clock
/// time, random numbers, or runtime state.
pub trait SchedulingPolicy: Debug + Send + Sync + 'static {
    /// Select the next entity to activate from the pending set.
    ///
    /// # Arguments
    /// * `pending` - Set of entities with queued commands awaiting activation
    /// * `budget_available` - Number of free budget slots
    ///
    /// # Returns
    /// The next entity triple to activate, or None if no eligible entities.
    fn select_next(
        &self,
        pending: &HashSet<EntityTriple>,
        budget_available: usize,
    ) -> Option<EntityTriple>;

    /// Whether a newly arriving entity should preempt the currently
    /// scheduled activation.
    fn should_preempt(
        &self,
        new_entity: &EntityTriple,
        current_target: &EntityTriple,
    ) -> bool;

    /// Get the configured concurrency budget size.
    fn budget_size(&self) -> usize;

    /// Get the fairness window: max number of scheduling decisions
    /// an entity may wait before guaranteed activation.
    fn fairness_window(&self) -> u64;
}
```

## Default Implementation: RoundRobinPolicy

```rust
/// Simple round-robin fairness policy.
///
/// Entities are activated in FIFO arrival order with fairness enforcement:
/// if any entity has waited longer than `fairness_window` scheduling decisions,
/// it is promoted to the front of the queue.
#[derive(Debug, Clone)]
pub struct RoundRobinPolicy {
    budget_size: usize,
    fairness_window: u64,
}

impl RoundRobinPolicy {
    pub fn new(budget_size: usize, fairness_window: u64) -> Self { ... }
}

impl SchedulingPolicy for RoundRobinPolicy {
    fn select_next(
        &self,
        pending: &HashSet<EntityTriple>,
        budget_available: usize,
    ) -> Option<EntityTriple> {
        // Scheduler maintains activation_queue (VecDeque in FIFO order)
        // and fairness_tracker (HashMap<EntityTriple, u64> of decisions waited)
        
        // 1. Check fairness circuit-breaker:
        //    - Find entity with fairness_tracker > fairness_window
        //    - If found, return that entity (promoted to front)
        // 2. Otherwise, pop from front of activation_queue
        // 3. If budget_available == 0, return None
        None // implementation detail — logic lives in Scheduler struct
    }

    fn should_preempt(&self, ..) -> bool { false }
    fn budget_size(&self) -> usize { self.budget_size }
    fn fairness_window(&self) -> u64 { self.fairness_window }
}
```

## Policy Invariants

1. **Deterministic**: Given the same `(pending_set, arrival_order, fairness_tracker, budget_available)`, `select_next` MUST return the same entity every time.

2. **No starvation**: Every entity in `pending` MUST eventually be selected within `fairness_window` scheduling decisions (enforced by circuit-breaker).

3. **Stateless**: The policy struct MUST NOT hold mutable scheduling state. All state lives in the Scheduler.

4. **Backend-agnostic**: The policy MUST NOT reference ExecutionBackend, Actor state, or Event Store.

5. **FIFO per entity**: Within a single entity, command ordering is the Actor's responsibility — the policy only selects WHICH entity to activate, not which command to process.
