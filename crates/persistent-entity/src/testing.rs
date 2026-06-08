use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::command_context::CommandContext;
use crate::entity_ref::EntityRef;
use crate::error::EntityError;
use crate::persistence::PersistenceFacade;
use crate::persistent_entity::PersistentEntity;
use crate::publisher::EventPublisher;
use crate::registry::EntityRegistry;
use crate::scheduler::EntityTriple;
use crate::snapshot::SnapshotStrategy;



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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCommand {
    /// The command type.
    pub command_type: String,
    /// The command data.
    pub data: String,
}

/// A test event for use in tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestEvent {
    /// The event type.
    pub event_type: String,
    /// The event data.
    pub data: String,
}

/// A test state for use in tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestState {
    /// The state value.
    pub value: u64,
    /// The state version.
    pub version: u64,
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
}

impl TestEntityRef {
    /// Create a new test entity reference.
    pub fn new<C, E, S>(
        _triple: EntityTriple,
        _registry: std::sync::Arc<EntityRegistry>,
        _persistence: std::sync::Arc<PersistenceFacade<E>>,
        _publisher: std::sync::Arc<dyn EventPublisher<E>>,
        _mailbox_capacity: usize,
        _snapshot_strategy: std::sync::Arc<dyn SnapshotStrategy>,
        _entity_handler: std::sync::Arc<dyn PersistentEntity<Command = C, Event = E, State = S>>,
    ) -> Self {
        Self {
            entity_type: "test".to_string(),
            entity_id: "test".to_string(),
            tenant_id: Some("test".to_string()),
        }
    }
}

#[async_trait]
impl EntityRef for TestEntityRef {
    async fn send_command<T, C>(
        &self,
        _command: C,
        _context: CommandContext,
    ) -> Result<T, EntityError>
    where
        T: Send + 'static,
        C: Serialize + Send + 'static,
    {
        // This is a test implementation that does nothing
        // In a real implementation, this would send the command to the entity
        unimplemented!("Test implementation")
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
