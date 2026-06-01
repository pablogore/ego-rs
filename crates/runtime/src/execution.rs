use ego_domain::{ActorId, ActorLifecycleState};
use std::fmt::Debug;

/// The unique identifier for an execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionId(String);

impl ExecutionId {
    /// Create a new `ExecutionId` with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Return the execution name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The state of an execution.
#[derive(Debug, Clone)]
pub enum ExecutionState {
    /// Actor identity registered but not yet starting.
    Created,
    /// Actor initializing (transition to Running expected).
    Starting,
    /// Actor is active and processing messages.
    Running,
    /// Actor is shutting down gracefully.
    Stopping,
    /// Actor has stopped. Terminal state — no further transitions.
    Stopped,
    /// Actor has failed. Terminal state — no further transitions.
    Failed,
}

impl ExecutionState {
    /// Returns `true` if this is a terminal state (Stopped or Failed).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }
}
