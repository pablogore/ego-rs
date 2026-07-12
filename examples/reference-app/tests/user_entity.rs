//! CORE-018 Phase 4 — `User` `PersistentEntity` (AD-6).
//!
//! Satisfies reference-service spec "Registering a user".

use persistent_entity::command_context::CommandContext;
use persistent_entity::persistent_entity::PersistentEntity;
use reference_app::domain::user::{UserCommand, UserEntity, UserState};

fn ctx() -> CommandContext {
    CommandContext::new("user".to_string())
}

#[tokio::test]
async fn register_on_unregistered_produces_exactly_one_user_registered_event() {
    let entity = UserEntity::new();
    let state = entity.initial_state();
    assert_eq!(state, UserState::Unregistered);

    let cmd = UserCommand::Register {
        user_id: "user-1".to_string(),
        email: "user@example.com".to_string(),
        tenant_id: "tenant-a".to_string(),
    };

    let events = entity
        .handle_command(&cmd, &state, &ctx())
        .await
        .expect("register should succeed");

    assert_eq!(events.len(), 1, "exactly one UserRegistered event expected");
}

#[tokio::test]
async fn apply_event_transitions_to_registered_with_email_and_tenant() {
    let entity = UserEntity::new();
    let state = entity.initial_state();
    let cmd = UserCommand::Register {
        user_id: "user-1".to_string(),
        email: "user@example.com".to_string(),
        tenant_id: "tenant-a".to_string(),
    };
    let events = entity
        .handle_command(&cmd, &state, &ctx())
        .await
        .expect("register should succeed");

    let new_state = entity
        .apply_event(&state, &events[0])
        .await
        .expect("apply_event should succeed");

    assert_eq!(
        new_state,
        UserState::Registered {
            email: "user@example.com".to_string(),
            tenant_id: "tenant-a".to_string(),
        }
    );
}

/// CORE-018 Phase 7 groundwork: a real (not test-only) validation rule that
/// gives `register_user_partial_failure.rs` a deterministic trigger for a
/// `User`-write failure after a `TenantOrganization`-write success (AD-5).
#[tokio::test]
async fn register_with_empty_email_is_rejected() {
    let entity = UserEntity::new();
    let state = entity.initial_state();
    let cmd = UserCommand::Register {
        user_id: "user-1".to_string(),
        email: String::new(),
        tenant_id: "tenant-a".to_string(),
    };

    let result = entity.handle_command(&cmd, &state, &ctx()).await;

    assert!(result.is_err(), "empty email must be rejected");
}
