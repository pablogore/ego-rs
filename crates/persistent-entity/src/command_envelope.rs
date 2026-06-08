//! Command envelope for wrapping commands with context.
//!
//! This module provides a command envelope that wraps commands with context information.

use serde::{Deserialize, Serialize};

use crate::command_context::CommandContext;

/// A command envelope that wraps a command with context information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEnvelope<C> {
    /// The command to be executed.
    pub command: C,
    /// The command context.
    pub context: CommandContext,
}