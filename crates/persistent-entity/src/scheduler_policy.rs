use std::collections::HashSet;
use std::fmt::Debug;

use crate::scheduler::EntityTriple;

pub trait SchedulingPolicy: Debug + Send + Sync + 'static {
    /// Suggest which entity should be activated next.
    ///
    /// This is a pure advisory recommendation. The Actor is the
    /// sole execution authority and may ignore this suggestion.
    fn suggest_activation(&self, pending: &HashSet<EntityTriple>) -> Option<EntityTriple>;

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
    fn suggest_activation(&self, pending: &HashSet<EntityTriple>) -> Option<EntityTriple> {
        if pending.is_empty() {
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
