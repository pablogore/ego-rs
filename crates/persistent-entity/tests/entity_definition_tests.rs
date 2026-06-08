use persistent_entity::command_context::CommandContext;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::PersistentEntity;
use persistent_entity::test_entity::TestEntity;
use persistent_entity::testing::{TestCommand, TestEvent, TestState};

#[tokio::test]
async fn test_counter_handler_produces_events() {
    let entity = TestEntity::new();
    let state = TestState::new(0);
    let ctx = CommandContext::new("test".to_string());

    let events = entity
        .handle_command(&TestCommand::Increment(1), &state, &ctx)
        .await
        .expect("Increment should succeed");
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], TestEvent::Incremented(1)));
}

#[tokio::test]
async fn test_counter_decrement_below_zero_returns_error() {
    let entity = TestEntity::new();
    let state = TestState::new(0);
    let ctx = CommandContext::new("test".to_string());

    let result = entity
        .handle_command(&TestCommand::Decrement(10), &state, &ctx)
        .await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), EntityError::Internal(_)));
}

#[tokio::test]
async fn test_counter_getstate_produces_zero_events() {
    let entity = TestEntity::new();
    let state = TestState::new(42);
    let ctx = CommandContext::new("test".to_string());

    let events = entity
        .handle_command(&TestCommand::GetState, &state, &ctx)
        .await
        .expect("GetState should succeed");
    assert!(events.is_empty());
}

#[tokio::test]
async fn test_counter_applier_evolves_state() {
    let entity = TestEntity::new();
    let state = TestState::new(0);

    let new_state = entity
        .apply_events(&state, &[TestEvent::Incremented(1)])
        .await
        .expect("apply should succeed");
    assert_eq!(new_state.value, 1);

    let new_state = entity
        .apply_events(
            &new_state,
            &[
                TestEvent::Incremented(3),
                TestEvent::Decremented(1),
            ],
        )
        .await
        .expect("apply should succeed");
    assert_eq!(new_state.value, 3);
}

#[tokio::test]
async fn test_counter_initial_state_is_zero() {
    let entity = TestEntity::new();
    let state = entity.initial_state();
    assert_eq!(state.value, 0);
}
