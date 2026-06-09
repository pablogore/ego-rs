use crate::executor::{Executor, LifecycleState, UnitOfWork};
use crate::projection::{is_non_mutating, materialize};
use crate::types::{DeterministicInput, ExecutionContext, RuntimeSliceId};
use crate::validation::{validate_deterministic_equivalence, validate_replay_equivalence};

fn test_slice_id() -> RuntimeSliceId {
    RuntimeSliceId::new("test-slice").unwrap()
}

fn test_context(inputs: Vec<(&str, &str)>) -> ExecutionContext {
    let det_inputs: Vec<DeterministicInput> = inputs
        .into_iter()
        .map(|(k, v)| DeterministicInput::new(k, v).unwrap())
        .collect();
    ExecutionContext::new(test_slice_id(), det_inputs).unwrap()
}

#[test]
fn test_executor_deterministic_execution() {
    let ctx = test_context(vec![("a", "1"), ("b", "2")]);
    let executor = Executor::new();
    let outcome1 = executor.execute(ctx.clone()).unwrap();
    let outcome2 = executor.execute(ctx).unwrap();

    // Verify deterministic execution by checking that observable semantics are identical
    assert_eq!(outcome1.observable_semantics, outcome2.observable_semantics);
    assert_eq!(outcome1.slice_id, outcome2.slice_id);
}

#[test]
fn test_executor_transitions() {
    let ctx = test_context(vec![("x", "1")]);

    let mut unit = UnitOfWork::new(ctx);
    assert_eq!(unit.state, LifecycleState::Pending);

    unit.execute().unwrap();
    assert!(matches!(unit.state, LifecycleState::Completed(_)));

    let result = unit.execute();
    assert!(result.is_err());
}

#[test]
fn test_executor_fail_closed_on_ambiguous_state() {
    let ctx = test_context(vec![("x", "1")]);
    let mut unit = UnitOfWork::new(ctx);
    unit.state = LifecycleState::Running;

    let executor = Executor::new();
    let result = executor.accept(unit);
    assert!(result.is_err());
}

#[test]
fn test_projection_non_mutating() {
    let ctx = test_context(vec![("a", "1")]);
    let executor = Executor::new();
    let outcome = executor.execute(ctx).unwrap();

    let projection = materialize(&outcome);

    // Verify that the projection is non-mutating by checking the result
    assert!(is_non_mutating(&outcome, &projection));
    assert_eq!(outcome.slice_id.as_str(), projection.slice_id);

    // Verify that the projection contains expected data
    assert!(!projection.observable_semantics.is_empty());
    assert_eq!(projection.observable_semantics.len(), 1);
}

#[test]
fn test_replay_equivalence() {
    let ctx = test_context(vec![("key", "value")]);
    let executor = Executor::new();

    let original = executor.execute(ctx.clone()).unwrap();
    let replay = executor.execute(ctx).unwrap();

    // Verify replay equivalence with multiple assertions
    assert_eq!(original.observable_semantics, replay.observable_semantics);
    assert_eq!(original.slice_id, replay.slice_id);
    assert!(validate_replay_equivalence(&original, &replay));
    assert!(validate_deterministic_equivalence(&original, &replay));
}

#[test]
fn test_validation_deterministic_equivalence() {
    let ctx = test_context(vec![("a", "1"), ("b", "2")]);
    let executor = Executor::new();

    let outcome_a = executor.execute(ctx.clone()).unwrap();
    let outcome_b = executor.execute(ctx).unwrap();

    // Verify deterministic equivalence with additional checks
    assert!(validate_deterministic_equivalence(&outcome_a, &outcome_b));

    // Verify that both outcomes have the same observable semantics
    assert_eq!(
        outcome_a.observable_semantics,
        outcome_b.observable_semantics
    );
    assert_eq!(outcome_a.slice_id, outcome_b.slice_id);
}

#[test]
fn test_infrastructure_free() {
    let ctx = test_context(vec![("x", "1")]);
    let executor = Executor::new();
    let outcome = executor.execute(ctx).unwrap();

    let projection = materialize(&outcome);

    // Verify that the projection contains observable semantics
    assert!(!projection.observable_semantics.is_empty());
    assert_eq!(projection.observable_semantics.len(), 1);

    // Verify that the projection has the expected structure
    assert_eq!(projection.slice_id, outcome.slice_id.as_str());
}
