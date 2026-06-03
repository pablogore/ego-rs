//! Immutable projections of execution outcomes.
//!
//! Projections are read-only views of `ExecutionOutcome` that can be
//! compared, serialized, or stored without mutating the original outcome.
//! They preserve the determinism invariant by construction.

use crate::types::ExecutionOutcome;

/// An immutable, sorted projection of an execution outcome.
///
/// # Purpose
/// Projections lock in the observable semantics at a point in time,
/// enabling deterministic comparison and replay validation.
#[derive(Debug, Clone)]
pub struct Projection {
    /// The runtime slice identifier this projection belongs to.
    pub slice_id: String,
    /// The observable semantics, sorted for deterministic comparison.
    pub observable_semantics: Vec<String>,
}

/// Materializes a `Projection` from an `ExecutionOutcome`.
///
/// The observable semantics are sorted to ensure deterministic
/// comparison across executions.
///
/// # Arguments
/// * `outcome` — the execution outcome to project.
///
/// # Returns
/// A `Projection` with sorted observable semantics.
pub fn materialize(outcome: &ExecutionOutcome) -> Projection {
    let mut semantics = outcome.observable_semantics.clone();
    semantics.sort();
    Projection {
        slice_id: outcome.slice_id.as_str().to_string(),
        observable_semantics: semantics,
    }
}

/// Returns `true` if the projection contains exactly the same observable
/// semantics as the original outcome (after sorting both).
///
/// # Purpose
/// Validates that no mutation occurred during projection — the projected
/// view is a faithful, non-mutating representation of the outcome.
///
/// # Arguments
/// * `original` — the original execution outcome.
/// * `projected` — the projection to compare.
pub fn is_non_mutating(original: &ExecutionOutcome, projected: &Projection) -> bool {
    let mut original_semantics = original.observable_semantics.clone();
    original_semantics.sort();
    original_semantics == projected.observable_semantics
}
