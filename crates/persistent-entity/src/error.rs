//! Error types for the persistent entity system.
//!
//! This module defines all the error types that can occur in the persistent entity system.

use std::fmt;

/// An error that can occur in the persistent entity system.
#[derive(Debug, Clone)]
pub enum EntityError {
    /// An entity was not found.
    EntityNotFound,
    /// An entity is already active.
    EntityAlreadyActive,
    /// An entity is not active.
    EntityNotActive,
    /// An error occurred during persistence.
    PersistenceError(String),
    /// An error occurred during event publishing.
    EventPublishingError(String),
    /// An error occurred during snapshotting.
    SnapshottingError(String),
    /// An error occurred during recovery.
    RecoveryError(String),
    /// An internal error occurred.
    Internal(String),
    /// The same operation key arrived carrying a different request.
    ///
    /// **Permanent, and deliberately not a [`VersionConflict`].** That variant
    /// reports stream concurrency: two writers raced, the loser reloads and
    /// retries, and its two version numbers tell the caller where it stands.
    /// This one has no versions to report and no retry that could help — the
    /// key is already bound to another request, and it stays bound. Collapsing
    /// the two would tell a caller to retry something that will never succeed,
    /// and would hide a client-side key-reuse bug inside a routine race.
    ///
    /// [`VersionConflict`]: EntityError::VersionConflict
    OperationConflict {
        /// The operation key that is already bound to a different request.
        operation_key: String,
    },
    /// A version conflict occurred.
    VersionConflict {
        /// The expected version.
        expected: u64,
        /// The actual version.
        actual: u64,
    },
    /// The mailbox was closed and the queue is fully drained.
    MailboxClosed,
}

impl fmt::Display for EntityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityError::EntityNotFound => write!(f, "Entity not found"),
            EntityError::EntityAlreadyActive => write!(f, "Entity already active"),
            EntityError::EntityNotActive => write!(f, "Entity not active"),
            EntityError::PersistenceError(msg) => write!(f, "Persistence error: {}", msg),
            EntityError::EventPublishingError(msg) => write!(f, "Event publishing error: {}", msg),
            EntityError::SnapshottingError(msg) => write!(f, "Snapshotting error: {}", msg),
            EntityError::RecoveryError(msg) => write!(f, "Recovery error: {}", msg),
            EntityError::Internal(msg) => write!(f, "Internal error: {}", msg),
            EntityError::VersionConflict { expected, actual } => write!(
                f,
                "Version conflict: expected {}, actual {}",
                expected, actual
            ),
            EntityError::OperationConflict { operation_key } => write!(
                f,
                "Operation conflict: operation key {:?} is already bound to a different \
                 request; retrying will not resolve it",
                operation_key
            ),
            EntityError::MailboxClosed => write!(f, "Mailbox closed"),
        }
    }
}

impl std::error::Error for EntityError {}
