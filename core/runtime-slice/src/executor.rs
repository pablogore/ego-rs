//! Deterministic execution engine.
//!
//! Provides the `Executor`, `UnitOfWork`, and `LifecycleState` types
//! that implement the governed execution state machine for runtime slices.
//!
//! ## State Machine
//!
//! ```text
//! Pending → Running → Completed(Outcome)
//!    |                    |
//!    └──→ Failed(Error) ←─┘
//! ```
//!
//! Transitions from terminal states (Completed, Failed) are rejected.

use crate::types::{ExecutionContext, ExecutionOutcome, RuntimeSliceError};
use std::collections::HashMap;

/// Lifecycle state of a unit of work.
///
/// Governed transitions:
/// - `Pending → Running` on start
/// - `Running → Completed(outcome)` on success
/// - `Running → Failed(error)` on error
/// - Terminal states (`Completed`, `Failed`) are immutable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleState {
    /// Initial state — execution not yet started.
    Pending,
    /// Execution is in progress.
    Running,
    /// Execution completed successfully with an observable outcome.
    Completed(ExecutionOutcome),
    /// Execution failed with an error.
    Failed(RuntimeSliceError),
}

impl LifecycleState {
    /// Returns `true` if this is a terminal state (Completed or Failed).
    ///
    /// Terminal states reject further execution attempts.
    pub fn is_terminal(&self) -> bool {
        matches!(self, LifecycleState::Completed(_) | LifecycleState::Failed(_))
    }
}

/// A single unit of governed deterministic work.
///
/// Binds an `ExecutionContext` to a `LifecycleState`. Each `UnitOfWork`
/// is executed exactly once by the `Executor`. Repeated execution is
/// rejected.
#[derive(Debug, Clone)]
pub struct UnitOfWork {
    /// The execution context — slice identity and deterministic inputs.
    pub context: ExecutionContext,
    /// Current lifecycle state of this unit.
    pub state: LifecycleState,
}

impl UnitOfWork {
    /// Creates a new `UnitOfWork` in `Pending` state.
    ///
    /// # Arguments
    /// * `context` — the execution context with slice identity and inputs.
    pub fn new(context: ExecutionContext) -> Self {
        Self {
            context,
            state: LifecycleState::Pending,
        }
    }

    /// Executes the unit of work, transitioning through the state machine.
    ///
    /// # Purpose
    /// Processes deterministic inputs to produce an `ExecutionOutcome`.
    /// Duplicate input keys are counted and reported as dedup entries.
    ///
    /// # Errors
    /// Returns `RuntimeSliceError::AmbiguousInput` if the unit is not
    /// in `Pending` state (already executed or in a terminal state).
    pub fn execute(&mut self) -> Result<(), RuntimeSliceError> {
        if self.state != LifecycleState::Pending {
            return Err(RuntimeSliceError::AmbiguousInput(
                "unit of work is not pending",
            ));
        }
        self.state = LifecycleState::Running;

        let mut semantics = Vec::new();
        let mut seen = HashMap::new();
        for input in &self.context.inputs {
            let key = input.key.clone();
            let value = input.value.clone();
            let entry = seen.entry(key.clone()).or_insert(0);
            *entry += 1;
            semantics.push(format!("processed:{}={}", key, value));
        }
        for (key, count) in &seen {
            if *count > 1 {
                semantics.push(format!("dedup:{}={}", key, count));
            }
        }
        semantics.sort();

        let outcome = ExecutionOutcome::new(self.context.slice_id.clone(), semantics)?;
        self.state = LifecycleState::Completed(outcome);
        Ok(())
    }
}

/// Deterministic executor for governed runtime slices.
///
/// Stateless — all state lives in `UnitOfWork`. The executor validates
/// state transitions and produces deterministic outcomes.
#[derive(Debug, Clone, Default)]
pub struct Executor;

impl Executor {
    /// Creates a new `Executor`.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self
    }

    /// Executes a context and returns the resulting outcome.
    ///
    /// # Arguments
    /// * `context` — the execution context to process.
    ///
    /// # Returns
    /// The `ExecutionOutcome` containing observable semantics.
    ///
    /// # Errors
    /// Returns `RuntimeSliceError` if execution fails or produces
    /// no outcome.
    pub fn execute(
        &self,
        context: ExecutionContext,
    ) -> Result<ExecutionOutcome, RuntimeSliceError> {
        let mut unit = UnitOfWork::new(context);
        unit.execute()?;
        match &unit.state {
            LifecycleState::Completed(outcome) => Ok(outcome.clone()),
            _ => Err(RuntimeSliceError::AmbiguousOutcome(
                "execution produced no outcome",
            )),
        }
    }

    /// Accepts a pre-created `UnitOfWork` and executes it in-place.
    ///
    /// # Arguments
    /// * `unit` — the unit of work to execute (must be in `Pending` state).
    ///
    /// # Returns
    /// The same `UnitOfWork` advanced to `Completed` or `Failed` state.
    ///
    /// # Errors
    /// Returns `RuntimeSliceError` if execution fails.
    pub fn accept(&self, mut unit: UnitOfWork) -> Result<UnitOfWork, RuntimeSliceError> {
        unit.execute()?;
        Ok(unit)
    }

    /// Forces a unit into `Running` state and executes it.
    ///
    /// Test-only — used to verify behavior with non-Pending initial states.
    #[cfg(test)]
    pub fn test_transition(&self, mut unit: UnitOfWork) -> UnitOfWork {
        unit.state = LifecycleState::Running;
        unit.execute().unwrap();
        unit
    }
}
