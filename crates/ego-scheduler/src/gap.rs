//! Gap detection and handling.

use crate::event::SchedulerEventEnvelope;

/// Represents a detected gap in sequence IDs.
#[derive(Debug, Clone, PartialEq)]
pub struct Gap {
    /// The start of the gap.
    pub start: u64,
    /// The end of the gap.
    pub end: u64,
}

impl Gap {
    /// Creates a new gap.
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }
}