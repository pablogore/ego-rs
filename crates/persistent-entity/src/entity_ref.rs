//! Entity reference API for interacting with persistent entities.
//!
//! This module provides the main interface for sending commands to persistent entities.
//! The EntityRef is the primary way to interact with entities in the system.

use crate::command_context::CommandContext;
use crate::error::EntityError;
use async_trait::async_trait;
use serde::Serialize;
use std::fmt::Debug;

/// A reference to a persistent entity that can be used to send commands.
///
/// This is the primary API for interacting with persistent entities.
#[async_trait]
pub trait EntityRef: Clone + Send + Sync + Debug {
    /// Send a command to the entity and await the result.
    ///
    /// # Arguments
    /// * `command` - The command to send to the entity
    /// * `context` - The command context containing metadata
    ///
    /// # Returns
    /// * `Result<T, EntityError>` - The result of the command or an error
    async fn send_command<T, C>(
        &self,
        command: C,
        context: CommandContext,
    ) -> Result<T, EntityError>
    where
        T: Send + 'static,
        C: Serialize + Send + 'static;
}
