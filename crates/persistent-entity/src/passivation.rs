use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A registry for managing active entities.
pub struct EntityRegistry {
    /// The active entities.
    active_entities: Arc<Mutex<HashMap<String, EntityHandle>>>,
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
    ) -> Option<EntityHandle> {
        let active_entities = self.active_entities.lock().await;
        active_entities.get(entity_id).cloned()
    }

    /// Register an active entity.
    pub async fn register_active(
        &self,
        entity_id: String,
        handle: EntityHandle,
    ) -> Result<(), ()> {
        let mut active_entities = self.active_entities.lock().await;
        active_entities.insert(entity_id, handle);
        Ok(())
    }

    /// Unregister an active entity.
    pub async fn unregister_active(&self, entity_id: &str) -> Result<(), ()> {
        let mut active_entities = self.active_entities.lock().await;
        active_entities.remove(entity_id);
        Ok(())
    }
}

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