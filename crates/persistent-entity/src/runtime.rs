//! Runtime orchestration for the persistent entity system.
//!
//! Provides the top-level [`EntityRuntime`] that wires together scheduling,
//! persistence, publishing, and snapshot management.

use std::marker::PhantomData;
use std::sync::Arc;

use ego_domain::event::DomainEvent;

use crate::entity_ref::EntityRef;
use crate::entity_ref_tokio::TokioEntityRef;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::PersistentEntity;
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::{EntityTriple, Scheduler};
use crate::scheduler_event::SchedulerEventSender;
use crate::snapshot::SnapshotStrategy;

/// Configuration for the entity runtime.
///
/// Controls mailbox capacity, concurrency budget, passivation timeout,
/// and tenant isolation settings.
pub struct RuntimeConfig {
    /// Maximum number of commands queued per mailbox.
    pub mailbox_capacity: usize,
    /// Maximum number of concurrently active actors.
    pub concurrency_budget: usize,
    /// Duration of inactivity before entity passivation.
    pub passivation_timeout: std::time::Duration,
    /// When true, all entities share the default tenant scope.
    pub single_tenant_mode: bool,
    /// Tenant identifier used when single_tenant_mode is false.
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

/// Top-level runtime that orchestrates entity lifecycle, scheduling, and persistence.
///
/// Owns shared references to registry, persistence, scheduler, publisher,
/// and snapshot strategy. Creates entity references for command dispatch.
pub struct EntityRuntime<E> {
    /// Shared entity registry.
    pub registry: Arc<EntityRegistry>,
    /// Shared scheduler for activation suggestions.
    pub scheduler: Arc<Scheduler>,
    /// Shared persistence facade.
    pub persistence: Arc<PersistenceFacade<E>>,
    /// Shared event publisher.
    pub publisher: Arc<dyn EventPublisher<E>>,
    /// Runtime configuration.
    pub config: RuntimeConfig,
    /// Snapshot strategy.
    pub snapshot_strategy: Arc<dyn SnapshotStrategy>,
    /// Scheduler event sender for lifecycle events.
    pub event_sender: SchedulerEventSender,
    _event: PhantomData<E>,
}

impl<E> EntityRuntime<E>
where
    E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static,
{
    /// Creates a new [`EntityRuntime`] with the given components.
    pub fn new(
        registry: Arc<EntityRegistry>,
        scheduler: Arc<Scheduler>,
        persistence: Arc<PersistenceFacade<E>>,
        publisher: Arc<dyn EventPublisher<E>>,
        config: RuntimeConfig,
        snapshot_strategy: Arc<dyn SnapshotStrategy>,
        event_sender: SchedulerEventSender,
    ) -> Self {
        EntityRuntime {
            registry,
            scheduler,
            persistence,
            publisher,
            config,
            snapshot_strategy,
            event_sender,
            _event: PhantomData,
        }
    }

    /// Returns a [`TokioEntityRef`] for sending commands to the identified entity.
    ///
    /// Spawns a real [`EntityActor`] via `tokio::spawn` and returns a ref
    /// backed by its mailbox.  This method MUST be called from within a Tokio
    /// runtime context (e.g., inside an `async fn` or `#[tokio::test]`).
    pub fn entity_ref<C, S>(
        &self,
        entity_type: &'static str,
        entity_id: impl Into<String>,
        entity_handler: Arc<dyn PersistentEntity<Command = C, Event = E, State = S>>,
    ) -> impl EntityRef
    where
        C: Send + Sync + serde::Serialize + 'static,
        S: serde::Serialize + Clone + serde::de::DeserializeOwned + Send + Sync + 'static,
    {
        let tenant_id = if self.config.single_tenant_mode {
            "default".to_string()
        } else {
            self.config.tenant_id.clone()
        };

        let triple = EntityTriple::new(tenant_id, entity_type, entity_id);

        TokioEntityRef::new(
            triple,
            self.registry.clone(),
            self.persistence.clone(),
            self.publisher.clone(),
            self.snapshot_strategy.clone(),
            entity_handler,
            self.event_sender.clone(),
            self.config.mailbox_capacity,
            self.config.passivation_timeout,
        )
    }

    /// Returns the number of currently active entities.
    pub async fn active_count(&self) -> usize {
        self.registry.active_count().await
    }

    /// Returns the number of passivated entities.
    pub async fn passivated_count(&self) -> usize {
        self.registry.passivated_count().await
    }
}
