//! Event ingestion stage — drains the event bus only.
//!
//! # Responsibility
//! Solely responsible for draining events from the bus.
//! No logic, no state mutation.

use crate::event_bus::{BusItem, SchedulerEventReceiver};

/// Drains all available events from the receiver.
pub fn drain(receiver: &mut SchedulerEventReceiver) -> Vec<BusItem> {
    receiver.drain_all()
}
