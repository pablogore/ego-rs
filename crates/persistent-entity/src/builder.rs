use std::sync::Arc;

use parking_lot::Mutex;

use ego_domain::persistence::{EventStore, Snapshot};
use ego_domain::DomainEvent;

use crate::effect_acceptor::EffectAcceptor;
use crate::persistence::{InMemoryEventStore, InMemorySnapshotStore, PersistenceFacade};
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::runtime::{EntityRuntime, RuntimeConfig};
use crate::scheduler::Scheduler;
use crate::scheduler_event::{event_bus_channel_with_config, SchedulerEventBusConfig};
use crate::scheduler_policy::RoundRobinPolicy;
use crate::snapshot::{PeriodicSnapshotStrategy, SnapshotStrategy};

pub struct EntityRuntimeBuilder<
    E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static,
> {
    mailbox_capacity: usize,
    concurrency_budget: usize,
    passivation_timeout: std::time::Duration,
    publisher: Option<Arc<dyn EventPublisher<E>>>,
    snapshot_strategy: Option<Arc<dyn SnapshotStrategy>>,
    single_tenant_mode: bool,
    tenant_id: String,
    registry: Option<Arc<EntityRegistry>>,
    event_bus_capacity: usize,
    /// Optionally injected event store. Defaults to in-memory.
    event_store: Option<Arc<Mutex<dyn EventStore<E> + Send>>>,
    /// Optionally injected snapshot store. Defaults to in-memory.
    snapshot_store: Option<Arc<Mutex<dyn Snapshot + Send>>>,
    /// Optional external-effects acceptance port (CORE-019 PR4 F-03 fix),
    /// threaded to every actor this runtime spawns. `None` by default —
    /// spawned actors keep the zero-cost, fail-closed-if-effects-described
    /// behavior unless a host opts in via [`Self::with_effect_acceptor`].
    effect_acceptor: Option<Arc<dyn EffectAcceptor>>,
}

impl<E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static> EntityRuntimeBuilder<E> {
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
            event_store: None,
            snapshot_store: None,
            effect_acceptor: None,
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

    /// Inject a custom event store.  If not set, an [`InMemoryEventStore`] is used.
    pub fn with_event_store(
        mut self,
        store: Arc<Mutex<dyn EventStore<E> + Send>>,
    ) -> Self {
        self.event_store = Some(store);
        self
    }

    /// Apply all fields from a [`RuntimeConfig`] at once.
    ///
    /// Convenience for callers that obtain a typed config from kit-config:
    ///
    /// ```
    /// use persistent_entity::builder::EntityRuntimeBuilder;
    /// use persistent_entity::TestEvent;
    /// use serde_json::json;
    ///
    /// // Simulates a `serde_json::Value` obtained from
    /// // `kit_config::ConfigLoader::get("persistent_entity")`.
    /// let value = json!({
    ///     "mailbox_capacity": 500,
    ///     "concurrency_budget": 50,
    ///     "passivation_timeout_secs": 60,
    ///     "single_tenant_mode": false,
    ///     "tenant_id": "tenant-a"
    /// });
    ///
    /// let config = serde_json::from_value(value).unwrap();
    /// let runtime = EntityRuntimeBuilder::<TestEvent>::new()
    ///     .with_config(config)
    ///     .build();
    ///
    /// assert_eq!(runtime.config.mailbox_capacity, 500);
    /// assert_eq!(runtime.config.tenant_id, "tenant-a");
    /// ```
    pub fn with_config(self, config: RuntimeConfig) -> Self {
        self.mailbox_capacity(config.mailbox_capacity)
            .concurrency_budget(config.concurrency_budget)
            .passivation_timeout(std::time::Duration::from_secs(config.passivation_timeout_secs))
            .single_tenant(config.single_tenant_mode)
            .tenant_id(config.tenant_id)
    }

