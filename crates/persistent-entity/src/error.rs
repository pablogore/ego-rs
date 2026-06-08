//! Error types for the persistent entity system.
//!
//! This module defines all the error types that can occur in the persistent entity system.


/// An error that can occur in the persistent entity system.
#[derive(Debug)]
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
}