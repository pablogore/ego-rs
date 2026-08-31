use std::sync::Arc;

use parking_lot::Mutex;

use ego_domain::persistence::{EventStore, Snapshot};
use ego_domain::DomainEvent;

use crate::effect_acceptor::EffectAcceptor;
use crate::error::PersistenceCompositionError;
use crate::persistence::{InMemoryEventStore, InMemorySnapshotStore, PersistenceFacade};
use crate::profile::{require_durably_configured, Profile};
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::runtime::{EntityRuntime, RuntimeConfig};
use crate::scheduler::Scheduler;
use crate::scheduler_event::{event_bus_channel_with_config, SchedulerEventBusConfig};
use crate::scheduler_policy::RoundRobinPolicy;
use crate::snapshot::{PeriodicSnapshotStrategy, SnapshotStrategy};
use ego_domain::Observability;

/// Rounds `d` up to the nearest whole second — never down. Used only for
/// `RuntimeConfig.passivation_timeout_secs`'s own whole-seconds
/// informational representation (see that struct's doc comment); the
/// `EntityRuntime` actors actually spawn with `d` itself, unrounded.
///
/// **`Duration::ZERO` is not special-cased and is NOT "disabled" or "no
/// timeout"** — per `TokioPassivationSignal`, a zero-duration passivation
/// timeout genuinely means the actor idles out on its very first check
/// (confirmed by `passivation_signal.rs`'s
/// `tokio_signal_zero_duration_resolves_immediately`), i.e. "passivate
/// (near-)instantly." Rounding `0` up to `1` here would misrepresent that
/// real, intentional behavior as a full second of idle tolerance, so `0`
/// stays `0`. Every OTHER sub-second remainder still rounds up rather than
/// truncating down to `0`, which would misleadingly suggest the same
/// instant-passivation behavior for a timeout that was actually configured
/// to allow some idle time (review PR #186 finding 3).
fn ceil_secs(d: std::time::Duration) -> u64 {
    if d.subsec_nanos() > 0 {
        d.as_secs().saturating_add(1)
    } else {
        d.as_secs()
    }
}

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
    event_store: Option<Arc<dyn EventStore<E> + Send + Sync>>,
    /// Optionally injected snapshot store. Defaults to in-memory.
    snapshot_store: Option<Arc<Mutex<dyn Snapshot + Send>>>,
    /// Optional external-effects acceptance port (CORE-019 PR4 F-03 fix),
    /// threaded to every actor this runtime spawns. `None` by default —
    /// spawned actors keep the zero-cost, fail-closed-if-effects-described
    /// behavior unless a host opts in via [`Self::with_effect_acceptor`].
    effect_acceptor: Option<Arc<dyn EffectAcceptor>>,
    /// Optional observability sink, threaded to every actor this runtime
    /// spawns. `None` by default — an actor without one takes exactly the path
    /// it took before this existed and pays for no observation it does not
    /// make.
    observability: Option<Arc<dyn Observability>>,
    /// What this composition declares about the deployment it is being built
    /// for (PROD-013, AD-1). `Profile::Dev` by default, preserving today's
    /// behavior byte-for-byte for every existing call site.
    profile: Profile,
}

