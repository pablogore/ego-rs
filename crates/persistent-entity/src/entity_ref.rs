//! Entity reference API for interacting with persistent entities.

use crate::command_context::CommandContext;
use crate::error::EntityError;
use async_trait::async_trait;
use serde::Serialize;
use std::fmt::Debug;

/// A reference to a persistent entity; the primary surface for dispatching commands.
#[async_trait]
pub trait EntityRef: Clone + Send + Sync + Debug {
    /// The command type this ref accepts. Fixed per impl — no runtime downcast needed.
    type Command: Serialize + Send + 'static;

    /// Sends a command to the entity and returns its result.
    async fn send_command<T>(
        &self,
        command: Self::Command,
        context: CommandContext,
    ) -> Result<T, EntityError>
    where
        T: Send + 'static;
}
