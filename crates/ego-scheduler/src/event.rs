//! Scheduler events and related types.

use crate::types::EntityTriple;
use sha2::{Sha256, Digest};

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
        // Create canonical representation of the payload for hashing
        let canonical_payload = match &payload {
            SchedulerEvent::ExecutionCompleted { entity, state_version } => {
                format!("ExecutionCompleted:{}:{}:{}:{}", entity.tenant, entity.entity_type, entity.entity_id, state_version)
            },
            SchedulerEvent::RecoveryCompleted { entity, state_version } => {
                format!("RecoveryCompleted:{}:{}:{}:{}", entity.tenant, entity.entity_type, entity.entity_id, state_version)
            },
        };
        
        // Compute SHA-256 hash of canonical payload
        let mut hasher = Sha256::new();
        hasher.update(canonical_payload.as_bytes());
        let result = hasher.finalize();
        let mut event_id = [0u8; 32];
        event_id.copy_from_slice(&result[..32]);
        
        Self {
            event_id,
            sequence_id,
            event_type: (&payload).into(),
            payload,
            source_actor,
        }
    }
}