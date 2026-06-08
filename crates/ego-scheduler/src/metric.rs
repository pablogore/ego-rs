//! Observability layer using tracing macros.
//!
//! # Invariants
//! - Metrics are purely diagnostic — no behavioral role
//! - All events logged at debug/info level
//! - No metric is used for scheduling decisions (I7)

use tracing::{info, debug};

/// Logs an event consumption at debug level.
pub fn log_event_consumed(total: u64) {
    debug!(total_events_consumed = total, "Event consumed");
}

/// Logs an activation suggestion at info level.
pub fn log_suggestion(entity: &str) {
    info!(suggested_entity = %entity, "Activation suggestion produced");
}

/// Logs a gap detection at debug level.
pub fn log_gap_detected(gap_count: u64, entity: &str) {
    debug!(detected_gaps = gap_count, source_actor = %entity, "Gap detected in event stream");
}

/// Logs a DropNewest event at debug level.
pub fn log_drop_newest() {
    debug!("Event dropped (DropNewest policy)");
}

/// Logs a DropOldest event at debug level.
pub fn log_drop_oldest() {
    debug!("Oldest event evicted (DropOldest policy)");
}
