//! A simple registry for tracking entities.
//!
//! This module provides a basic registry for tracking entity states.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A simple registry for tracking entities.
#[derive(Debug)]
pub struct EntityRegistry {
    /// The active entities.
    active_entities: Arc<Mutex<HashMap<String, bool>>>,
}

impl EntityRegistry {
    /// Create a new entity registry.
    pub fn new() -> Self {
        Self {
            active_entities: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get the count of active entities.
    pub async fn active_count(&self) -> usize {
        self.active_entities.lock().await.len()
    }

    /// Get the count of passivated entities.
    pub async fn passivated_count(&self) -> usize {
        // For now, we'll just return 0
        0
    }

    /// Mark an entity as passivated.
    pub async fn mark_passivated(&self, _entity_id: String, _version: u64) {
        // For now, we'll do nothing
    }

    /// Mark an entity as active.
    pub async fn mark_active(&self, entity_id: &str) {
        self.active_entities.lock().await.insert(entity_id.to_string(), true);
    }

    /// Remove an entity from the active registry.
    pub async fn remove_active(&self, entity_id: &str) {
        self.active_entities.lock().await.remove(entity_id);
    }
}