use crate::types::{ExecutionContext, ExecutionOutcome, RuntimeSliceError};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleState {
    Pending,
    Running,
    Completed(ExecutionOutcome),
    Failed(RuntimeSliceError),
}

impl LifecycleState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, LifecycleState::Completed(_) | LifecycleState::Failed(_))
    }
}

#[derive(Debug, Clone)]
pub struct UnitOfWork {
    pub context: ExecutionContext,
    pub state: LifecycleState,
}

impl UnitOfWork {
    pub fn new(context: ExecutionContext) -> Self {
        Self {
            context,
            state: LifecycleState::Pending,
        }
    }

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

#[derive(Debug, Clone, Default)]
pub struct Executor;

impl Executor {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self
    }

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

    pub fn accept(&self, mut unit: UnitOfWork) -> Result<UnitOfWork, RuntimeSliceError> {
        unit.execute()?;
        Ok(unit)
    }

    #[cfg(test)]
    pub fn test_transition(&self, mut unit: UnitOfWork) -> UnitOfWork {
        unit.state = LifecycleState::Running;
        unit.execute().unwrap();
        unit
    }
}
