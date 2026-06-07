use async_trait::async_trait;
use crate::persistent_entity::{CommandResult, PersistentEntity};
use crate::command_context::CommandContext;
use crate::error::EntityError;
use crate::testing::{TestCommand, TestEvent, TestState};

#[derive(Debug, Clone)]
pub struct TestEntity;

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
        match command.command_type.as_str() {
            "Increment" => {
                let value = state.value + command.data.parse::<u64>().unwrap();
                let new_state = TestState {
                    value,
                    version: state.version + 1,
                };
                Ok(vec![TestEvent {
                    event_type: "Incremented".to_string(),
                    data: format!("incremented by {}", command.data),
                }])
            }
            "GetState" => {
                Ok(vec![])
            }
            _ => panic!("Unknown command type"),
        }
    }

    async fn apply_events(
        &self,
        state: &Self::State,
        events: &[Self::Event],
    ) -> Result<Self::State, EntityError> {
        let mut new_state = state.clone();
        for event in events {
            if event.event_type == "Incremented" {
                // In a real implementation, we'd parse the data to get the increment value
                new_state.value += 1; // Placeholder
                new_state.version += 1;
            }
        }
        Ok(new_state)
    }
}