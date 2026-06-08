//! Scheduler events and related types.

use crate::types::EntityTriple;

/// Events that the scheduler consumes from CORE-006.
#[derive(Debug, Clone, PartialEq)]
pub enum SchedulerEvent {
    /// An execution has completed.
    ExecutionCompleted {
        /// The entity that completed execution.
        entity: EntityTriple,
        /// The state version after execution.
        state_version: u64,
    },
    /// A recovery has completed.
    RecoveryCompleted {
        /// The entity that completed recovery.
        entity: EntityTriple,
        /// The state version after recovery.
        state_version: u64,
    },
}

/// Classification of the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    /// Execution completed event.
    ExecutionCompleted,
    /// Recovery completed event.
    RecoveryCompleted,
}

impl From<&SchedulerEvent> for EventType {
    fn from(event: &SchedulerEvent) -> Self {
        match event {
            SchedulerEvent::ExecutionCompleted { .. } => EventType::ExecutionCompleted,
            SchedulerEvent::RecoveryCompleted { .. } => EventType::RecoveryCompleted,
        }
    }
}

/// Wrapper emitted from CORE-006 into the bounded event bus.
#[derive(Debug, Clone, PartialEq)]
pub struct SchedulerEventEnvelope {
    /// Deterministic SHA-256 hash of canonical payload.
    /// Identity annotation only — NOT part of determinism proof.
    pub event_id: [u8; 32],

    /// Monotonically increasing per-Actor stream.
    pub sequence_id: u64,

    /// Classification of the event.
    pub event_type: EventType,

    /// Event-specific structured data.
    pub payload: SchedulerEvent,

    /// The Actor that emitted this event.
    pub source_actor: EntityTriple,
}

impl SchedulerEventEnvelope {
    /// Creates a new SchedulerEventEnvelope.
    pub fn new(
        payload: SchedulerEvent,
        source_actor: EntityTriple,
        sequence_id: u64,
    ) -> Self {
        // In a real implementation, this would compute a SHA-256 hash of the payload
        // For now, we'll use a placeholder
        let mut event_id = [0u8; 32];
        event_id[0] = sequence_id as u8;
        event_id[1] = source_actor.tenant.as_bytes()[0].wrapping_add(source_actor.entity_type.as_bytes()[0]);
        
        Self {
            event_id,
            sequence_id,
            event_type: (&payload).into(),
            payload,
            source_actor,
        }
    }
}