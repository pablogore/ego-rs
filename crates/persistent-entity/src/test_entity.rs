use async_trait::async_trait;
use crate::persistent_entity::PersistentEntity;
use crate::command_context::CommandContext;
use crate::error::EntityError;
use crate::testing::{TestCommand, TestEvent, TestState};
use ego_domain::DomainEvent;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct TestEntity;

impl TestEntity {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl PersistentEntity for TestEntity {
    type Command = TestCommand;
    type Event = TestEvent;
    type State = TestState;

    fn initial_state(&self) -> Self::State {
        TestState {
            value: 0,
            version: 0,
        }
    }

    async fn handle_command(
        &self,
        command: &Self::Command,
        state: &Self::State,
        _context: &CommandContext,
    ) -> Result<Vec<Self::Event>, EntityError> {
        match command {
            TestCommand::Increment(value) => {
                Ok(vec![TestEvent::Incremented(*value)])
            }
            TestCommand::Decrement(value) => {
                if state.value < *value {
                    Err(EntityError::Internal("Cannot decrement below zero".to_string()))
                } else {
                    Ok(vec![TestEvent::Decremented(*value)])
                }
            }
            TestCommand::GetState => {
                Ok(vec![])
            }
        }
    }

    async fn apply_event(
        &self,
        state: &Self::State,
        event: &Self::Event,
    ) -> Result<Self::State, EntityError> {
        let mut new_state = state.clone();
        match event {
            TestEvent::Incremented(value) => {
                new_state.value += value;
                new_state.version += 1;
            }
            TestEvent::Decremented(value) => {
                new_state.value -= value;
                new_state.version += 1;
            }
        }
        Ok(new_state)
    }

    async fn apply_events(
        &self,
        state: &Self::State,
        events: &[Self::Event],
    ) -> Result<Self::State, EntityError> {
        let mut new_state = state.clone();
        for event in events {
            match event {
                TestEvent::Incremented(value) => {
                    new_state.value += value;
                    new_state.version += 1;
                }
                TestEvent::Decremented(value) => {
                    new_state.value -= value;
                    new_state.version += 1;
                }
            }
        }
        Ok(new_state)
    }
}

impl DomainEvent for TestEvent {
    fn aggregate_id(&self) -> &str {
        // For test purposes, we'll return a static string
        "test-aggregate"
    }

    fn event_type(&self) -> &str {
        match self {
            TestEvent::Incremented(_) => "TestEvent.Incremented",
            TestEvent::Decremented(_) => "TestEvent.Decremented",
        }
    }

    fn payload(&self) -> &serde_json::Value {
        // For test purposes, we'll return a static value
        &serde_json::Value::Null
    }

    fn occurred_at(&self) -> &chrono::DateTime<chrono::Utc> {
        // For test purposes, we'll return a static time
        // This is a workaround for the temporary value issue
        // We need to create a static DateTime for testing purposes
        static TIME: std::sync::OnceLock<DateTime<Utc>> = std::sync::OnceLock::new();
        TIME.get_or_init(|| Utc::now())
    }
}

// Remove the DomainEvent implementation for TestEntity since it's not an event
// The TestEntity is a PersistentEntity, not a DomainEvent