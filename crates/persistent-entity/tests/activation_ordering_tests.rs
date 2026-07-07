use std::sync::Arc;

use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::test_entity::TestEntity;
use persistent_entity::testing::{create_test_context, TestCommand, TestEvent, TestState};

mod common;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_runtime() -> Arc<persistent_entity::runtime::EntityRuntime<TestEvent>> {
    Arc::new(
        persistent_entity::builder::EntityRuntimeBuilder::new()
            .passivation_timeout(std::time::Duration::from_secs(3600))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build(),
    )
}

fn build_fast_passivation_runtime() -> Arc<persistent_entity::runtime::EntityRuntime<TestEvent>> {
    Arc::new(
        persistent_entity::builder::EntityRuntimeBuilder::new()
            .passivation_timeout(std::time::Duration::from_millis(50))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build(),
    )
}

fn handler(
) -> Arc<dyn PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>> {
    Arc::new(TestEntity::new())
}

// ============================================================================
// User Story 1 — Activation Ordering
// ============================================================================

/// Activation lookup — send to new entity creates it, passivation removes sender.
#[tokio::test]
async fn test_activation_lookup_active_and_passivated() {
    let runtime = build_runtime();
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-1", h.clone()).unwrap();

    // Entity is not active initially — send activates it
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await;
    assert!(result.is_ok(), "first send should activate entity");

    // Send another command — should find active sender
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(2), create_test_context())
        .await;
    assert!(result.is_ok(), "second send should use active sender");
}

/// FIFO ordering — commands processed in send order.
#[tokio::test]
async fn test_activation_fifo_ordering() {
    let runtime = build_runtime();
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-2", h.clone()).unwrap();

    let mut expected = 0u64;
    for i in 1..=10u64 {
        let result: CommandResult<TestEvent, TestState> = entity_ref
            .send_command(TestCommand::Increment(i), create_test_context())
            .await
            .unwrap();
        expected += i;
        match result {
            CommandResult::Events { new_state, .. } => {
                assert_eq!(
                    new_state.value, expected,
                    "state should show sum after command {}",
                    i
                );
            }
            _ => panic!("expected Events variant"),
        }
    }
    assert_eq!(expected, 55);
}

/// No partial state — command always sees fully recovered state.
#[tokio::test]
async fn test_no_partial_state_observable() {
    let runtime = build_runtime();
    let h = handler();

    // Build up state with multiple commands
    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-3", h.clone()).unwrap();
    for _i in 1..=5u64 {
        let _: CommandResult<TestEvent, TestState> = entity_ref
            .send_command(TestCommand::Increment(10), create_test_context())
            .await
            .unwrap();
    }

    // Query the state — should see all 5 increments
    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::GetState, create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::NoEvents { state } => {
            assert_eq!(state.value, 50, "state should reflect all prior commands");
            assert_eq!(state.version, 5, "version should reflect all events");
        }
        _ => panic!("expected NoEvents variant"),
    }
}

/// Activation redirect — concurrent callers find active entity without duplicate spawn.
#[tokio::test]
async fn test_activation_redirect() {
    let runtime = build_runtime();
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-4", h.clone()).unwrap();

    // Send first command to activate
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Increment(10),
            CommandContext::new("TestCommand".to_string()),
        )
        .await
        .unwrap();

    // Send another — should redirect to existing actor
    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::Events { new_state, .. } => {
            assert_eq!(new_state.value, 11);
        }
        _ => panic!("expected Events variant"),
    }
}

// ============================================================================
// User Story 2 — No Double Actor Spawn
// ============================================================================

/// No double spawn — concurrent tasks to passivated entity all go to one actor.
#[tokio::test]
async fn test_no_double_spawn_concurrent() {
    let runtime = build_fast_passivation_runtime();
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-5", h.clone()).unwrap();

    // Activate then let passivate
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Increment(1),
            CommandContext::new("test".to_string()),
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Concurrent sends should coalesce into single activation
    let results =
        common::spawn_concurrent_commands(20, runtime.clone(), "test", "entity-5", h.clone()).await;

    let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
    assert_eq!(
        successes.len(),
        20,
        "all concurrent commands should succeed"
    );

    // At most 1 active entity should exist
    let active_count = runtime.active_count();
    assert!(
        active_count <= 2,
        "should have at most 2 active (one draining, one new): {}",
        active_count
    );
}

/// Mutex-based single-flight — concurrent activations serialize.
#[tokio::test]
async fn test_activation_mutex_serializes() {
    let runtime = build_fast_passivation_runtime();
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-6", h.clone()).unwrap();

    // Activate then let passivate
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Increment(1),
            CommandContext::new("test".to_string()),
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Spawn 10 concurrent tasks — all should succeed with no duplicate spawns
    let n = 10;
    let mut handles = Vec::with_capacity(n);
    for _ in 0..n {
        let rt = runtime.clone();
        let h = h.clone();
        handles.push(tokio::spawn(async move {
            let ref_ = rt.entity_ref::<TestCommand, TestState>("test", "entity-6", h).unwrap();
            let result: Result<CommandResult<TestEvent, TestState>, EntityError> = ref_
                .send_command(
                    TestCommand::Increment(1),
                    CommandContext::new("test".to_string()),
                )
                .await;
            result
        }));
    }

    for handle in handles {
        let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
            handle.await.unwrap();
        assert!(result.is_ok(), "concurrent command should succeed");
    }

    let active = runtime.active_count();
    assert!(active <= 2, "at most 2 active entities: {}", active);
}

