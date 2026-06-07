//! Read side processor trait.

use async_trait::async_trait;

use super::error::ProjectionError;
use super::event_tag::EventTag;
use super::offset::Offset;
use super::state::ProjectionState;

/// Orchestrates the read-side projection lifecycle for a single projection.
///
/// Coordinates between the scheduler (which decides what to process),
/// the handler (which processes events), and the stores (which track state).
#[async_trait]
pub trait ReadSideProcessor<E>: Send + Sync {
    /// Returns the projection ID.
    fn projection_id(&self) -> &str;

    /// Returns the current state.
    fn state(&self) -> ProjectionState;

    /// Starts or resumes the projection.
    async fn start(&mut self) -> Result<(), ProjectionError>;

    /// Pauses the projection.
    async fn pause(&mut self) -> Result<(), ProjectionError>;

    /// Stops the projection and cleans up resources.
    async fn stop(&mut self) -> Result<(), ProjectionError>;

    /// Processes one batch of events for the given tag.
    ///
    /// Returns the new offset after processing, or an error.
    async fn process_batch(
        &mut self,
        tag: &EventTag,
        offset: Option<&Offset>,
    ) -> Result<Option<Offset>, ProjectionError>;
}
