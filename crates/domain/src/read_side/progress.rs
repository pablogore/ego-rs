//! Progress reporting SPI.

use super::event_tag::EventTag;
use super::offset::Offset;
use super::state::ProjectionState;

/// Error type for progress reporter operations.
#[derive(Debug, thiserror::Error)]
pub enum ProgressReporterError {
    /// Transient error (e.g., network issue to metrics backend).
    #[error("transient progress reporter error: {0}")]
    Transient(String),

    /// Fatal error (e.g., misconfigured reporter).
    #[error("fatal progress reporter error: {0}")]
    Fatal(String),
}

/// Progress reporting SPI for runtime observability.
///
/// Host injects implementation at runner construction.
/// All methods MUST be non-blocking (or as fast as possible).
/// The runtime MUST NOT depend on the reporter for correctness.
#[async_trait::async_trait]
pub trait ProgressReporter: Send + Sync {
    /// Called after each successful batch commit.
    fn on_batch_completed(
        &self,
        projection_id: &str,
        tag: &EventTag,
        count: usize,
        offset: &Offset,
    ) {
        let _ = (projection_id, tag, count, offset);
    }

    /// Called on transient, fatal, and poison event errors.
    fn on_error(&self, projection_id: &str, error: &str) {
        let _ = (projection_id, error);
    }

    /// Called on every state change.
    fn on_state_transition(
        &self,
        projection_id: &str,
        from: ProjectionState,
        to: ProjectionState,
    ) {
        let _ = (projection_id, from, to);
    }
}

/// A no-op implementation of `ProgressReporter`.
///
/// Use this when no observability is needed.
pub struct NoopProgressReporter;

#[async_trait::async_trait]
impl ProgressReporter for NoopProgressReporter {
    fn on_batch_completed(
        &self,
        _projection_id: &str,
        _tag: &EventTag,
        _count: usize,
        _offset: &Offset,
    ) {
    }

    fn on_error(&self, _projection_id: &str, _error: &str) {}

    fn on_state_transition(
        &self,
        _projection_id: &str,
        _from: ProjectionState,
        _to: ProjectionState,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_reporter_does_not_panic() {
        let reporter = NoopProgressReporter;
        let tag = EventTag::new("test");
        let offset = Offset::sequence(42);

        reporter.on_batch_completed("proj-1", &tag, 10, &offset);
        reporter.on_error("proj-1", "test error");
        reporter.on_state_transition("proj-1", ProjectionState::Running, ProjectionState::Paused);
    }
}
