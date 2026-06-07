use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::activation::SharedActivation;
use crate::scheduler::EntityTriple;

pub struct ActorHandle {
    pub sender: Box<dyn Any + Send + Sync>,
    pub join: tokio::task::JoinHandle<()>,
}

impl ActorHandle {
    pub fn new<C: Send + 'static>(
        sender: tokio::sync::mpsc::Sender<crate::mailbox::CommandEnvelope<C>>,
        join: tokio::task::JoinHandle<()>,
    ) -> Self {
        ActorHandle {
            sender: Box::new(sender),
            join,
        }
    }

    pub fn downcast_sender<C: Send + 'static>(
        &self,
    ) -> Option<&tokio::sync::mpsc::Sender<crate::mailbox::CommandEnvelope<C>>> {
        self.sender.downcast_ref::<tokio::sync::mpsc::Sender<crate::mailbox::CommandEnvelope<C>>>()
    }
}

pub struct PassivationEntry {
    pub last_known_version: u64,
    pub passivated_at: std::time::Instant,
}

pub struct EntityRegistry {
    active: Arc<Mutex<HashMap<EntityTriple, ActorHandle>>>,
    passivated: Arc<Mutex<HashMap<EntityTriple, PassivationEntry>>>,
    pending_activations: Arc<Mutex<HashMap<EntityTriple, Arc<SharedActivation>>>>,
}

impl EntityRegistry {
    pub fn new() -> Self {
        EntityRegistry {
            active: Arc::new(Mutex::new(HashMap::new())),
            passivated: Arc::new(Mutex::new(HashMap::new())),
            pending_activations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_active_sender<C: Send + 'static>(
        &self,
        entity: &EntityTriple,
    ) -> Option<tokio::sync::mpsc::Sender<crate::mailbox::CommandEnvelope<C>>> {
        let active = self.active.lock().await;
        active.get(entity).and_then(|handle| handle.downcast_sender::<C>().cloned())
    }

    pub async fn insert_active(
        &self,
        entity: EntityTriple,
        handle: ActorHandle,
    ) {
        let mut active = self.active.lock().await;
        active.insert(entity, handle);
    }

    pub async fn remove_active(&self, entity: &EntityTriple) {
        let mut active = self.active.lock().await;
        active.remove(entity);
    }

    pub async fn mark_passivated(&self, entity: EntityTriple, version: u64) {
        let mut passivated = self.passivated.lock().await;
        passivated.insert(entity, PassivationEntry {
            last_known_version: version,
            passivated_at: std::time::Instant::now(),
        });
    }

    pub async fn is_passivated(&self, entity: &EntityTriple) -> bool {
        let passivated = self.passivated.lock().await;
        passivated.contains_key(entity)
    }

    pub async fn remove_passivated(&self, entity: &EntityTriple) {
        let mut passivated = self.passivated.lock().await;
        passivated.remove(entity);
    }

    pub async fn get_or_create_activation(
        &self,
        entity: EntityTriple,
    ) -> Arc<SharedActivation> {
        let mut pending = self.pending_activations.lock().await;
        pending.entry(entity)
            .or_insert_with(|| Arc::new(SharedActivation::new()))
            .clone()
    }

    pub async fn remove_activation(&self, entity: &EntityTriple) {
        let mut pending = self.pending_activations.lock().await;
        pending.remove(entity);
    }

    pub async fn exists(&self, entity: &EntityTriple) -> bool {
        let active = self.active.lock().await;
        let passivated = self.passivated.lock().await;
        active.contains_key(entity) || passivated.contains_key(entity)
    }

    pub async fn active_count(&self) -> usize {
        let active = self.active.lock().await;
        active.len()
    }

    pub async fn passivated_count(&self) -> usize {
        let passivated = self.passivated.lock().await;
        passivated.len()
    }
}

impl Default for EntityRegistry {
    fn default() -> Self {
        Self::new()
    }
}