    /// Deserialize a [`serde_json::Value`] into a [`RuntimeConfig`] and apply it.
    ///
    /// This is the entry point for kit-config integration: callers receive a
    /// `serde_json::Value` from `kit_config::ConfigLoader` and pass it here —
    /// no direct dependency on kit-config is needed in this crate.
    pub fn from_value(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value).map(|c| Self::default().with_config(c))
    }

    /// Inject a custom snapshot store.  If not set, an [`InMemorySnapshotStore`] is used.
    pub fn with_snapshot_store(
        mut self,
        store: Arc<Mutex<dyn Snapshot + Send>>,
    ) -> Self {
        self.snapshot_store = Some(store);
        self
    }

    /// Wires an [`EffectAcceptor`] into every actor this runtime spawns
    /// (CORE-019 PR4 F-03 fix). This is the seam a host uses to connect
    /// `ego_service_sdk::Runtime::effect_acceptor()` (once
    /// `RuntimeBuilder::register_effect_executor` has been called at least
    /// once) to real, production-spawned entity actors — without this call,
    /// a registered executor has zero effect on any actor spawned through
    /// [`crate::runtime::EntityRuntime::entity_ref`], since the actor's own
    /// `effect_acceptor` field defaults to `None`.
    pub fn with_effect_acceptor(mut self, acceptor: Arc<dyn EffectAcceptor>) -> Self {
        self.effect_acceptor = Some(acceptor);
        self
    }

    pub fn build(self) -> EntityRuntime<E> {
        let publisher = self
            .publisher
            .unwrap_or_else(|| Arc::new(crate::testing::NoopPublisher::new()));
        let snapshot_strategy = self
            .snapshot_strategy
            .unwrap_or_else(|| Arc::new(PeriodicSnapshotStrategy::new(100)));

        let config = RuntimeConfig {
            mailbox_capacity: self.mailbox_capacity,
            concurrency_budget: self.concurrency_budget,
            passivation_timeout_secs: self.passivation_timeout.as_secs(),
            single_tenant_mode: self.single_tenant_mode,
            tenant_id: self.tenant_id,
        };

        let event_store: Arc<Mutex<dyn EventStore<E> + Send>> = self
            .event_store
            .unwrap_or_else(|| Arc::new(Mutex::new(InMemoryEventStore::new())));

        let snapshot_store: Arc<Mutex<dyn Snapshot + Send>> = self
            .snapshot_store
            .unwrap_or_else(|| Arc::new(Mutex::new(InMemorySnapshotStore::new())));

        let persistence = PersistenceFacade::with_stores(event_store, snapshot_store);

        // Use the provided registry or create a new one
        let registry = self
            .registry
            .unwrap_or_else(|| Arc::new(EntityRegistry::new()));

        // Create the bounded scheduler feedback event bus
        let bus_config = SchedulerEventBusConfig {
            capacity: self.event_bus_capacity,
        };
        let (event_sender, event_receiver) = event_bus_channel_with_config(bus_config);

        // `new_with_passivation_timeout` (review F1), not `new` — `new` is
        // kept only for source compatibility with existing external callers
        // and still reconstructs the lossy `config.passivation_timeout()`
        // internally. The full-precision `Duration` configured directly via
        // `.passivation_timeout()` is NOT reconstructed from `config` above,
        // whose `passivation_timeout_secs: u64` silently truncates any
        // sub-second value to zero (see `EntityRuntime`'s
        // `passivation_timeout` field doc comment).
        EntityRuntime::new_with_passivation_timeout(
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
            self.effect_acceptor,
            self.passivation_timeout,
        )
    }
}

impl<E: DomainEvent + Clone + serde::de::DeserializeOwned + serde::Serialize + Send + Sync + 'static> Default
    for EntityRuntimeBuilder<E>
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_context::CommandContext;
    use crate::entity_ref::EntityRef;
    use crate::persistent_entity::{CommandResult, PersistentEntity};
    use crate::snapshot::NoSnapshot;
    use crate::test_entity::TestEntity;
    use crate::testing::{TestCommand, TestEvent, TestState};

    /// Regression test for a bug found during CORE-028 Stage-1 flaky-test
    /// triage: `build()` used to reconstruct the actor-facing passivation
    /// timeout from `RuntimeConfig.passivation_timeout_secs: u64`, whose
    /// `.as_secs()` silently truncates any sub-second `Duration` to `0` —
    /// making every spawned actor passivate almost immediately regardless of
    /// what `.passivation_timeout(...)` was actually configured with. Fixed
    /// by threading the original full-precision `Duration` into
    /// `EntityRuntime` directly (see its `passivation_timeout` field).
    /// Advances Tokio's paused virtual clock by `step`, then yields
    /// repeatedly (bounded, not a guessed count) until the runtime reaches
    /// quiescence — review F2: a single `yield_now()` isn't guaranteed to
    /// drive a spawned actor through every one of its own sequential
    /// `.await` points (mailbox drain, snapshot store, state transition)
    /// after its timer fires; this loop keeps yielding only while progress
    /// is still observably happening, bounded so a genuine deadlock still
    /// fails loudly instead of hanging.
    async fn advance_and_settle(step: std::time::Duration) {
        tokio::time::advance(step).await;
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
    }

    // Review F2: deterministic virtual-time advance (`start_paused = true`),
    // not a real `tokio::time::sleep` — the original version slept 100ms and
    // asserted the entity was still active, which is itself a real-clock
    // race under contention (the same class of flakiness this whole fix was
    // triaging): a sufficiently delayed wake-up could observe the entity
    // already passivated even with a correct implementation. Advancing a
    // paused clock is unaffected by scheduling delay.
    #[tokio::test(start_paused = true)]
    async fn sub_second_passivation_timeout_is_not_truncated_to_zero() {
        let runtime = EntityRuntimeBuilder::<TestEvent>::new()
            .passivation_timeout(std::time::Duration::from_millis(300))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build();
        let handler: Arc<dyn PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>> =
            Arc::new(TestEntity::new());

        let entity_ref = runtime
            .entity_ref::<TestCommand, TestState>("test", "regression-entity", handler)
            .unwrap();
        let _: CommandResult<TestEvent, TestState> = entity_ref
            .send_command(TestCommand::Increment(1), CommandContext::new("test".to_string()))
            .await
            .unwrap();
        assert_eq!(runtime.active_count(), 1, "freshly activated entity must be active");

        // Well under the configured 300ms timeout, the entity must still be
        // active — a truncated-to-zero timeout would have already passivated
        // it by now.
        advance_and_settle(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            runtime.active_count(),
            1,
            "entity passivated in well under its configured 300ms timeout — \
             the sub-second Duration was likely truncated to zero again"
        );

        // Past the configured timeout (100ms + 250ms = 350ms > 300ms), it
        // must have genuinely passivated.
        advance_and_settle(std::time::Duration::from_millis(250)).await;
        assert_eq!(
            runtime.active_count(),
            0,
            "entity did not passivate after its configured 300ms timeout elapsed"
        );
    }
}
