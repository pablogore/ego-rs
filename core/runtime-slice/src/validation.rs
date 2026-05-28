use crate::types::ExecutionOutcome;

pub fn validate_deterministic_equivalence(
    a: &ExecutionOutcome,
    b: &ExecutionOutcome,
) -> bool {
    a.slice_id == b.slice_id && a.observable_semantics == b.observable_semantics
}

pub fn validate_replay_equivalence(
    original: &ExecutionOutcome,
    replay: &ExecutionOutcome,
) -> bool {
    original == replay
}
