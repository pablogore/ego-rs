//! Persistence error types.
//!
//! Defines `PersistenceError` with four variants covering the standard
//! failure modes of event-sourced persistence.

/// Error types for persistence operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PersistenceError {
    /// The requested aggregate was not found.
    #[error("aggregate '{}' not found", aggregate_id)]
    NotFound { aggregate_id: String },

    /// Optimistic concurrency conflict: expected version does not match actual.
    #[error("conflict for aggregate '{}': expected version {}, actual version {}", aggregate_id, expected, actual)]
    Conflict {
        aggregate_id: String,
        expected: i64,
        actual: i64,
    },

    /// A required tenant identifier is missing.
    #[error("missing tenant identifier")]
    MissingTenant,

    /// An internal/unexpected error occurred.
    #[error("internal error: {}", _0)]
    Internal(String),
}
