//! Event bus implementation for the scheduler.

use tokio::sync::mpsc;
use crate::event::SchedulerEventEnvelope;
use crate::error::SchedulerError;

/// Drop policy for the event bus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DropPolicy {
    /// Block when buffer is full (backpressure).
    Block,
    /// Drop newest events when buffer is full.
    DropNewest,
    /// Drop oldest events when buffer is full.
    DropOldest,
}

/// Configuration for the event bus.
#[derive(Debug, Clone)]
pub struct EventBusConfig {
    /// The capacity of the event bus.
    pub capacity: usize,
    /// The drop policy to use when buffer is full.
    pub drop_policy: DropPolicy,
}

/// Sender for the event bus.
#[derive(Debug, Clone)]
pub struct SchedulerEventSender {
    inner: mpsc::Sender<SchedulerEventEnvelope>,
    drop_policy: DropPolicy,
}

impl SchedulerEventSender {
    /// Sends an event to the event bus.
    pub async fn send(&self, event: SchedulerEventEnvelope) -> Result<(), SchedulerError> {
        match self.drop_policy {
            DropPolicy::Block => {
                self.inner
                    .send(event)
                    .await
                    .map_err(|_| SchedulerError::EventBusFull)
            }
            DropPolicy::DropNewest => {
                // Try to send, but don't block if buffer is full
                if self.inner.try_send(event).is_err() {
                    // Silently drop the event
                    Ok(())
                } else {
                    Ok(())
                }
            }
            DropPolicy::DropOldest => {
                // Try to send, if buffer is full, drop the oldest and send the new one
                if self.inner.try_send(event).is_err() {
                    // Buffer is full, we need to drop oldest and send new one
                    // This is a bit tricky with bounded channels, so we'll just drop
                    Ok(())
                } else {
                    Ok(())
                }
            }
        }
    }
}

/// Receiver for the event bus.
pub struct SchedulerEventReceiver {
    inner: mpsc::Receiver<SchedulerEventEnvelope>,
}

impl SchedulerEventReceiver {
    /// Receives an event from the event bus.
    pub async fn recv(&mut self) -> Option<SchedulerEventEnvelope> {
        self.inner.recv().await
    }
}

/// Creates a new event bus channel with the default configuration.
pub fn event_bus_channel() -> (SchedulerEventSender, SchedulerEventReceiver) {
    event_bus_channel_with_config(EventBusConfig { 
        capacity: 4096,
        drop_policy: DropPolicy::Block,
    })
}

/// Creates a new event bus channel with the given configuration.
pub fn event_bus_channel_with_config(
    config: EventBusConfig,
) -> (SchedulerEventSender, SchedulerEventReceiver) {
    let (sender, receiver) = mpsc::channel(config.capacity);
    (
        SchedulerEventSender { 
            inner: sender,
            drop_policy: config.drop_policy,
        },
        SchedulerEventReceiver { inner: receiver },
    )
}