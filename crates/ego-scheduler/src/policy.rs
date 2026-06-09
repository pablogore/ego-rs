//! Scheduling policy trait and built-in implementations.
//!
//! # Ownership
//! Policy is a pure trait consumed by PolicyEvaluator in the Scheduler pipeline.
//!
//! # Invariants
//! - I3: Output is advisory only — `suggest_activation` is NOT a command
//! - I7: Policy may only read `total_events_consumed` and `last_suggestion`
//! - I2: `pending` is BTreeSet — deterministic iteration, no cross-entity ordering

use crate::event_bus::EntityTriple;
use crate::state::SchedulerState;
use std::collections::BTreeSet;

/// Pure function trait for activation suggestion.
///
/// # Determinism
/// Identical inputs MUST produce identical outputs. No side effects, no I/O, no wall-clock.
///
/// # Advisory-Only (I3)
/// Output is strictly advisory — `suggest_activation` is NOT a command.
/// Policy MUST NOT influence execution directly or indirectly.
///
/// # Allowed Fields (I7)
/// - `total_events_consumed`
/// - `last_suggestion`
///
/// Forbidden: `replay_buffer`, `detected_gaps`, `last_sequence_id`, `state_hash`.
pub trait SchedulingPolicy: Send + Sync {
    fn suggest_activation(
        &self,
        state: &SchedulerState,
        pending: &BTreeSet<EntityTriple>,
    ) -> Option<EntityTriple>;
}

/// Round-robin scheduling policy.
///
/// Event-driven fairness: cursor advances on every consumed event
/// (`total_events_consumed % pending.len()`). BTreeSet provides deterministic
/// iteration — no manual sorting needed. Under skewed event distributions,
/// high-event-rate entities dominate cursor positions — deterministic and predictable.
#[derive(Debug, Clone, Default)]
pub struct RoundRobin;

impl RoundRobin {
    /// Creates a new RoundRobin policy.
    pub fn new() -> Self {
        Self
    }
}

impl SchedulingPolicy for RoundRobin {
    fn suggest_activation(
        &self,
        state: &SchedulerState,
        pending: &BTreeSet<EntityTriple>,
    ) -> Option<EntityTriple> {
        if pending.is_empty() {
            return None;
        }
        let index = (state.total_events_consumed as usize) % pending.len();
        pending.iter().nth(index).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_empty_returns_none() {
        let policy = RoundRobin;
        let state = SchedulerState::new();
        let pending = BTreeSet::new();
        assert!(policy.suggest_activation(&state, &pending).is_none());
    }

    #[test]
    fn contract_returns_member() {
        let policy = RoundRobin;
        let state = SchedulerState::new();
        let mut pending = BTreeSet::new();
        pending.insert(EntityTriple::new("t1".into(), "actor".into(), "a1".into()));
        pending.insert(EntityTriple::new("t1".into(), "actor".into(), "a2".into()));
        let suggestion = policy.suggest_activation(&state, &pending);
        assert!(suggestion.is_some());
        assert!(pending.contains(&suggestion.unwrap()));
    }

    #[test]
    fn contract_deterministic() {
        let policy = RoundRobin;
        let state = SchedulerState::new();
        let mut pending = BTreeSet::new();
        pending.insert(EntityTriple::new("t1".into(), "actor".into(), "a1".into()));
        pending.insert(EntityTriple::new("t1".into(), "actor".into(), "a2".into()));
        let first = policy.suggest_activation(&state, &pending);
        for _ in 0..100 {
            assert_eq!(policy.suggest_activation(&state, &pending), first);
        }
    }

    #[test]
    fn round_robin_rotates_with_consumption() {
        let policy = RoundRobin;
        let mut pending = BTreeSet::new();
        let e1 = EntityTriple::new("t1".into(), "actor".into(), "a1".into());
        let e2 = EntityTriple::new("t1".into(), "actor".into(), "a2".into());
        let e3 = EntityTriple::new("t1".into(), "actor".into(), "a3".into());
        pending.insert(e1.clone());
        pending.insert(e2.clone());
        pending.insert(e3.clone());

        let state0 = SchedulerState::new();
        let s0 = policy.suggest_activation(&state0, &pending).unwrap();

        let mut state1 = SchedulerState::new();
        state1.total_events_consumed = 1;
        let s1 = policy.suggest_activation(&state1, &pending).unwrap();
        assert!(s0 != s1 || pending.len() == 1);

        let mut state2 = SchedulerState::new();
        state2.total_events_consumed = 2;
        let s2 = policy.suggest_activation(&state2, &pending).unwrap();
        assert!(s1 != s2 || pending.len() <= 2);
    }
}
