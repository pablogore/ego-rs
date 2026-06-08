//! Gap detection and handling.


/// Represents a detected gap in sequence IDs.
#[derive(Debug, Clone, PartialEq)]
pub struct GapInfo {
    /// The start of the gap.
    pub start_seq: u64,
    /// The end of the gap.
    pub end_seq: u64,
    /// The actor that caused the gap.
    pub source_actor: crate::types::EntityTriple,
}

impl GapInfo {
    /// Creates a new gap.
    pub fn new(start_seq: u64, end_seq: u64, source_actor: crate::types::EntityTriple) -> Self {
        Self { start_seq, end_seq, source_actor }
    }
}