//! Policy evaluation stage — calls SchedulingPolicy::suggest_activation only.
//!
//! # Responsibility
//! Invokes the pure SchedulingPolicy function. No side effects, no decisions.
//!
//! # Invariant
//! I3: Output advisory only. I7: Policy reads only allowed fields.

use crate::event_bus::EntityTriple;
use crate::policy::SchedulingPolicy;
use crate::state::SchedulerState;
use std::collections::BTreeSet;

/// Evaluates the scheduling policy with current state and pending entities.
pub fn evaluate(
    policy: &dyn SchedulingPolicy,
    state: &SchedulerState,
    pending: &BTreeSet<EntityTriple>,
) -> Option<EntityTriple> {
    policy.suggest_activation(state, pending)
}
