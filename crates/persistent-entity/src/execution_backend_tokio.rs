//! Tokio-based execution backend (deprecated — retained for compatibility only).
//!
//! The `block_on` path has been removed. Command execution now happens
//! directly inside the spawned actor task via `.await`. This module is kept
//! only in case external consumers still reference [`TokioExecutionBackend`].
//!
//! # Deprecation notice
//!
//! [`TokioExecutionBackend`] and [`SyncTestBackend`] are deprecated. The actor
//! loop in [`crate::actor`] awaits handler methods directly; no
//! [`ExecutionBackend`] implementation is used on the hot path. These types
//! will be removed in a future change.

use crate::command_context::CommandContext;
use crate::error::EntityError;
#[allow(deprecated)]
use crate::execution_backend::ExecutionBackend;
use crate::persistent_entity::PersistentEntity;
use serde::de::DeserializeOwned;

/// Deprecated Tokio-based [`ExecutionBackend`].
///
/// Previously used `futures::executor::block_on` to drive async handlers
/// synchronously. That pattern panics inside a `current_thread` Tokio
/// runtime. The actor now awaits handlers directly; this type is a stub
/// retained for API compatibility.
#[deprecated(
    since = "0.2.0",
    note = "Use EntityActor directly; block_on has been removed"
)]
#[derive(Debug, Default, Clone)]
pub struct TokioExecutionBackend;

#[allow(deprecated)]
impl TokioExecutionBackend {
    /// Creates a new [`TokioExecutionBackend`].
    pub fn new() -> Self {
        Self
    }
}

#[allow(deprecated)]
impl ExecutionBackend for TokioExecutionBackend {
    fn execute<C, E, S>(
        &self,
        _entity: &dyn PersistentEntity<Command = C, Event = E, State = S>,
        _state: &S,
        _command: &C,
        _context: &CommandContext,
    ) -> Result<(Vec<E>, S), EntityError>
    where
        C: Send + Sync + serde::Serialize + 'static,
        E: Send + Sync + Clone + serde::Serialize + 'static,
        S: Clone + Send + Sync + serde::Serialize + DeserializeOwned + 'static,
    {
        Err(EntityError::Internal(
            "TokioExecutionBackend is deprecated; use EntityActor directly".to_string(),
        ))
    }
}

/// Deprecated test [`ExecutionBackend`].
///
/// Delegates to [`TokioExecutionBackend`], which is itself deprecated.
/// Kept for API compatibility only.
#[deprecated(
    since = "0.2.0",
    note = "Use EntityActor with InMemory stores directly; block_on has been removed"
)]
#[derive(Debug, Default, Clone)]
pub struct SyncTestBackend;

#[allow(deprecated)]
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
        #[allow(deprecated)]
        TokioExecutionBackend.execute(entity, state, command, context)
    }
}
