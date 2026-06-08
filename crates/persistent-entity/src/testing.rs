use std::any::Any;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::command_context::CommandContext;
use crate::entity_ref::EntityRef;
use crate::error::EntityError;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::{CommandResult, PersistentEntity};
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::EntityTriple;
use crate::snapshot::SnapshotStrategy;
use crate::test_entity::TestEntity;
use ego_domain::persistence::{EventStore, Snapshot, PersistenceError, StoredEvent};
use ego_domain::DomainEvent;

fn entity_store() -> &'static Mutex<HashMap<String, TestState>> {
    static STORE: OnceLock<Mutex<HashMap<String, TestState>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}



pub struct NoopPublisher<E> {
    _phantom: std::marker::PhantomData<E>,
}

impl<E> NoopPublisher<E> {
    pub fn new() -> Self {
        NoopPublisher {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<E: Send + Sync + 'static> Default for NoopPublisher<E> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<E: Send + Sync + 'static> EventPublisher<E> for NoopPublisher<E> {
    async fn publish(&self, _events: &[E]) -> Result<(), String> {
        Ok(())
    }
}

/// A test command for use in tests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TestCommand {
    /// Increment command.
    Increment(u64),
    /// Decrement command.
    Decrement(u64),
    /// GetState command.
    GetState,
}

impl TestCommand {
    /// Create a new increment command.
    pub fn increment(value: u64) -> Self {
        Self::Increment(value)
    }

    /// Create a new decrement command.
    pub fn decrement(value: u64) -> Self {
        Self::Decrement(value)
    }

    /// Create a new get state command.
    pub fn get_state() -> Self {
        Self::GetState
    }
}

/// A test event for use in tests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TestEvent {
    /// Incremented event.
    Incremented(u64),
    /// Decremented event.
    Decremented(u64),
}

impl TestEvent {
    /// Create a new incremented event.
    pub fn incremented(value: u64) -> Self {
        Self::Incremented(value)
    }

    /// Create a new decremented event.
    pub fn decremented(value: u64) -> Self {
        Self::Decremented(value)
    }
}

impl DomainEvent for TestEvent {
    fn aggregate_id(&self) -> &str {
        "test-aggregate"
    }

    fn event_type(&self) -> &str {
        match self {
            TestEvent::Incremented(_) => "Incremented",
            TestEvent::Decremented(_) => "Decremented",
        }
    }

    fn payload(&self) -> &serde_json::Value {
        static PAYLOAD: OnceLock<serde_json::Value> = OnceLock::new();
        PAYLOAD.get_or_init(|| serde_json::Value::Null)
    }

    fn occurred_at(&self) -> &DateTime<Utc> {
        // Per-call timestamp. Box::leak is acceptable in test-only code
        // to satisfy the `&DateTime<Utc>` return type without storing a
        // timestamp on the enum variant.
        Box::leak(Box::new(Utc::now()))
    }
}

/// A test state for use in tests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestState {
    /// The state value.
    pub value: u64,
    /// The state version.
    pub version: u64,
}

impl TestState {
    /// Create a new test state.
    pub fn new(value: u64) -> Self {
        Self {
            value,
            version: 0,
        }
    }
}

/// A test entity reference for use in tests.
#[derive(Debug, Clone)]
pub struct TestEntityRef {
    /// The entity type.
    pub entity_type: String,
    /// The entity identifier.
    pub entity_id: String,
    /// The tenant identifier.
    pub tenant_id: Option<String>,
    /// The entity registry for tracking active entities.
    pub registry: Option<std::sync::Arc<EntityRegistry>>,
}

impl TestEntityRef {
    /// Create a new test entity reference.
    pub fn new<C, E, S>(
        triple: EntityTriple,
        registry: std::sync::Arc<EntityRegistry>,
        _persistence: std::sync::Arc<PersistenceFacade<E>>,
        _publisher: std::sync::Arc<dyn EventPublisher<E>>,
        _mailbox_capacity: usize,
        _snapshot_strategy: std::sync::Arc<dyn SnapshotStrategy>,
        _entity_handler: std::sync::Arc<dyn PersistentEntity<Command = C, Event = E, State = S>>,
    ) -> Self {
        Self {
            entity_type: triple.entity_type.to_string(),
            entity_id: triple.entity_id.clone(),
            tenant_id: Some(triple.tenant_id.clone()),
            registry: Some(registry),
        }
    }

    fn key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.tenant_id.as_deref().unwrap_or("default"),
            self.entity_type,
            self.entity_id,
        )
    }
}

