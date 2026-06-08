//! Execution backend trait for command processing.
//!
//! Defines the [`ExecutionBackend`] trait that abstracts command execution
//! for persistent entities. Implementations wrap async handler invocation
//! and event application in a synchronous interface.

use std::fmt::Debug;

use serde::de::DeserializeOwned;

use crate::command_context::CommandContext;
use crate::error::EntityError;
use crate::persistent_entity::PersistentEntity;

/// Abstracts command execution for persistent entities.
///
/// Implementations handle the bridge between async handler logic and
/// synchronous callers, either through Tokio's `block_on` or other
/// executor mechanisms.
pub trait ExecutionBackend: Debug + Send + Sync {
    /// Executes a command against an entity's current state.
    ///
    /// Returns the produced events and the new state, or an error.
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
        S: Clone + Send + Sync + serde::Serialize + DeserializeOwned + 'static;
}
