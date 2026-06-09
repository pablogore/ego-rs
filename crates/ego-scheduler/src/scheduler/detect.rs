//! Gap detection stage — structural gap check only.
//!
//! # Responsibility
//! Detects gaps via `sequence_id != last + 1`. Increments counter only.
//! No classification, no causal inference, no recovery.
//!
//! # Invariant
//! Uniform treatment — all gaps treated identically. No per-cause attribution.

use crate::event_bus::BusItem;
use crate::metric;
use crate::state::SchedulerState;

/// Detects sequence gaps and increments the counter (structural only).
pub fn detect(events: &[BusItem], state: &mut SchedulerState) {
    for item in events {
        let seq = item.event.sequence_id;
        if let Some(last) = state.last_sequence_id {
            if seq != last + 1 {
                state.detected_gaps += 1;
                metric::log_gap_detected(state.detected_gaps, &item.event.source_actor.entity_id);
            }
        }
        state.last_sequence_id = Some(seq);
    }
}
