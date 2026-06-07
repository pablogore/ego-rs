//! Handler trait — processes batches of events.

use async_trait::async_trait;

use super::error::ProjectionError;
use super::event_stream::EventStreamElement;

/// Processes a batch of events for a projection.
///
/// Implementations should be idempotent where possible.
/// Errors are classified as Transient, Fatal, or PoisonEvent.
#[async_trait]
pub trait Handler<E>: Send + Sync {
    /// Processes a batch of events.
    ///
    /// Returns `Ok(())` on success, or a `ProjectionError` classifying the failure.
    async fn handle(&self, events: &[EventStreamElement<E>]) -> Result<(), ProjectionError>;
}
