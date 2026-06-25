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
            EntityError::MailboxClosed => write!(f, "Mailbox closed"),
        }
    }
}

impl std::error::Error for EntityError {}
