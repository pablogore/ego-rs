//! Batch executor — orchestrates read-side processing with backpressure.

use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::read_side::config::ReadSideConfig;
use ego_domain::read_side::dedup::DedupStore;
use ego_domain::read_side::handler::Handler;

use ego_domain::read_side::offset::OffsetStore;
use ego_domain::read_side::progress::ProgressReporter;
use ego_domain::read_side::session::ReadSideSession;
use ego_domain::read_side::store::ReadSideStore;

/// Executes read-side sessions with backpressure control.
pub struct BatchExecutor<E>
where
    E: Clone + Send,
{
    backpressure: Arc<crate::read_side::backpressure::Backpressure>,
    _phantom: PhantomData<E>,
}

impl<E> BatchExecutor<E>
where
    E: Clone + Send,
{
    /// Creates a new batch executor with the given configuration and backpressure.
    pub fn new(
        _config: ReadSideConfig,
        backpressure: Arc<crate::read_side::backpressure::Backpressure>,
    ) -> Self {
        Self {
            backpressure,
            _phantom: PhantomData,
        }
    }

    /// Executes a read-side session with backpressure constraints.
    pub async fn execute_session<S, H, DS, OS, PR>(
        &self,
        session: ReadSideSession<E, H, S, DS, OS, PR>,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        S: ReadSideStore<E> + Send + Sync,
        H: Handler<E> + Send,
        DS: DedupStore + Send + Sync,
        OS: OffsetStore + Send + Sync,
        PR: ProgressReporter + Send,
    {
        // Acquire backpressure permit
        let _permit = self.backpressure.acquire().await?;

        // Execute the session (reads the persisted offset internally).
        session.execute().await?;

        Ok(())
    }
}
