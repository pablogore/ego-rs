use std::sync::Arc;

use ego_domain::DomainEvent;

use crate::persistence::PersistenceFacade;
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::runtime::{EntityRuntime, RuntimeConfig};
use crate::scheduler::Scheduler;
use crate::scheduler_event::{
    event_bus_channel_with_config, SchedulerEventBusConfig,
};
use crate::scheduler_policy::RoundRobinPolicy;
use crate::snapshot::{SnapshotStrategy, PeriodicSnapshotStrategy};

pub struct EntityRuntimeBuilder<E: DomainEvent + Clone + serde::de::DeserializeOwned + Send + Sync + 'static> {
    mailbox_capacity: usize,
    concurrency_budget: usize,
    passivation_timeout: std::time::Duration,
    publisher: Option<Arc<dyn EventPublisher<E>>>,
    snapshot_strategy: Option<Arc<dyn SnapshotStrategy>>,
    single_tenant_mode: bool,
    tenant_id: String,
    registry: Option<Arc<EntityRegistry>>,
    event_bus_capacity: usize,
}

impl<E: DomainEvent + Clone + serde::de::DeserializeOwned + 'static> EntityRuntimeBuilder<E> {
    pub fn new() -> Self {
        EntityRuntimeBuilder {
            mailbox_capacity: 1000,
            concurrency_budget: 10000,
            passivation_timeout: std::time::Duration::from_secs(300),
            publisher: None,
            snapshot_strategy: None,
            single_tenant_mode: true,
            tenant_id: String::new(),
            registry: None,
            event_bus_capacity: 4096,
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

    /// Set the capacity of the bounded scheduler event bus.
    ///
    /// Default: 4096. Higher values reduce event loss under load spikes
    /// at the cost of more memory. Lower values apply backpressure sooner.
    pub fn event_bus_capacity(mut self, capacity: usize) -> Self {
        self.event_bus_capacity = capacity;
        self
    }

    pub fn build(self) -> EntityRuntime<E> {
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

        // Create the bounded scheduler feedback event bus
        let bus_config = SchedulerEventBusConfig {
            capacity: self.event_bus_capacity,
        };
        let (event_sender, event_receiver) = event_bus_channel_with_config(bus_config);

        EntityRuntime::new(
            registry.clone(),
            Arc::new(Scheduler::new(
                registry.clone(),
                Arc::new(RoundRobinPolicy::new(100, 10)),
                event_receiver,
            )),
            Arc::new(persistence),
            publisher,
            config,
            snapshot_strategy,
            event_sender,
        )
    }
}

impl<E: DomainEvent + Clone + serde::de::DeserializeOwned + 'static> Default for EntityRuntimeBuilder<E> {
    fn default() -> Self {
        Self::new()
    }
}
