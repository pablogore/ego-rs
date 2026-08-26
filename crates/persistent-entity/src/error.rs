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

/// A composition was declared production and a persistent capability it uses
/// has no explicitly configured implementation.
///
/// Deliberately not an [`EntityError`] variant: `EntityError` reports what
/// went wrong while an entity was *running* a command. This one reports that
/// the runtime must not be built at all, and nothing that handles a command
/// failure should have to consider it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PersistenceCompositionError {
    /// `Profile::Production` is declared but `capability` has no configured
    /// implementation. `fix` names the exact call that configures it.
    #[error(
        "Profile::Production is declared but no {capability} is configured — a \
         production composition must never fall back to volatile storage. \
         Configure one with {fix}, or state that this composition is not \
         production with .profile(Profile::Dev)"
    )]
    NotConfigured {
        /// The capability with no configured implementation.
        capability: &'static str,
        /// The exact call that fixes it.
        fix: &'static str,
    },
}

#[cfg(test)]
mod persistence_composition_error_tests {
    use super::*;

    /// Mirrors PROD-012's `the_refusal_names_the_registration_and_the_opt_out`:
    /// the message must name both the missing capability and the exact call
    /// that fixes it, not just that something is wrong (IS-7).
    #[test]
    fn the_refusal_names_the_capability_and_the_fix() {
        let message = PersistenceCompositionError::NotConfigured {
            capability: "event store",
            fix: "EntityRuntimeBuilder::with_event_store(store)",
        }
        .to_string();

        assert!(
            message.contains("event store"),
            "the error must name the missing capability: {message}"
        );
        assert!(
            message.contains("EntityRuntimeBuilder::with_event_store(store)"),
            "the error must name the exact fixing call: {message}"
        );
    }
}