#[async_trait]
impl EntityRef for TestEntityRef {
    async fn send_command<T, C>(
        &self,
        command: C,
        context: CommandContext,
    ) -> Result<T, EntityError>
    where
        T: Send + 'static,
        C: Serialize + Send + 'static,
    {
        let tc: &TestCommand = (&command as &dyn Any)
            .downcast_ref::<TestCommand>()
            .ok_or_else(|| EntityError::Internal("expected TestCommand".to_string()))?;

        let entity = TestEntity::new();
        let k = self.key();

        // Register entity as active in registry
        if let Some(ref reg) = self.registry {
            reg.mark_active(&self.entity_id).await;
        }

        // Load current state (drop lock before await)
        let current_state = {
            let mut store = entity_store().lock().unwrap();
            store.entry(k.clone()).or_insert_with(|| entity.initial_state()).clone()
        };

        // Execute command (no lock held across await)
        let events = entity.handle_command(tc, &current_state, &context).await?;

        // Update state
        let result = if events.is_empty() {
            CommandResult::NoEvents {
                state: current_state,
            }
        } else {
            let new_state = entity.apply_events(&current_state, &events).await?;
            // Persist updated state
            let mut store = entity_store().lock().unwrap();
            store.insert(k, new_state.clone());
            CommandResult::Events { new_state, events }
        };

        let boxed: Box<dyn Any> = Box::new(result);
        let downcast: Box<T> = boxed.downcast().map_err(|_| {
            EntityError::Internal("type mismatch: expected CommandResult<TestEvent, TestState>".to_string())
        })?;
        Ok(*downcast)
    }
}

/// In-memory event store for testing.
///
/// Stores events per aggregate per tenant. Enforces optimistic concurrency.
pub struct InMemoryEventStore<E> {
    streams: Mutex<HashMap<String, Vec<StoredEvent<E>>>>,
}

impl<E> InMemoryEventStore<E> {
    pub fn new() -> Self {
        InMemoryEventStore {
            streams: Mutex::new(HashMap::new()),
        }
    }
}

impl<E: Clone + Send + Sync + 'static + DomainEvent> EventStore<E> for InMemoryEventStore<E> {
    fn append(
        &mut self,
        stream_id: &str,
        _tenant_id: Option<&str>,
        version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let mut streams = self.streams.lock().unwrap();
        let stream = streams.entry(stream_id.to_string()).or_insert_with(Vec::new);
        
        // Check expected version
        if stream.len() as i64 != version {
            return Err(PersistenceError::Conflict {
                aggregate_id: stream_id.to_string(),
                expected: version,
                actual: stream.len() as i64,
            });
        }
        
        for event in events {
            stream.push(event);
        }
        
        Ok(stream.len() as i64)
    }

    fn load(
        &self,
        stream_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        let streams = self.streams.lock().unwrap();
        if let Some(stream) = streams.get(stream_id) {
            Ok(stream.clone())
        } else {
            Ok(Vec::new())
        }
    }

    fn list_aggregate_ids(
        &self,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<String>, PersistenceError> {
        let streams = self.streams.lock().unwrap();
        Ok(streams.keys().cloned().collect())
    }
}

/// In-memory snapshot store for testing.
///
/// Stores the latest snapshot per aggregate per tenant.
pub struct InMemorySnapshotStore {
    snapshots: Mutex<HashMap<String, (i64, Vec<u8>)>>,
}

impl InMemorySnapshotStore {
    pub fn new() -> Self {
        InMemorySnapshotStore {
            snapshots: Mutex::new(HashMap::new()),
        }
    }
}

impl Snapshot for InMemorySnapshotStore {
    fn save_snapshot(
        &mut self,
        stream_id: &str,
        _tenant_id: Option<&str>,
        version: i64,
        snapshot: serde_json::Value,
    ) -> Result<(), PersistenceError> {
        let mut snapshots = self.snapshots.lock().unwrap();
        snapshots.insert(stream_id.to_string(), (version, serde_json::to_vec(&snapshot).unwrap()));
        Ok(())
    }

    fn load_snapshot(
        &self,
        stream_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Option<(i64, serde_json::Value)>, PersistenceError> {
        let snapshots = self.snapshots.lock().unwrap();
        if let Some((version, data)) = snapshots.get(stream_id) {
            let snapshot = serde_json::from_slice(data).unwrap();
            Ok(Some((*version, snapshot)))
        } else {
            Ok(None)
        }
    }
}

/// Create a test command context.
pub fn create_test_context() -> CommandContext {
    CommandContext {
        tenant_id: Some("test-tenant".to_string()),
        entity_type: "test-entity".to_string(),
        entity_id: "test-entity-id".to_string(),
        expected_version: None,
        causation_id: None,
        metadata: HashMap::new(),
    }
}


