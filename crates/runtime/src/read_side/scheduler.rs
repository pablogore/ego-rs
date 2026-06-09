//! Tag scheduler — manages per-projection polling and dispatch.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::read_side::backpressure::Backpressure;
use crate::read_side::batch_executor::BatchExecutor;
use ego_domain::read_side::config::ReadSideConfig;
use ego_domain::read_side::dedup::DedupStore;
use ego_domain::read_side::event_tag::EventTag;
use ego_domain::read_side::handler::Handler;
use ego_domain::read_side::offset::OffsetStore;
use ego_domain::read_side::progress::ProgressReporter;
use ego_domain::read_side::scheduler::TagScheduler;
use ego_domain::read_side::store::ReadSideStore;

/// Scheduler for managing tag-based projection processing.
///
/// Handles per-projection polling intervals and dispatches tag streams
/// respecting concurrency limits.
pub struct TagSchedulerImpl<E>
where
    E: Clone + Send + Sync,
{
    config: ReadSideConfig,
    backpressure: Arc<Backpressure>,
    batch_executor: BatchExecutor<E>,
    /// Tracks active projections and their tag processing state
    active_projections: HashMap<String, ProjectionState>,
}

/// State tracking for active projections
struct ProjectionState {
    /// Tags currently being processed
    _active_tags: Vec<EventTag>,
    /// Whether the projection is currently running
    _is_running: bool,
}

impl<E> TagSchedulerImpl<E>
where
    E: Clone + Send + Sync,
{
    /// Creates a new tag scheduler with the given configuration.
    pub fn new(config: ReadSideConfig) -> Self {
        let backpressure = Arc::new(Backpressure::new(config.max_in_flight));
        let batch_executor = BatchExecutor::new(config.clone(), backpressure.clone());

        Self {
            config,
            backpressure,
            batch_executor,
            active_projections: HashMap::new(),
        }
    }
}

#[async_trait]
impl<E> TagScheduler<E> for TagSchedulerImpl<E>
where
    E: Clone + Send + Sync,
{
    async fn start_projection(
        &mut self,
        projection_id: String,
        tags: Vec<EventTag>,
        tenant: String,
        handler: impl Handler<E> + Send + Clone,
        read_store: impl ReadSideStore<E> + Send + Sync + Clone,
        dedup_store: impl DedupStore + Send + Sync + Clone,
        offset_store: impl OffsetStore + Send + Sync + Clone,
        reporter: impl ProgressReporter + Send + Clone,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Store projection state
        self.active_projections.insert(
            projection_id.clone(),
            ProjectionState {
                _active_tags: tags.clone(),
                _is_running: true,
            },
        );

        // Process tags in parallel with backpressure
        for tag in tags {
            // Check if we can process this tag (respect concurrency limits)
            if self.backpressure.can_process().await {
                // Create a session for this tag
                let session = ego_domain::read_side::session::ReadSideSession::new(
                    projection_id.clone(),
                    tag.clone(),
                    tenant.clone(),
                    self.config.clone(),
                    handler.clone(),
                    read_store.clone(),
                    dedup_store.clone(),
                    offset_store.clone(),
                    reporter.clone(),
                );

                // Execute the session
                self.batch_executor.execute_session(session).await?;
            }
        }

        Ok(())
    }
}
