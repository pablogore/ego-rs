//! Event bus implementation for the scheduler.

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::event::SchedulerEventEnvelope;
use crate::error::SchedulerError;

/// Configuration for the event bus.
#[derive(Debug, Clone)]
pub struct EventBusConfig {
    /// The capacity of the event bus.
    pub capacity: usize,
}

/// Sender for the event bus.
#[derive(Debug, Clone)]
pub struct SchedulerEventSender {
    inner: mpsc::UnboundedSender<SchedulerEventEnvelope>,
}

impl SchedulerEventSender {
    /// Sends an event to the event bus.
    pub fn send(&self, event: SchedulerEventEnvelope) -> Result<(), SchedulerError> {
        self.inner
            .send(event)
            .map_err(|_| SchedulerError::EventBusFull)
    }
}

/// Receiver for the event bus.
pub struct SchedulerEventReceiver {
    inner: mpsc::UnboundedReceiver<SchedulerEventEnvelope>,
}

impl SchedulerEventReceiver {
    /// Receives an event from the event bus.
    pub async fn recv(&mut self) -> Option<SchedulerEventEnvelope> {
        self.inner.recv().await
    }
}

/// Creates a new event bus channel with the default configuration.
pub fn event_bus_channel() -> (SchedulerEventSender, SchedulerEventReceiver) {
    event_bus_channel_with_config(EventBusConfig { capacity: 4096 })
}

/// Creates a new event bus channel with the given configuration.
pub fn event_bus_channel_with_config(
    _config: EventBusConfig,
) -> (SchedulerEventSender, SchedulerEventReceiver) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (
        SchedulerEventSender { inner: sender },
        SchedulerEventReceiver { inner: receiver },
    )
}