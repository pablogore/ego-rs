//! Deduplication store SPI.

use async_trait::async_trait;
use thiserror::Error;

use super::event_tag::EventTag;

/// Error type for dedup store operations.
#[derive(Debug, Error)]
pub enum DedupStoreError {
    /// Transient error (e.g., connection issue).
    #[error("transient dedup store error: {0}")]
    Transient(String),

    /// Fatal error (e.g., data corruption).
    #[error("fatal dedup store error: {0}")]
    Fatal(String),
}

/// Deduplication store SPI.
///
/// Deduplication scope: (projection_id, tag, event_id).
/// Replay dedup is ON by default.
#[async_trait]
pub trait DedupStore {
    /// Checks if an event has already been seen.
    async fn seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<bool, DedupStoreError>;

    /// Marks an event as seen.
    async fn mark_seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<(), DedupStoreError>;
}
