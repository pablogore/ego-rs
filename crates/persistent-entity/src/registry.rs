//! A simple registry for tracking entities.
//!
//! This module provides a basic registry for tracking entity states.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

const MAX_PASSIVATED_ENTRIES: usize = 10_000;

/// A simple registry for tracking entities.
#[derive(Debug)]
pub struct EntityRegistry {
    /// Currently active entities.
    active_entities: Arc<Mutex<HashSet<String>>>,
    /// Entities that have passivated (aggregate_id → final version).
    passivated_entities: Arc<Mutex<HashMap<String, u64>>>,
}

impl EntityRegistry {
    /// Create a new entity registry.
    pub fn new() -> Self {
        Self {
            active_entities: Arc::new(Mutex::new(HashSet::new())),
            passivated_entities: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get the count of active entities.
    pub fn active_count(&self) -> usize {
        self.active_entities.lock().unwrap().len()
    }

    /// Get the count of passivated entities.
    pub fn passivated_count(&self) -> usize {
        self.passivated_entities.lock().unwrap().len()
    }

    /// Mark an entity as passivated, removing it from the active set.
    ///
    /// Caps the passivated map at `MAX_PASSIVATED_ENTRIES` by evicting one
    /// arbitrary entry when the limit is reached, bounding memory in
    /// high-churn deployments.
    pub fn mark_passivated(&self, entity_id: String, version: u64) {
        self.active_entities.lock().unwrap().remove(&entity_id);
        let mut passivated = self.passivated_entities.lock().unwrap();
        if passivated.len() >= MAX_PASSIVATED_ENTRIES {
            if let Some(oldest) = passivated.keys().next().cloned() {
                passivated.remove(&oldest);
            }
        }
        passivated.insert(entity_id, version);
    }

    /// Mark an entity as active, also removing it from the passivated set.
    pub fn mark_active(&self, entity_id: &str) {
        self.active_entities
            .lock()
            .unwrap()
            .insert(entity_id.to_string());
        self.passivated_entities
            .lock()
            .unwrap()
            .remove(entity_id);
    }

    /// Remove an entity from the active registry.
    pub fn remove_active(&self, entity_id: &str) {
        self.active_entities.lock().unwrap().remove(entity_id);
    }
}
