//! Read side store SPI — fetches events by tag.

use async_trait::async_trait;
use thiserror::Error;

use super::event_stream::EventStreamElement;
use super::event_tag::EventTag;
use super::offset::Offset;

/// Error type for read side store operations.
#[derive(Debug, Error)]
pub enum ReadSideStoreError {
    /// Transient error (e.g., connection issue).
    #[error("transient read side store error: {0}")]
    Transient(String),

    /// Fatal error (e.g., data corruption).
    #[error("fatal read side store error: {0}")]
    Fatal(String),
}

/// Read-optimized event store interface for tag-based projection consumption.
///
/// Separate from `EventStore`. Fetches events by tag with offset-based pagination.
#[async_trait]
pub trait ReadSideStore<E> {
    /// Fetches up to `batch_size` events for `tenant` under `tag`, starting
    /// after `offset`.
    ///
    /// Returns events with `event_version > offset` in ascending version order.
    /// If offset is `None`, returns from the beginning (used in replay).
    ///
    /// Tenant isolation is type-enforced by this parameter rather than left to
    /// the convention of folding the tenant into `tag`: an implementation MUST
    /// only return events whose `tenant_id` equals `tenant`, regardless of how
    /// `tag` was constructed. An empty `tenant` MUST return no events (fail
    /// closed) — a missing tenant must never silently surface another tenant's
    /// data.
    ///
    /// # Arguments
    /// * `tenant` - The tenant to scope the fetch to. Empty means "no tenant",
    ///   which returns nothing.
    /// * `tag` - The tag to fetch events for
    /// * `offset` - Optional offset (last processed event_version). `None` means from beginning.
    /// * `batch_size` - Maximum number of events to return
    ///
    /// # Returns
    /// A vector of `EventStreamElement` sorted by `event_version` ascending.
    async fn fetch(
        &self,
        tenant: &str,
        tag: &EventTag,
        offset: Option<&Offset>,
        batch_size: usize,
    ) -> Result<Vec<EventStreamElement<E>>, ReadSideStoreError>;
}