impl<
        E: DomainEvent
            + Clone
            + serde::de::DeserializeOwned
            + serde::Serialize
            + Send
            + Sync
            + 'static,
    > EntityRuntimeBuilder<E>
{
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
            observability: None,
            profile: Profile::Dev,
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
    pub fn with_event_store(mut self, store: Arc<dyn EventStore<E> + Send + Sync>) -> Self {
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
            .passivation_timeout(std::time::Duration::from_secs(
                config.passivation_timeout_secs,
            ))
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
    pub fn with_snapshot_store(mut self, store: Arc<Mutex<dyn Snapshot + Send>>) -> Self {
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

    /// Wires an [`Observability`] into every actor this runtime spawns.
    ///
    /// Without it, an actor's own `observability` field stays `None` and the
    /// receipt gate emits nothing — the same shape `with_effect_acceptor` has,
    /// and for the same reason: a capability the deployment did not ask for
    /// costs it nothing.
    ///
    /// # This must be called before the runtime is shared
    ///
    /// There is no retroactive path. A host registers an entity runtime as
    /// `Arc<EntityRuntime<_>>`, and
    /// [`crate::runtime::EntityRuntime::with_observability`] consumes `self`, so
    /// once that `Arc` exists nothing can add a sink to the runtime inside it.
    /// Reaching for `ego_service_sdk::Runtime::observability()` does not help
    /// either: that accessor exists only on a **built** `Runtime`, by which point
    /// every entity runtime it holds has already been constructed.
    ///
    /// The host therefore keeps one sink and hands it to both builders:
    ///
    /// ```ignore
    /// let observability: Arc<dyn Observability> = Arc::new(/* ... */);
    ///
    /// let entity_runtime = EntityRuntimeBuilder::<UserEvent>::new()
    ///     .with_observability(observability.clone())
    ///     .build();
    ///
    /// let runtime = RuntimeBuilder::new()
    ///     .with_observability(observability)
    ///     .with_entity::<User>(Arc::new(entity_runtime))?
    ///     .build();
    /// ```
    ///
    /// Wiring only the `RuntimeBuilder` is the failure worth naming: the
    /// reservation and purge signals appear, `idempotency.receipt.outcome` does
    /// not, and nothing reports that half the instrumentation is dark.
    pub fn with_observability(mut self, observability: Arc<dyn Observability>) -> Self {
        self.observability = Some(observability);
        self
    }

    /// Declares what this composition is being built for (PROD-013, AD-1).
    /// `Profile::Dev` by default. Under `Profile::Production`, [`Self::build`]
    /// panics and [`Self::try_build`] refuses whenever the event store or the
    /// snapshot store has no explicitly configured implementation.
    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        self
    }

    /// Whether this configuration can honour the profile it declares.
    ///
    /// The **one** definition of that rule for this builder. `build` panics on
    /// its error and `try_build` returns it, so the two cannot come to
    /// disagree about what a valid configuration is.
    ///
    /// Event store is checked first, deliberately: when both are missing the
    /// caller sees the one they are far more likely to have meant to
    /// configure (AD-3).
    fn validate_persistence(&self) -> Result<(), PersistenceCompositionError> {
        require_durably_configured(
            self.profile,
            self.event_store.as_ref().is_some_and(|s| s.is_durable()),
            "event store",
            "EntityRuntimeBuilder::with_event_store(store)",
        )?;
        require_durably_configured(
            self.profile,
            self.snapshot_store
                .as_ref()
                .is_some_and(|s| s.lock().is_durable()),
            "snapshot store",
            "EntityRuntimeBuilder::with_snapshot_store(store)",
        )
    }

    /// Consumes the builder and produces an [`EntityRuntime`], returning the
    /// profile gate's refusal instead of panicking.
    pub fn try_build(self) -> Result<EntityRuntime<E>, PersistenceCompositionError> {
        // Before delegating, not after. `build` panics on this condition, so
        // checking afterwards would mean this method could never return the
        // error it exists to return — the panic would already have unwound.
        self.validate_persistence()?;
        Ok(self.build())
    }

    /// Consumes the builder and produces an [`EntityRuntime`].
    ///
    /// # Panics
    ///
    /// Panics when [`Profile::Production`] is declared and a gated persistent
    /// capability has no configured implementation.
    ///
    /// A panic rather than a `Result`, because this signature is what all 67
    /// existing call sites already call, and because the alternative is worse
    /// than a loud stop: a runtime that declares production and silently writes
    /// every event into process memory loses them on the next restart, and
    /// reports nothing. Bootstrap is the cheapest moment to refuse.
    ///
    /// [`Self::try_build`] returns the same condition as a structured error.
    pub fn build(self) -> EntityRuntime<E> {
        if let Err(err) = self.validate_persistence() {
            panic!("{err}");
        }
        let publisher = self
            .publisher
            .unwrap_or_else(|| Arc::new(crate::testing::NoopPublisher::new()));
        let snapshot_strategy = self
            .snapshot_strategy
            .unwrap_or_else(|| Arc::new(PeriodicSnapshotStrategy::new(100)));

        let config = RuntimeConfig {
            mailbox_capacity: self.mailbox_capacity,
            concurrency_budget: self.concurrency_budget,
            // Rounded UP (never truncated down to 0) — see RuntimeConfig's
            // doc comment. `EntityRuntime::passivation_timeout()` carries
            // the exact value; this field is only this struct's own
            // whole-seconds informational approximation.
            passivation_timeout_secs: ceil_secs(self.passivation_timeout),
            single_tenant_mode: self.single_tenant_mode,
            tenant_id: self.tenant_id,
        };

        let event_store: Arc<dyn EventStore<E> + Send + Sync> = self
            .event_store
            .unwrap_or_else(|| Arc::new(InMemoryEventStore::new()));

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
        let runtime = EntityRuntime::new_with_passivation_timeout(
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
        );
        match self.observability {
            Some(observability) => runtime.with_observability(observability),
            None => runtime,
        }
    }
}

impl<
        E: DomainEvent
            + Clone
            + serde::de::DeserializeOwned
            + serde::Serialize
            + Send
            + Sync
            + 'static,
    > Default for EntityRuntimeBuilder<E>
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
    use ego_domain::operation::OperationReceipt;
    use ego_domain::persistence::{EventStoreUnitOfWork, PersistenceError, StoredEvent};

    /// A stub event store that declares itself durable without doing any
    /// real I/O. Used only to isolate the *other* capability's check in a
    /// partial-configuration test — `InMemoryEventStore` there would make
    /// both capabilities register as "configured" but not durable, hiding
    /// which one the test is actually about (AD-3: presence is not
    /// durability).
    struct DurableStubEventStore;

    #[async_trait::async_trait]
    impl EventStore<TestEvent> for DurableStubEventStore {
        fn is_durable(&self) -> bool {
            true
        }

        async fn append(
            &self,
            _aggregate_type: &str,
            _aggregate_id: &str,
            _tenant_id: Option<&str>,
            _expected_version: i64,
            _events: Vec<StoredEvent<TestEvent>>,
        ) -> Result<i64, PersistenceError> {
            unreachable!("stub only satisfies the durability check; never called")
        }

        async fn load(
            &self,
            _aggregate_type: &str,
            _aggregate_id: &str,
            _tenant_id: Option<&str>,
        ) -> Result<Vec<StoredEvent<TestEvent>>, PersistenceError> {
            unreachable!("stub only satisfies the durability check; never called")
        }

        async fn list_aggregate_ids(
            &self,
            _tenant_id: Option<&str>,
        ) -> Result<Vec<(String, String)>, PersistenceError> {
            unreachable!("stub only satisfies the durability check; never called")
        }

        async fn begin(
            &self,
        ) -> Result<Box<dyn EventStoreUnitOfWork<TestEvent>>, PersistenceError> {
            unreachable!("stub only satisfies the durability check; never called")
        }

        async fn find_receipt(
            &self,
            _aggregate_type: &str,
            _aggregate_id: &str,
            _tenant_id: Option<&str>,
            _operation_key: &str,
        ) -> Result<Option<OperationReceipt>, PersistenceError> {
            unreachable!("stub only satisfies the durability check; never called")
        }
    }

    /// A stub snapshot store that declares itself durable without doing any
    /// real I/O — the `Snapshot`-trait counterpart of
    /// [`DurableStubEventStore`], for the same isolation reason.
    struct DurableStubSnapshotStore;

    impl Snapshot for DurableStubSnapshotStore {
        fn is_durable(&self) -> bool {
            true
        }

        fn save_snapshot(
            &mut self,
            _aggregate_id: &str,
            _tenant_id: Option<&str>,
            _version: i64,
            _payload: serde_json::Value,
        ) -> Result<(), PersistenceError> {
            unreachable!("stub only satisfies the durability check; never called")
        }

        fn load_snapshot(
            &self,
            _aggregate_id: &str,
            _tenant_id: Option<&str>,
        ) -> Result<Option<(i64, serde_json::Value)>, PersistenceError> {
            unreachable!("stub only satisfies the durability check; never called")
        }
    }

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

    /// Regression test for a bug found during CORE-028 Stage-1 flaky-test
    /// triage: `build()` used to reconstruct the actor-facing passivation
    /// timeout from `RuntimeConfig.passivation_timeout_secs: u64`, whose
    /// `.as_secs()` silently truncates any sub-second `Duration` to `0` —
    /// making every spawned actor passivate almost immediately regardless of
    /// what `.passivation_timeout(...)` was actually configured with. Fixed
    /// by threading the original full-precision `Duration` into
    /// `EntityRuntime` directly (see its `passivation_timeout` field).
    ///
    /// Review F2: uses deterministic virtual-time advance
    /// (`start_paused = true`), not a real `tokio::time::sleep` — the
    /// original version slept 100ms and asserted the entity was still
    /// active, which is itself a real-clock race under contention (the same
    /// class of flakiness this whole fix was triaging): a sufficiently
    /// delayed wake-up could observe the entity already passivated even
    /// with a correct implementation. Advancing a paused clock is
    /// unaffected by scheduling delay.
    #[tokio::test(start_paused = true)]
    async fn sub_second_passivation_timeout_is_not_truncated_to_zero() {
        let runtime = EntityRuntimeBuilder::<TestEvent>::new()
            .passivation_timeout(std::time::Duration::from_millis(300))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build();
        let handler: Arc<
            dyn PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>,
        > = Arc::new(TestEntity::new());

        let entity_ref = runtime
            .entity_ref::<TestCommand, TestState>("test", "regression-entity", handler)
            .unwrap();
        let _: CommandResult<TestEvent, TestState> = entity_ref
            .send_command(
                TestCommand::Increment(1),
                CommandContext::new("test".to_string()),
            )
            .await
            .unwrap();
        assert_eq!(
            runtime.active_count(),
            1,
            "freshly activated entity must be active"
        );

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

    // Follow-up gotcha found by this repo's own pre-commit review after the
    // F1/F2 fixes above: `EntityRuntime.config.passivation_timeout_secs` is
    // still populated via a `Duration`-to-`u64`-seconds conversion, so it
    // could still silently read `0` for a sub-second-configured runtime —
    // moving the same truncation bug from actor behavior to a public
    // introspection field. Fixed by rounding up (`ceil_secs`) instead of
    // truncating, and by adding `EntityRuntime::passivation_timeout()` as
    // the actual ground-truth accessor.
    #[test]
    fn ceil_secs_rounds_up_but_never_truncates_a_nonzero_duration_to_zero() {
        assert_eq!(
            ceil_secs(std::time::Duration::ZERO),
            0,
            "zero is genuinely an instant/near-immediate passivation timeout, not \"disabled\" \
             — it must not be rounded up to a misleading 1-second tolerance"
        );
        assert_eq!(
            ceil_secs(std::time::Duration::from_millis(300)),
            1,
            "a sub-second nonzero duration must round up to 1, never truncate to 0"
        );
        assert_eq!(
            ceil_secs(std::time::Duration::from_secs(1)),
            1,
            "an exact second stays exact"
        );
        assert_eq!(
            ceil_secs(std::time::Duration::from_millis(1500)),
            2,
            "any remainder past a whole second rounds up to the next one"
        );
        assert_eq!(
            ceil_secs(std::time::Duration::MAX),
            u64::MAX,
            "rounding up must saturate at u64::MAX, never overflow/panic on an extreme Duration"
        );
    }

    #[test]
    fn entity_runtime_passivation_timeout_is_the_exact_configured_value() {
        let runtime = EntityRuntimeBuilder::<TestEvent>::new()
            .passivation_timeout(std::time::Duration::from_millis(300))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build();

        assert_eq!(
            runtime.passivation_timeout(),
            std::time::Duration::from_millis(300),
            "EntityRuntime::passivation_timeout() must be the exact configured Duration"
        );
        assert_eq!(
            runtime.config.passivation_timeout_secs, 1,
            "the informational config field must round up (never read a misleading 0)"
        );
    }

    /// SC-1: `Profile::Production` with no event store configured must be
    /// refused, naming the missing capability and the exact fixing call.
    #[test]
    fn try_build_rejects_missing_event_store_under_production() {
        let result = EntityRuntimeBuilder::<TestEvent>::new()
            .profile(Profile::Production)
            .with_snapshot_store(Arc::new(Mutex::new(InMemorySnapshotStore::new())))
            .try_build();
        let err = match result {
            Err(err) => err,
            Ok(_) => panic!("no event store configured under Profile::Production must refuse"),
        };

        let message = err.to_string();
        assert!(
            message.contains("event store"),
            "must name the missing capability: {message}"
        );
        assert!(
            message.contains("EntityRuntimeBuilder::with_event_store(store)"),
            "must name the exact fixing call: {message}"
        );
    }

    /// SC-2: `Profile::Production` with no snapshot store configured must be
    /// refused, naming the missing capability and the exact fixing call.
    #[test]
    fn try_build_rejects_missing_snapshot_store_under_production() {
        let result = EntityRuntimeBuilder::<TestEvent>::new()
            .profile(Profile::Production)
            .with_event_store(Arc::new(DurableStubEventStore))
            .try_build();
        let err = match result {
            Err(err) => err,
            Ok(_) => panic!("no snapshot store configured under Profile::Production must refuse"),
        };

        let message = err.to_string();
        assert!(
            message.contains("snapshot store"),
            "must name the missing capability: {message}"
        );
        assert!(
            message.contains("EntityRuntimeBuilder::with_snapshot_store(store)"),
            "must name the exact fixing call: {message}"
        );
    }

    /// SC-6 / AD-7's subsumption / EC-1's asymmetric site 15: a partial
    /// configuration — exactly one of the two stores configured — must be
    /// refused under `Profile::Production`, identifying whichever store is
    /// actually missing. Both orderings are exercised: event-only (mirroring
    /// EC-1's real site 15, which wires an event store and forgets the
    /// snapshot store) and snapshot-only.
    #[test]
    fn try_build_rejects_partial_configuration_under_production() {
        let event_only_result = EntityRuntimeBuilder::<TestEvent>::new()
            .profile(Profile::Production)
            .with_event_store(Arc::new(DurableStubEventStore))
            .try_build();
        let event_only_err = match event_only_result {
            Err(err) => err,
            Ok(_) => panic!("event store configured, snapshot store missing, must refuse"),
        };
        assert!(
            event_only_err.to_string().contains("snapshot store"),
            "must identify the snapshot store as the missing one: {event_only_err}"
        );

        let snapshot_only_result = EntityRuntimeBuilder::<TestEvent>::new()
            .profile(Profile::Production)
            .with_snapshot_store(Arc::new(Mutex::new(InMemorySnapshotStore::new())))
            .try_build();
        let snapshot_only_err = match snapshot_only_result {
            Err(err) => err,
            Ok(_) => panic!("snapshot store configured, event store missing, must refuse"),
        };
        assert!(
            snapshot_only_err.to_string().contains("event store"),
            "must identify the event store as the missing one: {snapshot_only_err}"
        );
    }

    /// SC-5: `Profile::Dev` (the default) must still build on nothing
    /// configured, falling back to in-memory stores exactly as before this
    /// change.
    #[test]
    fn dev_profile_builds_on_nothing_configured() {
        EntityRuntimeBuilder::<TestEvent>::new()
            .profile(Profile::Dev)
            .try_build()
            .expect("Profile::Dev with nothing configured must still build");
    }

    /// `build()` must panic on the exact same condition `try_build()`
    /// refuses (AD-4/AD-6) — the gate must not be decorative on the
    /// infallible path.
    #[test]
    #[should_panic(expected = "event store")]
    fn build_panics_on_same_condition_try_build_refuses() {
        let _ = EntityRuntimeBuilder::<TestEvent>::new()
            .profile(Profile::Production)
            .build();
    }

    /// AD-3's central guarantee, the exact scenario a reviewer flagged
    /// before this was implemented: `Profile::Production` must refuse an
    /// *explicitly wired* volatile store, not just a missing one.
    /// `is_some()` cannot tell `InMemoryEventStore` apart from
    /// `PostgreSQLEventStore` — only `is_durable()` can, and this is what
    /// proves the gate actually calls it.
    #[test]
    fn try_build_rejects_an_explicit_in_memory_store_under_production() {
        let event_store_result = EntityRuntimeBuilder::<TestEvent>::new()
            .profile(Profile::Production)
            .with_event_store(Arc::new(InMemoryEventStore::new()))
            .with_snapshot_store(Arc::new(Mutex::new(DurableStubSnapshotStore)))
            .try_build();
        assert!(
            event_store_result.is_err(),
            "an explicitly-wired InMemoryEventStore must not satisfy Profile::Production"
        );

        let snapshot_store_result = EntityRuntimeBuilder::<TestEvent>::new()
            .profile(Profile::Production)
            .with_event_store(Arc::new(DurableStubEventStore))
            .with_snapshot_store(Arc::new(Mutex::new(InMemorySnapshotStore::new())))
            .try_build();
        assert!(
            snapshot_store_result.is_err(),
            "an explicitly-wired InMemorySnapshotStore must not satisfy Profile::Production"
        );
    }
}
