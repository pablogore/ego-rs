//! Tokio-based implementations of the [`ExecutionBackend`] trait.
//!
//! Provides [`TokioExecutionBackend`] for real execution and [`SyncTestBackend`]
//! for test environments.

use futures::executor::block_on;
use serde::de::DeserializeOwned;

use crate::command_context::CommandContext;
use crate::error::EntityError;
use crate::execution_backend::ExecutionBackend;
use crate::persistent_entity::PersistentEntity;

/// An [`ExecutionBackend`] that runs handler logic via `tokio::task::block_on`.
///
/// Suitable for use within Tokio runtimes where blocking is acceptable.
#[derive(Debug, Default, Clone)]
pub struct TokioExecutionBackend;

impl TokioExecutionBackend {
    /// Creates a new [`TokioExecutionBackend`].
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

/// A test-only [`ExecutionBackend`] that delegates to [`TokioExecutionBackend`].
///
/// Provides the same behavior for synchronous test contexts.
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
        S: Clone + Send + Sync + serde::Serialize + DeserializeOwned + 'static,
    {
        TokioExecutionBackend.execute(entity, state, command, context)
    }
}