//! State reduction stage — wraps SchedulerState::apply().
//!
//! # Responsibility
//! Calls SchedulerState::apply() for each routed event.
//! Pure function wrapper — no branching logic, no decisions.
//!
//! # Invariant
//! I1: apply() is pure — (Event, S) → S. No entity switch detection inside.

use crate::scheduler::route::RoutedEvent;
use crate::state::SchedulerState;

/// Applies routed events to the scheduler state via pure apply() calls.
pub fn apply(routed: Vec<RoutedEvent>, state: &mut SchedulerState) {
    for r in routed {
        state.apply(&r.item.event);
    }
}
