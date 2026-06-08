//! A simple scheduler for entity management.
//!
//! This module provides a basic scheduler for entity management.

use std::sync::Arc;

/// A simple entity triple identifier.
#[derive(Debug, Clone)]
pub struct EntityTriple {
    /// The tenant identifier.
    pub tenant_id: String,
    /// The entity type.
    pub entity_type: String,
    /// The entity identifier.
    pub entity_id: String,
}

impl EntityTriple {
    /// Create a new entity triple.
    pub fn new(tenant_id: String, entity_type: &str, entity_id: impl Into<String>) -> Self {
        Self {
            tenant_id,
            entity_type: entity_type.to_string(),
            entity_id: entity_id.into(),
        }
    }

    /// Get the aggregate identifier.
    pub fn aggregate_id(&self) -> String {
        format!("{}-{}", self.entity_type, self.entity_id)
    }
}

/// A simple scheduler for entity management.
#[derive(Debug)]
pub struct Scheduler {
    /// The entity registry.
    pub registry: Arc<crate::registry::EntityRegistry>,
}

impl Scheduler {
    /// Create a new scheduler.
    pub fn new(registry: Arc<crate::registry::EntityRegistry>) -> Self {
        Self { registry }
    }
}