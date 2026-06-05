//! Equivalence and replay-safety validators for deterministic execution.
//!
//! # Purpose
//! These validators enforce the determinism axiom: given identical inputs,
//! two executions MUST produce identical observable outcomes.

use crate::types::ExecutionOutcome;

/// Returns `true` if two outcomes have the same slice identity and
/// observable semantics — confirming deterministic equivalence.
///
/// # Purpose
/// Two executions with identical inputs must produce identical outcomes.
/// This function validates that invariant.
///
/// # Arguments
/// * `a` — first execution outcome.
/// * `b` — second execution outcome (produced from identical inputs).
pub fn validate_deterministic_equivalence(a: &ExecutionOutcome, b: &ExecutionOutcome) -> bool {
    a.slice_id == b.slice_id && a.observable_semantics == b.observable_semantics
}

/// Returns `true` if the replay outcome is structurally identical to
/// the original — confirming replay equivalence.
///
/// # Purpose
/// Replay must produce bitwise-identical outcomes. This is stricter
/// than deterministic equivalence (it uses full `PartialEq`).
///
/// # Arguments
/// * `original` — the original execution outcome.
/// * `replay` — the replayed execution outcome.
pub fn validate_replay_equivalence(original: &ExecutionOutcome, replay: &ExecutionOutcome) -> bool {
    original == replay
}
