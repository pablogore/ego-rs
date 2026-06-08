//! Error types for the scheduler crate.

use thiserror::Error;

/// Errors that can occur within the scheduler.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum SchedulerError {
    /// The event bus is full and cannot accept new events.
    #[error("Event bus is full")]
    EventBusFull,

    /// A gap was detected in the sequence of events.
    #[error("Gap detected in sequence: {start}..{end}")]
    GapDetected { start: u64, end: u64 },

    /// An invalid sequence ID was encountered.
    #[error("Invalid sequence ID: expected {expected}, got {actual}")]
    InvalidSequence { expected: u64, actual: u64 },

    /// A state hash mismatch occurred.
    #[error("State hash mismatch")]
    StateHashMismatch,
}