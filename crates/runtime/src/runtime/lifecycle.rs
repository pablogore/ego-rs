/// The state of an execution.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ExecutionState {
    /// Actor is active and processing messages.
    Active,
    /// Actor is shutting down gracefully (no new messages accepted).
    Draining,
    /// Actor has stopped. Terminal state — no further transitions.
    Terminated,
    /// Actor has failed. Terminal state — no further transitions.
    Failed,
}

// Safety: ExecutionState contains only unit variants with no data,
// so it is trivially Send + Sync.
unsafe impl Send for ExecutionState {}
unsafe impl Sync for ExecutionState {}
