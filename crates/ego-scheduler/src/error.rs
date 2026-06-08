//! Scheduler error types.
//!
//! # Ownership
//! Errors are produced by the event bus and consumed by callers.
//!
//! # Invariants
//! - `BusFull`: returned when bounded channel is full under Block policy
//! - `ChannelClosed`: returned when receiver is dropped

use thiserror::Error;

/// Errors that can occur in scheduler operations.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum SchedulerError {
    /// Event bus is at capacity under Block policy.
    #[error("Event bus is full")]
    BusFull,
    /// Channel has been closed (receiver dropped).
    #[error("Channel closed")]
    ChannelClosed,
}
