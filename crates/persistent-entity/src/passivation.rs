//! Passivation policy and registry for persistent entities.
//!
//! This module handles entity passivation policies and maintains a registry of active entities.

use crate::error::EntityError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A registry for tracking active entities.
#[derive(Debug)]
pub struct EntityRegistry {
    /// The active entities.
    active_entities: Arc<Mutex<HashMap<String, EntityHandle>>>,
}

/// A handle to an active entity.
#[derive(Debug)]
pub struct EntityHandle {
    /// The sender for communicating with the entity.
    pub sender: tokio::sync::mpsc::UnboundedSender<Command>,
}

/// A command that can be sent to an entity.
#[derive(Debug)]
pub enum Command {
    /// A command to process.
    ProcessCommand(Box<dyn Send + Sync>),
    /// A command to passivate the entity.
    Passivate,
}

impl EntityRegistry {
    /// Create a new entity registry.
    pub fn new() -> Self {
        Self {
            active_entities: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get an active sender for an entity.
    pub async fn get_active_sender(
        &self,
        entity_id: &str,
    ) -> Result<Option<EntityHandle>, EntityError> {
        let active_entities = self.active_entities.lock().await;
        Ok(active_entities.get(entity_id).cloned())
    }

    /// Register an active entity.
    pub async fn register_active(
        &self,
        entity_id: String,
        handle: EntityHandle,
    ) -> Result<(), EntityError> {
        let mut active_entities = self.active_entities.lock().await;
        active_entities.insert(entity_id, handle);
        Ok(())
    }

    /// Remove an active entity.
    pub async fn remove_active(&self, entity_id: &str) -> Result<(), EntityError> {
        let mut active_entities = self.active_entities.lock().await;
        active_entities.remove(entity_id);
        Ok(())
    }
}