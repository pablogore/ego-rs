//! Passivation types for entity lifecycle management.
//!
//! This module provides types for managing entity passivation.

/// A handle to an active entity.
#[derive(Clone)]
pub struct EntityHandle {
    /// The entity's state.
    pub state: Option<String>,
}

/// A message that can be sent to an entity.
pub enum EntityMessage {
    /// A command to process.
    ProcessCommand(Box<dyn Send + Sync + 'static>),
    /// A message to handle.
    HandleMessage(Box<dyn Send + Sync + 'static>),
}
