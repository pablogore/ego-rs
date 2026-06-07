use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;

use crate::entity_ref::EntityRef;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::PersistentEntity;
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::{Scheduler, EntityTriple};
use crate::snapshot::SnapshotStrategy;

pub struct RuntimeConfig {
    pub mailbox_capacity: usize,
    pub concurrency_budget: usize,
    pub passivation_timeout: std::time::Duration,
    pub single_tenant_mode: bool,
    pub tenant_id: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        RuntimeConfig {
            mailbox_capacity: 1000,
            concurrency_budget: 10000,
            passivation_timeout: std::time::Duration::from_secs(300),
            single_tenant_mode: true,
            tenant_id: String::new(),
        }
    }
}

pub struct EntityRuntime<E: DomainEvent> {
    pub registry: Arc<EntityRegistry>,
    pub scheduler: Arc<Scheduler>,
    pub persistence: Arc<PersistenceFacade<E>>,
    pub publisher: Arc<dyn EventPublisher<E>>,
    pub config: RuntimeConfig,
    pub snapshot_strategy: Arc<dyn SnapshotStrategy>,
    _event: PhantomData<E>,
}

impl<E: DomainEvent + Clone + serde::de::DeserializeOwned + 'static> EntityRuntime<E> {
    pub fn new(
        registry: Arc<EntityRegistry>,
        scheduler: Arc<Scheduler>,
        persistence: Arc<PersistenceFacade<E>>,
        publisher: Arc<dyn EventPublisher<E>>,
        config: RuntimeConfig,
        snapshot_strategy: Arc<dyn SnapshotStrategy>,
    ) -> Self {
        EntityRuntime {
            registry,
            scheduler,
            persistence,
            publisher,
            config,
            snapshot_strategy,
            _event: PhantomData,
        }
    }

    pub fn entity_ref<C, S>(
        &self,
        entity_type: &'static str,
        entity_id: impl Into<String>,
        entity_handler: Arc<dyn PersistentEntity<C, E, S>>,
    ) -> EntityRef<C, E, S>
    where
        C: Send + 'static,
        S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + 'static,
    {
        let tenant_id = if self.config.single_tenant_mode {
            "default".to_string()
        } else {
            self.config.tenant_id.clone()
        };

        let triple = EntityTriple::new(tenant_id, entity_type, entity_id);

        EntityRef::new(
            triple,
            self.registry.clone(),
            self.persistence.clone(),
            self.publisher.clone(),
            self.config.mailbox_capacity,
            self.snapshot_strategy.clone(),
            entity_handler,
        )
    }

    pub async fn active_count(&self) -> usize {
        self.registry.active_count().await
    }

    pub async fn passivated_count(&self) -> usize {
        self.registry.passivated_count().await
    }
}
