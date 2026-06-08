use std::collections::HashSet;
use std::fmt::Debug;

use crate::types::EntityTriple;

pub trait SchedulingPolicy: Debug + Send + Sync + 'static {
    fn select_next(
        &self,
        pending: &HashSet<EntityTriple>,
        budget_available: usize,
    ) -> Option<EntityTriple>;

    fn should_preempt(
        &self,
        _new_entity: &EntityTriple,
        _current_target: &EntityTriple,
    ) -> bool {
        false
    }

    fn budget_size(&self) -> usize;

    fn fairness_window(&self) -> u64;
}

#[derive(Debug, Clone)]
pub struct RoundRobinPolicy {
    budget_size: usize,
    fairness_window: u64,
}

impl RoundRobinPolicy {
    pub fn new(budget_size: usize, fairness_window: u64) -> Self {
        Self {
            budget_size,
            fairness_window,
        }
    }
}

impl SchedulingPolicy for RoundRobinPolicy {
    fn select_next(
        &self,
        pending: &HashSet<EntityTriple>,
        budget_available: usize,
    ) -> Option<EntityTriple> {
        if budget_available == 0 || pending.is_empty() {
            return None;
        }
        pending.iter().next().cloned()
    }

    fn budget_size(&self) -> usize {
        self.budget_size
    }

    fn fairness_window(&self) -> u64 {
        self.fairness_window
    }
}
