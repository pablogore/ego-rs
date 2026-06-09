//! Trait defining the interface for persistent entities.
//!
//! This trait must be implemented by all persistent entities in the system.

use crate::command_context::CommandContext;
use crate::error::EntityError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// Result type for command processing.
#[derive(Debug, Clone)]
pub enum CommandResult<E, S> {
    /// Command produced events and a new state.
    Events {
        /// The new state after applying events.
        new_state: S,
        /// The events produced by the command.
        events: Vec<E>,
    },
    /// Command produced no events but returned a state.
    NoEvents {
        /// The state returned by the command.
        state: S,
    },
}

/// Trait that all persistent entities must implement.
///
/// This trait defines the core behavior of persistent entities, including
/// command handling and event application.
#[async_trait]
pub trait PersistentEntity: Send + Sync + Debug {
    /// The type of commands this entity can handle.
    type Command: Serialize + Send + Sync + 'static;

    /// The type of events this entity produces.
    type Event: Serialize + Send + Sync + 'static;

    /// The type of state this entity maintains.
    type State: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static;

    /// Get the initial state for this entity.
    ///
    /// # Returns
    /// * `Self::State` - The initial state
    fn initial_state(&self) -> Self::State;

    /// Handle a command and produce events.
    ///
    /// # Arguments
    /// * `command` - The command to process
    /// * `state` - The current state of the entity
    /// * `context` - The command context
    ///
    /// # Returns
    /// * `Result<Vec<Self::Event>, EntityError>` - The events produced by the command
    async fn handle_command(
        &self,
        command: &Self::Command,
        state: &Self::State,
        context: &CommandContext,
    ) -> Result<Vec<Self::Event>, EntityError>;

    /// Apply a single event to update the entity state.
    ///
    /// # Arguments
    /// * `state` - The current state of the entity
    /// * `event` - The event to apply
    ///
    /// # Returns
    /// * `Result<Self::State, EntityError>` - The updated state
    async fn apply_event(
        &self,
        state: &Self::State,
        event: &Self::Event,
    ) -> Result<Self::State, EntityError>;

    /// Apply events to update the entity state.
    ///
    /// # Arguments
    /// * `state` - The current state of the entity
    /// * `events` - The events to apply
    ///
    /// # Returns
    /// * `Result<Self::State, EntityError>` - The updated state
    async fn apply_events(
        &self,
        state: &Self::State,
        events: &[Self::Event],
    ) -> Result<Self::State, EntityError>;
}
