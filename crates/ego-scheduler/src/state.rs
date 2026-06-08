//! Scheduler state management.

use std::collections::{HashMap, VecDeque};
use crate::event::SchedulerEventEnvelope;
use crate::types::EntityTriple;
use crate::gap::GapInfo;

/// The deterministic projection state of the scheduler.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SchedulerState {
    /// Total number of events consumed.
    pub total_events_consumed: u64,

    /// The last sequence ID observed.
    pub last_sequence_id: Option<u64>,

    /// Per-actor sequence tracking.
    pub actor_sequences: HashMap<EntityTriple, u64>,

    /// Detected gaps in sequence IDs.
    pub detected_gaps: Vec<GapInfo>,

    /// Replay buffer for diagnostics (bounded to 1024).
    pub replay_buffer: VecDeque<SchedulerEventEnvelope>,
}

impl SchedulerState {
    /// Creates a new, empty scheduler state.
    pub fn new() -> Self {
        Self {
            total_events_consumed: 0,
            last_sequence_id: None,
            actor_sequences: HashMap::new(),
            detected_gaps: Vec::new(),
            replay_buffer: VecDeque::with_capacity(1024),
        }
    }

    /// Applies an event to the scheduler state.
    pub fn apply_event(&mut self, envelope: &SchedulerEventEnvelope) {
        self.total_events_consumed += 1;
        
        // Update last sequence ID
        self.last_sequence_id = Some(envelope.sequence_id);
        
        // Update actor sequence
        let last_seq = self.actor_sequences.get(&envelope.source_actor).copied().unwrap_or(0);
        if envelope.sequence_id != last_seq + 1 {
            // Gap detected
            let gap_info = GapInfo::new(last_seq, envelope.sequence_id, envelope.source_actor.clone());
            self.detected_gaps.push(gap_info);
        }
        self.actor_sequences.insert(envelope.source_actor.clone(), envelope.sequence_id);
        
        // Update replay buffer (bounded to 1024)
        if self.replay_buffer.len() >= 1024 {
            self.replay_buffer.pop_front();
        }
        self.replay_buffer.push_back(envelope.clone());
    }
}