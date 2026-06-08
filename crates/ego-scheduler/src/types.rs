//! Core data types for the scheduler.

use std::hash::Hash;

/// A triple identifier for an actor/entity.
///
/// This is used to uniquely identify entities within the scheduler system.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct EntityTriple {
    /// The tenant identifier.
    pub tenant: String,
    /// The entity type.
    pub entity_type: String,
    /// The entity identifier.
    pub entity_id: String,
}

impl EntityTriple {
    /// Creates a new EntityTriple.
    pub fn new(tenant: String, entity_type: String, entity_id: String) -> Self {
        Self {
            tenant,
            entity_type,
            entity_id,
        }
    }
}