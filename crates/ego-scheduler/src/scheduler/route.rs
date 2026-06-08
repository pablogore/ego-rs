//! Entity routing stage — detects entity switches only.
//!
//! # Responsibility
//! Detects entity switches via `current_entity != event.source_actor`.
//! Resets per-entity scoped fields on switch. No state mutation beyond reset.
//!
//! # Invariant
//! I2: No cross-entity ordering — entity switch detection is Scheduler-owned,
//! not inside SchedulerState::apply().

use crate::event_bus::{BusItem, EntityTriple};
use crate::state::SchedulerState;

/// A routed event with entity switch information.
pub struct RoutedEvent {
    pub item: BusItem,
    /// Whether this event caused an entity switch.
    #[allow(dead_code)]
    pub entity_switched: bool,
}

/// Routes events, detecting entity switches and resetting per-entity fields (I2).
pub fn route(
    events: Vec<BusItem>,
    state: &mut SchedulerState,
    current_entity: &mut Option<EntityTriple>,
) -> Vec<RoutedEvent> {
    let mut routed = Vec::with_capacity(events.len());
    for item in events {
        let source = &item.event.source_actor;
        let switched = match current_entity {
            Some(ref current) if current != source => {
                *current_entity = Some(source.clone());
                state.last_sequence_id = None;
                state.detected_gaps = 0;
                true
            }
            None => {
                *current_entity = Some(source.clone());
                false
            }
            _ => false,
        };
        routed.push(RoutedEvent { item, entity_switched: switched });
    }
    routed
}
