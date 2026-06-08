//! Gap detection types.
//!
//! # Ownership
//! Gap detection logic lives in scheduler/detect.rs.
//! This module provides only the GapInfo type for observability.
//!
//! # Invariants
//! - Per-actor scoped only — no cross-entity gap inference
//! - Uniform treatment — no gap-type classification, no per-cause attribution

use crate::event_bus::EntityTriple;

/// Structured gap record for observability.
/// Per-actor scoped — gap detection operates independently per entity stream.
/// No cross-entity ordering semantics.
#[derive(Debug, Clone)]
pub struct GapInfo {
    /// Start of the gap range (exclusive of last consumed).
    pub start_seq: u64,
    /// End of the gap range (inclusive of the gap boundary).
    pub end_seq: u64,
    /// Actor stream where gap was detected.
    pub source_actor: EntityTriple,
}
