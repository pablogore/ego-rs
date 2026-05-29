use std::fmt::Debug;

/// The error type for runtime operations.
#[derive(Debug, Clone, Send, Sync)]
pub enum RuntimeError {
    /// An error occurred while spawning an actor.
    SpawnError(String),
    /// An error occurred while sending a message to an actor.
    SendError(String),
    /// An error occurred while getting the state of an execution.
    GetStateError(String),
    /// An error occurred while stopping an actor.
    StopError(String),
    /// An error occurred while restarting an actor.
    RestartError(String),
    /// An error occurred while escalating a failure.
    EscalateError(String),
}
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnError(msg) => write!(f, "Spawn error: {}", msg),
            Self::SendError(msg) => write!(f, "Send error: {}", msg),
            Self::GetStateError(msg) => write!(f, "Get state error: {}", msg),
            Self::StopError(msg) => write!(f, "Stop error: {}", msg),
            Self::RestartError(msg) => write!(f, "Restart error: {}", msg),
            Self::EscalateError(msg) => write!(f, "Escalate error: {}", msg),
        }
    }
}

impl std::error::Error for RuntimeError {}
