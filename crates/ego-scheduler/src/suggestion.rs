//! Suggestion types and related functionality.

use crate::types::EntityTriple;

/// Represents a suggestion for an entity to activate.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivationSuggestion {
    /// The entity to activate.
    pub entity: EntityTriple,
}