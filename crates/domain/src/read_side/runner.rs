//! Read side runner — fetches, batches, and delivers events.

use super::config::ReadSideConfig;
use super::dedup::DedupStore;
use super::event_tag::EventTag;
use super::handler::Handler;
use super::offset::OffsetStore;
use super::progress::ProgressReporter;
use super::session::ReadSideSession;
use super::store::ReadSideStore;

/// Orchestrates read-side projections.
///
/// Manages the lifecycle of processors and coordinates between
/// the scheduler (which decides what to process) and the session
/// (which executes batches).
pub struct ReadSideRunner<E, H, RS, DS, OS, PR>
where
    E: Clone,
    H: Handler<E>,
    RS: ReadSideStore<E>,
    DS: DedupStore,
    OS: OffsetStore,
    PR: ProgressReporter,
{
    config: ReadSideConfig,
    _phantom: std::marker::PhantomData<(E, H, RS, DS, OS, PR)>,
}

impl<E, H, RS, DS, OS, PR> ReadSideRunner<E, H, RS, DS, OS, PR>
where
    E: Clone,
    H: Handler<E>,
    RS: ReadSideStore<E>,
    DS: DedupStore,
    OS: OffsetStore,
    PR: ProgressReporter,
{
    /// Creates a new runner with the given configuration.
    pub fn new(config: ReadSideConfig) -> Self {
        Self {
            config,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Creates a new session for a projection and tag.
    pub fn create_session(
        &self,
        projection_id: String,
        tag: EventTag,
        tenant: String,
        handler: H,
        read_store: RS,
        dedup_store: DS,
        offset_store: OS,
        reporter: PR,
    ) -> ReadSideSession<E, H, RS, DS, OS, PR> {
        ReadSideSession::new(
            projection_id,
            tag,
            tenant,
            self.config.clone(),
            handler,
            read_store,
            dedup_store,
            offset_store,
            reporter,
        )
    }
}
