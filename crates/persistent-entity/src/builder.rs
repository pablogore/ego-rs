use std::sync::Arc;

use ego_domain::DomainEvent;
use ego_domain::persistence::EventStore;

use crate::persistence::PersistenceFacade;
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::runtime::{EntityRuntime, RuntimeConfig};
use crate::scheduler::Scheduler;
use crate::snapshot::{SnapshotStrategy, PeriodicSnapshotStrategy};

pub struct EntityRuntimeBuilder<E: DomainEvent> {
    mailbox_capacity: usize,
    concurrency_budget: usize,
    passivation_timeout: std::time::Duration,
    event_store: Option<Box<dyn EventStore<E> + Send>>,
    snapshot_store: Option<Box<dyn ego_domain::persistence::Snapshot + Send>>,
    publisher: Option<Arc<dyn EventPublisher<E>>>,
    snapshot_strategy: Option<Arc<dyn SnapshotStrategy>>,
    single_tenant_mode: bool,
    tenant_id: String,
    registry: Option<Arc<EntityRegistry>>,
}

impl<E: DomainEvent + Clone + serde::de::DeserializeOwned + 'static> EntityRuntimeBuilder<E> {
    pub fn new() -> Self {
        EntityRuntimeBuilder {
            mailbox_capacity: 1000,
            concurrency_budget: 10000,
            passivation_timeout: std::time::Duration::from_secs(300),
            event_store: None,
            snapshot_store: None,
            publisher: None,
            snapshot_strategy: None,
            single_tenant_mode: true,
            tenant_id: String::new(),
            registry: None,
        }
    }

    pub fn mailbox_capacity(mut self, cap: usize) -> Self {
        self.mailbox_capacity = cap;
        self
    }

    pub fn concurrency_budget(mut self, budget: usize) -> Self {
        self.concurrency_budget = budget;
        self
    }

    pub fn passivation_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.passivation_timeout = timeout;
        self
    }

    pub fn snapshot_strategy(mut self, strategy: Arc<dyn SnapshotStrategy>) -> Self {
        self.snapshot_strategy = Some(strategy);
        self
    }

    pub fn with_event_store(mut self, store: Box<dyn EventStore<E> + Send>) -> Self {
        self.event_store = Some(store);
        self
    }

    pub fn with_snapshot_store(mut self, store: Box<dyn ego_domain::persistence::Snapshot + Send>) -> Self {
        self.snapshot_store = Some(store);
        self
    }

    pub fn with_publisher(mut self, publisher: Arc<dyn EventPublisher<E>>) -> Self {
        self.publisher = Some(publisher);
        self
    }

    pub fn single_tenant(mut self, enabled: bool) -> Self {
        self.single_tenant_mode = enabled;
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = tenant_id.into();
        self
    }

    pub fn with_registry(mut self, registry: Arc<EntityRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn build(self) -> EntityRuntime<E> {
        let _event_store = self.event_store.unwrap_or_else(|| {
            Box::new(crate::testing::InMemoryEventStore::new())
        });
        let _snapshot_store = self.snapshot_store.unwrap_or_else(|| {
            Box::new(crate::testing::InMemorySnapshotStore::new())
        });
        let publisher = self.publisher.unwrap_or_else(|| {
            Arc::new(crate::testing::NoopPublisher::new())
        });
        let snapshot_strategy = self.snapshot_strategy.unwrap_or_else(|| {
            Arc::new(PeriodicSnapshotStrategy::new(100))
        });

        let config = RuntimeConfig {
            mailbox_capacity: self.mailbox_capacity,
            concurrency_budget: self.concurrency_budget,
            passivation_timeout: self.passivation_timeout,
            single_tenant_mode: self.single_tenant_mode,
            tenant_id: self.tenant_id,
        };

        let persistence = PersistenceFacade::new();
        
        // Use the provided registry or create a new one
        let registry = self.registry.unwrap_or_else(|| Arc::new(EntityRegistry::new()));

        EntityRuntime::new(
            registry.clone(),
            Arc::new(Scheduler::new(registry.clone())),
            Arc::new(persistence),
            publisher,
            config,
            snapshot_strategy,
        )
    }
}

impl<E: DomainEvent + Clone + serde::de::DeserializeOwned + 'static> Default for EntityRuntimeBuilder<E> {
    fn default() -> Self {
        Self::new()
    }
}