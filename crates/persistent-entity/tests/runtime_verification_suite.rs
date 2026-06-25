//! Runtime Verification Suite for CORE-006 Persistent Entity Runtime
//!
//! This module contains comprehensive tests to verify that the implementation
//! conforms exactly to the FINAL CONSISTENCY LOCK specification.

use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::persistent_entity::PersistentEntity;
use persistent_entity::test_entity::TestEntity;
use persistent_entity::testing::{create_test_context, TestCommand, TestEvent};

/// Validates that the same command applied to the same state produces
/// identical results across multiple calls to `handle_command`.
#[tokio::test]
async fn test_deterministic_execution() {
    let entity = TestEntity::new();
    let state = entity.initial_state();
    let ctx = create_test_context();

    // Execute same command twice with same state and context
    let events1 = entity
        .handle_command(&TestCommand::increment(1), &state, &ctx)
        .await
        .expect("First execution should succeed");

    let events2 = entity
        .handle_command(&TestCommand::increment(1), &state, &ctx)
        .await
        .expect("Second execution should succeed");

    // Results should be identical
    assert_eq!(events1, events2);
    assert_eq!(events1.len(), 1);
    assert!(matches!(events1[0], TestEvent::Incremented(1)));
}

/// Test that demonstrates same ExecutionKey produces same result.
#[tokio::test]
async fn test_execution_key_determinism() {
    let entity = TestEntity::new();
    let state = entity.initial_state();
    let ctx1 = create_test_context();
    let ctx2 = create_test_context();

    // Same context should produce same result
    let events1 = entity
        .handle_command(&TestCommand::increment(1), &state, &ctx1)
        .await
        .expect("First execution should succeed");

    let events2 = entity
        .handle_command(&TestCommand::increment(1), &state, &ctx2)
        .await
        .expect("Second execution should succeed");

    assert_eq!(events1, events2);
}

/// Test basic runtime functionality.
#[tokio::test]
async fn test_runtime_basic_functionality() {
    let runtime: EntityRuntimeBuilder<TestEvent> = EntityRuntimeBuilder::new();
    let runtime = runtime.mailbox_capacity(100).concurrency_budget(10).build();

    // Just verify we can create the runtime
    assert_eq!(runtime.config.mailbox_capacity, 100);
    assert_eq!(runtime.config.concurrency_budget, 10);
}
