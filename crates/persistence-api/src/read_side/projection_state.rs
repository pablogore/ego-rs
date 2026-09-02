//! Projection state store SPI.
//!
//! Stores and retrieves projection state information for recovery and rebuild scenarios.

use async_trait::async_trait;
use thiserror::Error;

use super::event_tag::EventTag;
use super::state::ProjectionState;

/// Error type for projection state store operations.
#[derive(Debug, Error)]
pub enum ProjectionStateStoreError {
    /// Transient error (e.g., connection issue).
    #[error("transient projection state store error: {0}")]
    Transient(String),

    /// Fatal error (e.g., data corruption).
    #[error("fatal projection state store error: {0}")]
    Fatal(String),
}

/// Projection state store SPI.
///
/// Stores and retrieves projection state information for recovery and rebuild scenarios.
#[async_trait]
pub trait ProjectionStateStore {
    /// Reads the current state for a projection and tag.
    async fn read_state(
        &self,
        projection_id: &str,
        tag: &EventTag,
    ) -> Result<Option<ProjectionState>, ProjectionStateStoreError>;

    /// Writes the current state for a projection and tag.
    async fn write_state(
        &self,
        projection_id: &str,
        tag: &EventTag,
        state: &ProjectionState,
    ) -> Result<(), ProjectionStateStoreError>;
}
