//! Read side session — batch execution with metadata-atomic commit.

use std::marker::PhantomData;

use super::config::ReadSideConfig;
use super::dedup::DedupStore;
use super::error::ProjectionError;
use super::event_tag::EventTag;
use super::handler::Handler;
use super::offset::Offset;
use super::offset::OffsetStore;
use super::progress::ProgressReporter;
use super::store::ReadSideStore;

/// A session manages the execution of a single batch of events.
///
/// Phase 1: Fetch events from ReadSideStore
/// Phase 2: Filter duplicates via DedupStore
/// Phase 3: Execute handler
/// Phase 4: Commit offsets and dedup markers atomically
pub struct ReadSideSession<E, H, RS, DS, OS, PR>
where
    E: Clone,
    H: Handler<E>,
    RS: ReadSideStore<E>,
    DS: DedupStore,
    OS: OffsetStore,
    PR: ProgressReporter,
{
    _phantom: PhantomData<E>,
    projection_id: String,
    tag: EventTag,
    tenant: String,
    config: ReadSideConfig,
    handler: H,
    read_store: RS,
    dedup_store: DS,
    offset_store: OS,
    reporter: PR,
}

impl<E, H, RS, DS, OS, PR> ReadSideSession<E, H, RS, DS, OS, PR>
where
    E: Clone,
    H: Handler<E>,
    RS: ReadSideStore<E>,
    DS: DedupStore,
    OS: OffsetStore,
    PR: ProgressReporter,
{
    /// Creates a new session.
    pub fn new(
        projection_id: String,
        tag: EventTag,
        tenant: String,
        config: ReadSideConfig,
        handler: H,
        read_store: RS,
        dedup_store: DS,
        offset_store: OS,
        reporter: PR,
    ) -> Self {
        Self {
            _phantom: std::marker::PhantomData,
            projection_id,
            tag,
            tenant,
            config,
            handler,
            read_store,
            dedup_store,
            offset_store,
            reporter,
        }
    }

    /// Executes one batch: fetch, dedup, handle, commit.
    ///
    /// Returns the new offset after the batch, or an error.
    /// Returns `Ok(None)` if no events were available.
    pub async fn execute(
        &self,
        last_offset: Option<&Offset>,
    ) -> Result<Option<Offset>, ProjectionError> {
        // Phase 1: Fetch events
        let events = self
            .read_store
            .fetch(&self.tag, last_offset, self.config.batch_size)
            .await
            .map_err(|e| ProjectionError::transient(format!("fetch failed: {}", e)))?;

        if events.is_empty() {
            return Ok(None);
        }

        // Phase 2: Filter duplicates
        let mut unique_events = Vec::new();
        for event in &events {
            let is_duplicate = self
                .dedup_store
                .seen(&self.projection_id, &self.tag, event.event_id())
                .await
                .map_err(|e| ProjectionError::transient(format!("dedup check failed: {}", e)))?;

            if !is_duplicate {
                unique_events.push((*event).clone());
            }
        }

        if unique_events.is_empty() {
            return Ok(None);
        }

        // Phase 3: Execute handler
        let result = self.handler.handle(&unique_events).await;

        // Phase 4: Commit
        match result {
            Ok(()) => {
                let new_offset = Offset::sequence(unique_events.last().unwrap().event_version());

                // Mark all events as seen
                for event in &unique_events {
                    self.dedup_store
                        .mark_seen(&self.projection_id, &self.tag, event.event_id())
                        .await
                        .map_err(|e| {
                            ProjectionError::transient(format!("mark dedup failed: {}", e))
                        })?;
                }

                // Write offset
                self.offset_store
                    .write_offset(&self.projection_id, &self.tag, &self.tenant, &new_offset)
                    .await
                    .map_err(|e| {
                        ProjectionError::transient(format!("write offset failed: {}", e))
                    })?;

                // Report progress
                self.reporter.on_batch_completed(
                    &self.projection_id,
                    &self.tag,
                    unique_events.len(),
                    &new_offset,
                );

                Ok(Some(new_offset))
            }
            Err(err) => {
                self.reporter
                    .on_error(&self.projection_id, &format!("{}", err));
                Err(err)
            }
        }
    }
}
