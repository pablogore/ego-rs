//! Suggestion emission stage — writes last_suggestion only.
//!
//! # Responsibility
//! Writes the advisory suggestion to SchedulerState. No logic, no decisions.
//!
//! # Invariant
//! I3: Output advisory only — `suggest_activation` is NOT a command.
//! Execution authority belongs exclusively to CORE-006.

use crate::event_bus::EntityTriple;
use crate::metric;
use crate::state::SchedulerState;

/// Emits an advisory suggestion by writing to SchedulerState (I3).
pub fn emit(state: &mut SchedulerState, suggestion: Option<EntityTriple>) {
    if let Some(ref entity) = suggestion {
        metric::log_suggestion(&entity.entity_id);
    }
    state.last_suggestion = suggestion;
}
