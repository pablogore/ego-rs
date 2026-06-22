//! Tag scheduler trait.

use async_trait::async_trait;

use crate::read_side::dedup::DedupStore;
use crate::read_side::event_tag::EventTag;
use crate::read_side::handler::Handler;
use crate::read_side::offset::OffsetStore;
use crate::read_side::progress::ProgressReporter;
use crate::read_side::store::ReadSideStore;

/// Scheduler for managing tag-based projection processing.
///
/// Handles per-projection polling intervals and dispatches tag streams
/// respecting concurrency limits.
#[async_trait]
pub trait TagScheduler<E>: Send + Sync
where
    E: Clone + Send + Sync,
{
    /// Starts processing for a projection with the given tags.
    #[allow(clippy::too_many_arguments)]
    async fn start_projection(
        &mut self,
        projection_id: String,
        tags: Vec<EventTag>,
        tenant: String,
        handler: impl Handler<E> + Clone,
        read_store: impl ReadSideStore<E> + Send + Sync + Clone,
        dedup_store: impl DedupStore + Send + Sync + Clone,
        offset_store: impl OffsetStore + Send + Sync + Clone,
        reporter: impl ProgressReporter + Clone,
    ) -> Result<(), Box<dyn std::error::Error>>;
}
