//! Event bus transport layer.
//!
//! # Ownership
//! Channel created by `event_bus_channel()`. Sender is Clone (multi-producer).
//! Receiver owned exclusively by Scheduler (single consumer I6). Dropping receiver
//! closes the channel.
//!
//! # Invariants
//! - I5: DropPolicy deterministic given identical arrival order
//! - I6: Single-consumer bus
//!
//! # Failure Semantics
//! - `try_send` fire-and-forget; SendError is final, no retry
//! - DropPolicy applies strictly at enqueue time

use crate::error::SchedulerError;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use tokio::sync::mpsc;

/// Triple identifier for an entity: tenant, type, and id.
/// Derives Ord for BTreeSet deterministic iteration (I2).
#[derive(Debug, Clone, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct EntityTriple {
    pub tenant: String,
    pub entity_type: String,
    pub entity_id: String,
}

impl EntityTriple {
    /// Creates a new EntityTriple.
    pub fn new(tenant: String, entity_type: String, entity_id: String) -> Self {
        Self {
            tenant,
            entity_type,
            entity_id,
        }
    }
}

/// Events consumed by the scheduler from CORE-006.
#[derive(Debug, Clone, PartialEq)]
pub enum SchedulerEvent {
    /// An entity completed execution.
    ExecutionCompleted {
        entity: EntityTriple,
        state_version: u64,
    },
    /// An entity completed recovery.
    RecoveryCompleted {
        entity: EntityTriple,
        state_version: u64,
    },
}

/// Classification of a scheduler event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    ExecutionCompleted,
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

/// Envelope wrapping a scheduler event with metadata.
/// event_id is a SHA-256 identity annotation — not part of determinism (I1).
#[derive(Debug, Clone, PartialEq)]
pub struct SchedulerEventEnvelope {
    pub event_id: [u8; 32],
    pub sequence_id: u64,
    pub event_type: EventType,
    pub payload: SchedulerEvent,
    pub source_actor: EntityTriple,
}

impl SchedulerEventEnvelope {
    /// Creates a new envelope with a SHA-256 event_id computed from the payload.
    pub fn new(payload: SchedulerEvent, source_actor: EntityTriple, sequence_id: u64) -> Self {
        let event_type = EventType::from(&payload);
        let mut hasher = Sha256::new();
        hasher.update(format!("{:?}", &payload).as_bytes());
        hasher.update(source_actor.tenant.as_bytes());
        hasher.update(source_actor.entity_type.as_bytes());
        hasher.update(source_actor.entity_id.as_bytes());
        hasher.update(sequence_id.to_le_bytes());
        let hash = hasher.finalize();
        let mut event_id = [0u8; 32];
        event_id.copy_from_slice(&hash);
        Self {
            event_id,
            sequence_id,
            event_type,
            payload,
            source_actor,
        }
    }
}

/// An item dequeued from the event bus.
#[derive(Debug, Clone)]
pub struct BusItem {
    pub sequence: u64,
    pub event: SchedulerEventEnvelope,
}

/// Policy for handling buffer overflow at enqueue time (I5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum DropPolicy {
    /// Sender blocks until space available.
    Block,
    /// Incoming event silently dropped.
    DropNewest,
    /// Oldest buffered event evicted.
    ///
    /// NOTE: True eviction requires access to the receiver side, which the
    /// sender does not hold. Current behaviour falls back to DropNewest —
    /// the incoming event is silently accepted (Ok) rather than returning an
    /// error.
    /// TODO #79: implement true DropOldest via shared receiver.
    DropOldest,
}

/// Configuration for the event bus.
#[derive(serde::Deserialize)]
pub struct EventBusConfig {
    pub capacity: usize,
    pub drop_policy: DropPolicy,
}

impl Default for EventBusConfig {
    fn default() -> Self {
        Self {
            capacity: 4096,
            drop_policy: DropPolicy::Block,
        }
    }
}

impl EventBusConfig {
    /// Deserialize a [`serde_json::Value`] into an [`EventBusConfig`].
    ///
    /// Entry point for kit-config integration — callers pass the `Value`
    /// obtained from `kit_config::ConfigLoader` without any direct dependency
    /// on kit-config in this crate.
    pub fn from_value(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value)
    }
}

/// Multi-producer sender for the event bus (I6: Clone for distribution to CORE-006).
#[derive(Clone)]
pub struct SchedulerEventSender {
    sender: mpsc::Sender<BusItem>,
    drop_policy: DropPolicy,
}

impl SchedulerEventSender {
    /// Fire-and-forget send. SendError is final — no retry (I6).
    /// DropPolicy applied strictly at enqueue time.
    pub fn try_send(&self, envelope: SchedulerEventEnvelope) -> Result<(), SchedulerError> {
        let item = BusItem {
            sequence: envelope.sequence_id,
            event: envelope,
        };
        match self.sender.try_send(item) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => match self.drop_policy {
                DropPolicy::Block => Err(SchedulerError::BusFull),
                DropPolicy::DropNewest => Ok(()),
                // TODO #79: implement true DropOldest (requires shared rx).
                // Falls back to DropNewest: silently discard the incoming
                // event rather than returning an error.
                DropPolicy::DropOldest => Ok(()),
            },
            Err(mpsc::error::TrySendError::Closed(_)) => Err(SchedulerError::ChannelClosed),
        }
    }
}

/// Single-consumer receiver owned exclusively by Scheduler (I6).
pub struct SchedulerEventReceiver {
    receiver: mpsc::Receiver<BusItem>,
}

impl SchedulerEventReceiver {
    /// Drains all available events from the bus. Non-blocking.
    pub fn drain_all(&mut self) -> Vec<BusItem> {
        let mut items = Vec::new();
        while let Ok(item) = self.receiver.try_recv() {
            items.push(item);
        }
        items
    }
}

/// Creates a default event bus channel (capacity 4096, Block policy).
pub fn event_bus_channel() -> (SchedulerEventSender, SchedulerEventReceiver) {
    event_bus_channel_with_config(EventBusConfig::default())
}

/// Creates an event bus channel with custom configuration.
pub fn event_bus_channel_with_config(
    config: EventBusConfig,
) -> (SchedulerEventSender, SchedulerEventReceiver) {
    let (sender, receiver) = mpsc::channel(config.capacity);
    let tx = SchedulerEventSender {
        sender,
        drop_policy: config.drop_policy,
    };
    let rx = SchedulerEventReceiver { receiver };
    (tx, rx)
}

/// Creates a BTreeSet of pending entities from an iterator.
pub fn pending_from_iter<I: IntoIterator<Item = EntityTriple>>(iter: I) -> BTreeSet<EntityTriple> {
    iter.into_iter().collect()
}
