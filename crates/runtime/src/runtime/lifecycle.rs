//! Execution lifecycle states.
//!
//! Defines the state machine that governs each execution unit's lifecycle.
//!
//! ## State Machine
//!
//! ```text
//! Active → Draining → Terminated
//!    └──→ Failed
//! ```
//!
//! Terminal states (`Terminated`, `Failed`) are immutable — no further
//! transitions are permitted.

/// The lifecycle state of an execution.
///
/// # Contract
/// - `Active`: execution is running and accepting messages.
/// - `Draining`: shutdown requested, no new messages accepted but in-flight
///   messages may complete.
/// - `Terminated`: execution has stopped cleanly. Terminal.
/// - `Failed`: execution panicked or encountered an unrecoverable error. Terminal.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum ExecutionState {
    /// Execution is active and processing messages.
    Active,
    /// Execution is shutting down gracefully (no new messages accepted).
    Draining,
    /// Execution has stopped. Terminal state — no further transitions.
    Terminated,
    /// Execution has failed. Terminal state — no further transitions.
    Failed,
}

// Safety: ExecutionState contains only unit variants with no data,
// so it is trivially Send + Sync.
unsafe impl Send for ExecutionState {}
unsafe impl Sync for ExecutionState {}
