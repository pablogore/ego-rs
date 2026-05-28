use crate::types::ExecutionOutcome;

#[derive(Debug, Clone)]
pub struct Projection {
    pub slice_id: String,
    pub observable_semantics: Vec<String>,
}

pub fn materialize(outcome: &ExecutionOutcome) -> Projection {
    let mut semantics = outcome.observable_semantics.clone();
    semantics.sort();
    Projection {
        slice_id: outcome.slice_id.as_str().to_string(),
        observable_semantics: semantics,
    }
}

pub fn is_non_mutating(original: &ExecutionOutcome, projected: &Projection) -> bool {
    let mut original_semantics = original.observable_semantics.clone();
    original_semantics.sort();
    original_semantics == projected.observable_semantics
}
