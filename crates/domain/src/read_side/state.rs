//! Projection state machine.

use serde::{Deserialize, Serialize};

/// Projection lifecycle state.
///
/// Transitions:
/// - RUNNING -> REPLAYING: ReadSideRunner::replay() call
/// - RUNNING -> REBUILDING: ReadSideRunner::rebuild() call
/// - RUNNING -> PAUSED: manual pause or transient threshold exceeded
/// - PAUSED -> RUNNING: manual resume
/// - RUNNING -> FAILED: ProjectionError::Fatal or unrecoverable runtime error
/// - REPLAYING -> RUNNING: automatic on completion
/// - REBUILDING -> RUNNING: automatic on completion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionState {
    /// Processing events normally.
    Running,
    /// Replaying from the beginning.
    Replaying,
    /// Rebuilding from scratch.
    Rebuilding,
    /// Paused (manually or due to threshold).
    Paused,
    /// Failed (fatal error or unrecoverable runtime error).
    Failed,
}

impl ProjectionState {
    /// Returns true if the projection is in the Running state.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    /// Returns true if the projection is in the Replaying state.
    pub fn is_replaying(&self) -> bool {
        matches!(self, Self::Replaying)
    }

    /// Returns true if the projection is in the Rebuilding state.
    pub fn is_rebuilding(&self) -> bool {
        matches!(self, Self::Rebuilding)
    }

    /// Returns true if the projection is in the Paused state.
    pub fn is_paused(&self) -> bool {
        matches!(self, Self::Paused)
    }

    /// Returns true if the projection is in the Failed state.
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed)
    }
}

impl std::fmt::Display for ProjectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "RUNNING"),
            Self::Replaying => write!(f, "REPLAYING"),
            Self::Rebuilding => write!(f, "REBUILDING"),
            Self::Paused => write!(f, "PAUSED"),
            Self::Failed => write!(f, "FAILED"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_running() {
        assert!(ProjectionState::Running.is_running());
        assert!(!ProjectionState::Paused.is_running());
    }

    #[test]
    fn test_is_replaying() {
        assert!(ProjectionState::Replaying.is_replaying());
        assert!(!ProjectionState::Running.is_replaying());
    }

    #[test]
    fn test_is_rebuilding() {
        assert!(ProjectionState::Rebuilding.is_rebuilding());
        assert!(!ProjectionState::Running.is_rebuilding());
    }

    #[test]
    fn test_is_paused() {
        assert!(ProjectionState::Paused.is_paused());
        assert!(!ProjectionState::Running.is_paused());
    }

    #[test]
    fn test_is_failed() {
        assert!(ProjectionState::Failed.is_failed());
        assert!(!ProjectionState::Running.is_failed());
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", ProjectionState::Running), "RUNNING");
        assert_eq!(format!("{}", ProjectionState::Replaying), "REPLAYING");
        assert_eq!(format!("{}", ProjectionState::Rebuilding), "REBUILDING");
        assert_eq!(format!("{}", ProjectionState::Paused), "PAUSED");
        assert_eq!(format!("{}", ProjectionState::Failed), "FAILED");
    }

    #[test]
    fn test_equality() {
        assert_eq!(ProjectionState::Running, ProjectionState::Running);
        assert_ne!(ProjectionState::Running, ProjectionState::Paused);
    }

    #[test]
    fn test_clone_copy() {
        let state = ProjectionState::Running;
        let cloned = state;
        assert_eq!(state, cloned);
    }
}
