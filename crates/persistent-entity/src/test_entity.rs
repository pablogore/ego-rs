//! A test-only [`PersistentEntity`] implementation for use in tests and examples.
//!
//! Provides [`TestEntity`] with [`TestCommand`], [`TestEvent`], and [`TestState`]
//! for exercising the entity lifecycle without real domain logic.

use async_trait::async_trait;
use crate::persistent_entity::PersistentEntity;
use crate::command_context::CommandContext;
use crate::error::EntityError;
use crate::testing::{TestCommand, TestEvent, TestState};

/// A test-only entity that supports increment, decrement, and get-state commands.
///
/// Used in integration tests and examples to validate the persistent entity
/// lifecycle without requiring a real domain implementation.
#[derive(Debug, Clone)]
pub struct TestEntity;

impl TestEntity {
    /// Creates a new [`TestEntity`].
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