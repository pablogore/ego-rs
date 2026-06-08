//! Scheduling policy trait and implementations.

use std::collections::HashSet;
use crate::state::SchedulerState;
use crate::types::EntityTriple;

/// A scheduling policy that produces advisory activation suggestions.
///
/// This is a pure function that takes the current state and a set of pending entities
/// and returns an optional entity to activate.
pub trait SchedulingPolicy: Send + Sync {
    /// Suggests an entity to activate based on the current state and pending entities.
    ///
    /// This function is deterministic and pure - it must produce the same result
    /// given the same inputs.
    fn suggest_activation(
        &self,
        state: &SchedulerState,
        pending_entities: &HashSet<EntityTriple>,
    ) -> Option<EntityTriple>;
}

/// A round-robin scheduling policy.
///
/// This policy cycles through pending entities deterministically.
#[derive(Debug, Clone)]
pub struct RoundRobin;

impl SchedulingPolicy for RoundRobin {
    fn suggest_activation(
        &self,
        state: &SchedulerState,
        pending_entities: &HashSet<EntityTriple>,
    ) -> Option<EntityTriple> {
        if pending_entities.is_empty() {
            return None;
        }

        // Convert to sorted vector for deterministic ordering
        let mut sorted: Vec<EntityTriple> = pending_entities.iter().cloned().collect();
        sorted.sort_by(|a, b| {
            a.tenant.cmp(&b.tenant)
                .then_with(|| a.entity_type.cmp(&b.entity_type))
                .then_with(|| a.entity_id.cmp(&b.entity_id))
        });

        // Determine index based on total events consumed
        let index = state.total_events_consumed as usize % sorted.len();
        sorted.get(index).cloned()
    }
}