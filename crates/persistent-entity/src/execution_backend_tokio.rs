use futures::executor::block_on;
use serde::de::DeserializeOwned;

use crate::command_context::CommandContext;
use crate::error::EntityError;
use crate::execution_backend::ExecutionBackend;
use crate::persistent_entity::PersistentEntity;

#[derive(Debug, Default, Clone)]
pub struct TokioExecutionBackend;

impl TokioExecutionBackend {
    pub fn new() -> Self {
        Self
    }
}

impl ExecutionBackend for TokioExecutionBackend {
    fn execute<C, E, S>(
        &self,
        entity: &dyn PersistentEntity<Command = C, Event = E, State = S>,
        state: &S,
        command: &C,
        context: &CommandContext,
    ) -> Result<(Vec<E>, S), EntityError>
    where
        C: Send + Sync + serde::Serialize + 'static,
        E: Send + Sync + Clone + serde::Serialize + 'static,
        S: Clone + Send + Sync + serde::Serialize + DeserializeOwned + 'static,
    {
        let events = block_on(entity.handle_command(command, state, context))
            .map_err(|e| EntityError::Internal(e.to_string()))?;

        let new_state = if events.is_empty() {
            state.clone()
        } else {
            block_on(entity.apply_events(state, &events))
                .map_err(|e| EntityError::Internal(e.to_string()))?
        };

        Ok((events, new_state))
    }
}

#[derive(Debug, Default, Clone)]
pub struct SyncTestBackend;

impl ExecutionBackend for SyncTestBackend {
    fn execute<C, E, S>(
        &self,
        entity: &dyn PersistentEntity<Command = C, Event = E, State = S>,
        state: &S,
        command: &C,
        context: &CommandContext,
    ) -> Result<(Vec<E>, S), EntityError>
    where
        C: Send + Sync + serde::Serialize + 'static,
        E: Send + Sync + Clone + serde::Serialize + 'static,
        S: Clone + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static,
    {
        TokioExecutionBackend.execute(entity, state, command, context)
    }
}