//! Deterministic projection state.
//!
//! # Ownership
//! SchedulerState is a pure data container, not a runtime engine.
//! Owned and mutated exclusively by the Scheduler pipeline.
//!
//! # Invariants
//! - I1: Pure function of observed stream f(observed_stream)
//! - I2: Single-stream model — tracks one entity at a time
//! - I4: ReplayBuffer excluded from PartialEq; non-semantic
//!
//! # Failure Semantics
//! - `apply()` is pure — no side effects, no entity switch detection

use crate::event_bus::{EntityTriple, SchedulerEventEnvelope};
use std::collections::VecDeque;

/// Deterministic projection state of the scheduler.
/// Single-stream model: tracks exactly one entity's projection at a time (I2).
#[derive(Debug, Clone)]
pub struct SchedulerState {
    /// Lifetime count across all entities (I7: allowed policy input).
    pub total_events_consumed: u64,
    /// Most recent sequence_id of the currently projected entity. Resets on entity switch (I2).
    pub last_sequence_id: Option<u64>,
    /// Gap count for the current entity's stream segment. Resets on entity switch (I2).
    pub detected_gaps: u64,
    /// Most recent advisory suggestion (I7: allowed policy input).
    pub last_suggestion: Option<EntityTriple>,
    /// Optional snapshot hash for integrity.
    pub state_hash: Option<[u8; 32]>,
    /// Bounded diagnostic buffer (1024). Non-semantic — excluded from PartialEq (I4).
    pub replay_buffer: VecDeque<(u64, SchedulerEventEnvelope)>,
}

impl SchedulerState {
    /// Creates a new empty SchedulerState.
    pub fn new() -> Self {
        Self {
            total_events_consumed: 0,
            last_sequence_id: None,
            detected_gaps: 0,
            last_suggestion: None,
            state_hash: None,
            replay_buffer: VecDeque::new(),
        }
    }

    /// Pure projection function: (Event, S) → S (I1).
    /// Performs state transformation only — no entity switch detection,
    /// no reset logic beyond field updates, no orchestration.
    pub fn apply(&mut self, envelope: &SchedulerEventEnvelope) {
        self.total_events_consumed += 1;
        self.last_sequence_id = Some(envelope.sequence_id);

        if self.replay_buffer.len() >= 1024 {
            self.replay_buffer.pop_front();
        }
        self.replay_buffer
            .push_back((envelope.sequence_id, envelope.clone()));
    }
}

/// Manual PartialEq excludes replay_buffer (I4).
/// Two states with identical semantic fields are equivalent regardless of buffer content.
impl PartialEq for SchedulerState {
    fn eq(&self, other: &Self) -> bool {
        self.total_events_consumed == other.total_events_consumed
            && self.last_sequence_id == other.last_sequence_id
            && self.detected_gaps == other.detected_gaps
            && self.last_suggestion == other.last_suggestion
            && self.state_hash == other.state_hash
    }
}

impl Default for SchedulerState {
    fn default() -> Self {
        Self::new()
    }
}
