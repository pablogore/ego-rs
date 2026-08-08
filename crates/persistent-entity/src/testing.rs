use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

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
use ego_domain::DomainEvent;

// Re-export persistence stores so integration tests and external crates can
// use them without importing from `persistence` directly.
pub use crate::persistence::{InMemoryEventStore, InMemorySnapshotStore};

/// Convenience constructor that returns a fresh, isolated test store.
pub struct TestStore;

impl TestStore {
    /// Returns a new, empty store — each test should create its own.
    // Test helper: `new` intentionally returns a shared store handle, not `Self`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Arc<Mutex<HashMap<String, TestState>>> {
        Arc::new(Mutex::new(HashMap::new()))
    }
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
        static PINNED: OnceLock<DateTime<Utc>> = OnceLock::new();
        PINNED.get_or_init(|| {
            DateTime::from_timestamp(1_750_000_000, 0).expect("valid pinned timestamp")
        })
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
        Self { value, version: 0 }
    }
}

/// A test entity reference for use in tests.
///
/// Handles commands inline without spawning an actor. Suitable for unit
/// tests that do not need the full actor lifecycle.
#[derive(Debug, Clone)]
pub struct TestEntityRef {
    /// The entity type.
    pub entity_type: String,
    /// The entity identifier.
    pub entity_id: String,
    /// The tenant identifier.
    pub tenant_id: Option<String>,
    /// The entity registry for tracking active entities.
    pub registry: Option<Arc<EntityRegistry>>,
    /// Per-instance isolated store — no global shared state.
    pub store: Arc<Mutex<HashMap<String, TestState>>>,
}

impl TestEntityRef {
    /// Create a new test entity reference.
    // Mirrors the production constructor's dependency set; params struct is out of scope.
    #[allow(clippy::too_many_arguments)]
    pub fn new<C, E, S>(
        triple: EntityTriple,
        registry: Arc<EntityRegistry>,
        _persistence: Arc<PersistenceFacade<E>>,
        _publisher: Arc<dyn EventPublisher<E>>,
        _mailbox_capacity: usize,
        _snapshot_strategy: Arc<dyn SnapshotStrategy>,
        _entity_handler: Arc<dyn PersistentEntity<Command = C, Event = E, State = S>>,
        store: Arc<Mutex<HashMap<String, TestState>>>,
    ) -> Self {
        Self {
            entity_type: triple.entity_type.to_string(),
            entity_id: triple.entity_id.clone(),
            tenant_id: Some(triple.tenant_id.clone()),
            registry: Some(registry),
            store,
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
    type Command = TestCommand;

    async fn send_command<T>(
        &self,
        command: TestCommand,
        context: CommandContext,
    ) -> Result<T, EntityError>
    where
        T: Send + 'static,
    {
        let entity = TestEntity::new();
        let k = self.key();
        let tc = &command;

        // ponytail: TestEntityRef handles commands inline against its own in-memory
        // `store`, never through a real registry-routed actor, so there is no live
        // routing entry to mark — the registry field is accepted for API parity only.

        // Load current state (drop lock before await)
        let current_state = {
            let mut store = self.store.lock().unwrap();
            store
                .entry(k.clone())
                .or_insert_with(|| entity.initial_state())
                .clone()
        };

        // Execute command (no lock held across await)
        let events = entity.handle_command(tc, &current_state, &context).await?;

        let result = if events.is_empty() {
            CommandResult::NoEvents {
                state: current_state,
            }
        } else {
            let new_state = entity.apply_events(&current_state, &events).await?;
            let mut store = self.store.lock().unwrap();
            store.insert(k, new_state.clone());
            CommandResult::Events { new_state, events }
        };

        let boxed: Box<dyn Any> = Box::new(result);
        let downcast: Box<T> = boxed.downcast().map_err(|_| {
            EntityError::Internal(
                "type mismatch: expected CommandResult<TestEvent, TestState>".to_string(),
            )
        })?;
        Ok(*downcast)
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
        operation_key: None,
        fingerprint: None,
    }
}
