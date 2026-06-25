//! A simple registry for tracking entities.
//!
//! This module provides a basic registry for tracking entity states.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A simple registry for tracking entities.
#[derive(Debug)]
pub struct EntityRegistry {
    /// Currently active entities (aggregate_id → true).
    active_entities: Arc<Mutex<HashMap<String, bool>>>,
    /// Entities that have passivated (aggregate_id → final version).
    passivated_entities: Arc<Mutex<HashMap<String, u64>>>,
}

impl EntityRegistry {
    /// Create a new entity registry.
    pub fn new() -> Self {
        Self {
            active_entities: Arc::new(Mutex::new(HashMap::new())),
            passivated_entities: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get the count of active entities.
    pub async fn active_count(&self) -> usize {
        self.active_entities.lock().await.len()
    }

    /// Get the count of passivated entities.
    pub async fn passivated_count(&self) -> usize {
        self.passivated_entities.lock().await.len()
    }

    /// Mark an entity as passivated, removing it from the active set.
    pub async fn mark_passivated(&self, entity_id: String, version: u64) {
        self.active_entities.lock().await.remove(&entity_id);
        self.passivated_entities
            .lock()
            .await
            .insert(entity_id, version);
    }

    /// Mark an entity as active.
    pub async fn mark_active(&self, entity_id: &str) {
        self.active_entities
            .lock()
            .await
            .insert(entity_id.to_string(), true);
    }

    /// Remove an entity from the active registry.
    pub async fn remove_active(&self, entity_id: &str) {
        self.active_entities.lock().await.remove(entity_id);
    }
}
