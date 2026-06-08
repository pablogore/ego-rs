use crate::error::EntityError;
use crate::registry::EntityRegistry;
use crate::scheduler::EntityTriple;
use std::sync::Arc;

pub struct Supervisor {
    registry: Arc<EntityRegistry>,
}

impl Supervisor {
    pub fn new(registry: Arc<EntityRegistry>) -> Self {
        Supervisor { registry }
    }

    pub async fn on_actor_failure(
        &self,
        entity: &EntityTriple,
        error: &EntityError,
    ) {
        log::error!(
            "Actor failed for entity {}: {}",
            entity.aggregate_id(),
            error
        );
        self.registry.remove_active(&entity.aggregate_id()).await;
    }

    pub async fn on_recovery_failure(
        &self,
        entity: &EntityTriple,
        error: &EntityError,
    ) {
        log::error!(
            "Recovery failed for entity {}: {}",
            entity.aggregate_id(),
            error
        );
        self.registry.remove_active(&entity.aggregate_id()).await;
    }
}
