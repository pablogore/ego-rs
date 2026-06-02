use std::fmt;

use crate::runtime::execution::ExecutionId;

/// Reasons why a message send can fail.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SendErrorKind {
    /// No execution with the given `ExecutionId` exists.
    NotFound,
    /// The execution is no longer accepting messages.
    Closed,
}

impl fmt::Display for SendErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "execution not found"),
            Self::Closed => write!(f, "execution is closed"),
        }
    }
}

impl std::error::Error for SendErrorKind {}

/// Error returned when a message send fails.
#[derive(Debug)]
pub struct SendError {
    /// The execution id that was targeted.
    pub id: ExecutionId,
    /// The reason the send failed.
    pub cause: SendErrorKind,
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "send failed for execution {}: {}", self.id, self.cause)
    }
}

impl std::error::Error for SendError {}

/// Reasons why a spawn can fail.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum SpawnErrorKind {
    /// The runtime is no longer accepting new executions.
    Closed,
    /// An internal runtime error prevented spawning.
    Internal(String),
}

impl fmt::Display for SpawnErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "runtime is closed"),
            Self::Internal(msg) => write!(f, "internal runtime error: {}", msg),
        }
    }
}

impl std::error::Error for SpawnErrorKind {}

/// Error returned when spawning a new execution fails.
#[derive(Debug)]
pub struct SpawnError {
    /// The reason the spawn failed.
    pub cause: SpawnErrorKind,
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "spawn failed: {}", self.cause)
    }
}

impl std::error::Error for SpawnError {}
