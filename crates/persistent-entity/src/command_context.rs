//! Command context information available during command processing.
//!
//! This module provides context information that is available during command processing,
//! including metadata about the command execution environment.

use ego_domain::operation::OperationKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Context information available during command processing.
///
/// This struct contains metadata about the command execution environment,
/// including tenant information, version information, and other relevant data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandContext {
    /// The tenant identifier for the command.
    pub tenant_id: Option<String>,

    /// The entity type identifier.
    pub entity_type: String,

    /// The entity identifier.
    pub entity_id: String,

    /// The expected version for optimistic concurrency control.
    pub expected_version: Option<u64>,

    /// The causation identifier for the command.
    pub causation_id: Option<String>,

    /// Additional metadata for the command.
    pub metadata: HashMap<String, String>,

    /// The caller-supplied operation key carried from the service boundary,
    /// set once at ingress and passed through unchanged to
    /// `EntityActor::execute_command` and the `handle_command` call it
    /// makes — never regenerated, normalised, or reconstructed along the
    /// way.
    pub operation_key: Option<OperationKey>,
}

impl CommandContext {
    /// Create a new command context.
    ///
    /// # Arguments
    /// * `entity_type` - The type of the entity
    ///
    /// # Returns
    /// * `CommandContext` - A new command context with default values
    pub fn new(entity_type: String) -> Self {
        Self {
            tenant_id: None,
            entity_type,
            entity_id: String::new(),
            expected_version: None,
            causation_id: None,
            metadata: HashMap::new(),
            operation_key: None,
        }
    }
}