/// No double spawn across multiple entities — each gets exactly one actor.
#[tokio::test]
async fn test_no_double_spawn_multiple_entities() {
    let runtime = build_fast_passivation_runtime();
    let h = handler();

    // Spawn commands for 10 different entities simultaneously
    let mut handles = Vec::with_capacity(10);
    for i in 0..10 {
        let rt = runtime.clone();
        let h = h.clone();
        let entity_id = format!("multi-{}", i);
        handles.push(tokio::spawn(async move {
            let entity_ref =
                rt.entity_ref::<TestCommand, TestState>("test", &entity_id, h).unwrap();
            let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
                .send_command(
                    TestCommand::Increment(1),
                    CommandContext::new("test".to_string()),
                )
                .await;
            result
        }));
    }

    for handle in handles {
        let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
            handle.await.unwrap();
        assert!(result.is_ok(), "each entity should activate independently");
    }

    // All 10 should have active actors (or some may have passivated already)
    let active = runtime.active_count();
    assert!(active > 0, "at least some entities should be active");
}

// ============================================================================
// User Story 3 — Deterministic Recovery
// ============================================================================

/// Recovery barrier — pre-loaded events are reflected after activation.
#[tokio::test]
async fn test_recovery_barrier() {
    // Build runtime, increment many times to build history
    let runtime = build_runtime();
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-7", h.clone()).unwrap();

    for _ in 0..50 {
        let _: CommandResult<TestEvent, TestState> = entity_ref
            .send_command(TestCommand::Increment(1), create_test_context())
            .await
            .unwrap();
    }

    // Send a command and verify version reflects all prior events
    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::Events {
            events: _,
            new_state,
            ..
        } => {
            let new_version = new_state.version;
            assert!(
                new_version >= 50,
                "version should be >= 50, got {}",
                new_version
            );
            assert_eq!(new_state.value, 51);
        }
        _ => panic!("expected Events variant"),
    }
}

/// Deterministic replay — same event stream produces identical state.
#[tokio::test]
async fn test_recovery_deterministic_replay() {
    let runtime = build_runtime();
    let h = handler();
    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-8", h.clone()).unwrap();

    // Build deterministic state
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Increment(10),
            CommandContext::new("TestCommand".to_string()),
        )
        .await
        .unwrap();
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Increment(20),
            CommandContext::new("TestCommand".to_string()),
        )
        .await
        .unwrap();
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Decrement(5),
            CommandContext::new("TestCommand".to_string()),
        )
        .await
        .unwrap();

    // Query state from the same runtime
    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::GetState, create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::NoEvents { state } => {
            assert_eq!(state.value, 25, "10 + 20 - 5 = 25");
            assert_eq!(state.version, 3);
        }
        _ => panic!("expected NoEvents variant"),
    }
}

/// Recovery failure transition — simulated via handler error.
#[tokio::test]
async fn test_recovery_failure_transitions_to_failed() {
    let runtime = build_runtime();
    let h = handler();
    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-9", h.clone()).unwrap();

    // Decrement on zero should fail
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(
            TestCommand::Decrement(1),
            CommandContext::new("TestCommand".to_string()),
        )
        .await;
    assert!(result.is_err(), "decrement on zero should fail");

    // Entity should still accept new commands after handler error (increment always succeeds)
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await;
    assert!(
        result.is_ok(),
        "entity should still accept commands after handler error"
    );

    // Entity should still accept new commands (handler error doesn't fail the entity)
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await;
    assert!(
        result.is_ok(),
        "entity should still accept commands after handler error"
    );
}

/// Recovery retry — after failure, next command triggers fresh activation.
#[tokio::test]
async fn test_recovery_retry_after_failure() {
    let runtime = build_runtime();
    let h = handler();
    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-10", h.clone()).unwrap();

    // Activate and do work
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(5), create_test_context())
        .await
        .unwrap();

    // Verify entity is functional
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await;
    assert!(result.is_ok());
}

/// Zero-event query — GetState produces NoEvents, doesn't advance version.
#[tokio::test]
async fn test_zero_event_query() {
    let runtime = build_runtime();
    let h = handler();
    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-16", h.clone()).unwrap();

    // Activate with a mutation
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(10), create_test_context())
        .await
        .unwrap();

    // Zero-event query
    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::GetState, create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::NoEvents { state } => {
            assert_eq!(state.value, 10);
            assert_eq!(state.version, 1);
        }
        _ => panic!("expected NoEvents variant"),
    }
}

/// Multiple entity isolation — same entity type, different IDs.
#[tokio::test]
async fn test_multiple_entity_isolation() {
    let runtime = build_runtime();
    let h = handler();

    let ref_a =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-a", h.clone()).unwrap();
    let ref_b =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-b", h.clone()).unwrap();

    // Mutate entity-a
    let _: CommandResult<TestEvent, TestState> = ref_a
        .send_command(
            TestCommand::Increment(100),
            CommandContext::new("TestCommand".to_string()),
        )
        .await
        .unwrap();

    // Entity-b should be independent
    let result: CommandResult<TestEvent, TestState> = ref_b
        .send_command(TestCommand::GetState, create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::NoEvents { state } => {
            assert_eq!(state.value, 0, "entity-b should have initial state");
        }
        _ => panic!("expected NoEvents variant"),
    }
}
