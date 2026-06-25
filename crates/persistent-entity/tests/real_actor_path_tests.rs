/// Integration tests for the real `TokioEntityRef` → `EntityActor` path.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ego_domain::persistence::{PersistenceError, Snapshot};
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::CommandResult;
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::test_entity::TestEntity;
use persistent_entity::testing::{TestCommand, TestEvent, TestState};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn handler(
) -> Arc<dyn persistent_entity::persistent_entity::PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>>
{
    Arc::new(TestEntity::new())
}

fn ctx() -> CommandContext {
    CommandContext::new("counter".to_string())
}

// ---------------------------------------------------------------------------
// Failing snapshot store — causes recovery to return an error
// ---------------------------------------------------------------------------

struct FailingSnapshotStore;

impl Snapshot for FailingSnapshotStore {
    fn save_snapshot(
        &mut self,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _version: i64,
        _payload: serde_json::Value,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn load_snapshot(
        &self,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Option<(i64, serde_json::Value)>, PersistenceError> {
        Err(PersistenceError::Internal(
            "injected recovery failure".to_string(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Basic command execution inside a current_thread runtime
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn test_real_actor_command_reply() {
    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .build();
    let entity_ref = runtime.entity_ref::<TestCommand, TestState>("counter", "c1", handler());

    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        entity_ref.send_command(TestCommand::Increment(5), ctx()).await;

    assert!(result.is_ok(), "command should succeed: {:?}", result.err());
    match result.unwrap() {
        CommandResult::Events { new_state, events } => {
            assert_eq!(new_state.value, 5);
            assert_eq!(events.len(), 1);
        }
        other => panic!("expected CommandResult::Events, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Recovery from pre-seeded events
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn test_recovery_replays_seeded_events() {
    use ego_domain::persistence::{EventStore, StoredEvent};
    use persistent_entity::persistence::InMemoryEventStore;

    let event_store = Arc::new(Mutex::new(InMemoryEventStore::<TestEvent>::new()));

    // EntityTriple::aggregate_id() returns "{entity_type}-{entity_id}".
    // Pre-seed under the same key the actor will use for recovery.
    let aggregate_key = "counter-c-recovery";
    let events: Vec<StoredEvent<TestEvent>> = (1u64..=3)
        .map(|v| StoredEvent::without_correlation(TestEvent::Incremented(v)))
        .collect();
    {
        let mut store = event_store.lock().unwrap();
        store
            .append(aggregate_key, None, 0, events)
            .expect("pre-seed must succeed");
    }

    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_event_store(event_store)
        .build();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("counter", "c-recovery", handler());

    // GetState emits no events — the actor returns the recovered state as-is.
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        entity_ref.send_command(TestCommand::GetState, ctx()).await;

    assert!(result.is_ok(), "GetState should succeed: {:?}", result.err());
    match result.unwrap() {
        CommandResult::NoEvents { state } => {
            // 3 events seeded, each increments state.version by 1.
            assert_eq!(
                state.version, 3,
                "recovered state should reflect 3 replayed events"
            );
            // values are 1+2+3 = 6
            assert_eq!(state.value, 6, "recovered value should be 1+2+3=6");
        }
        other => panic!("expected CommandResult::NoEvents, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Passivation: actor exits after timeout, registry updated
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_passivation_updates_registry() {
    let runtime = Arc::new(
        EntityRuntimeBuilder::<TestEvent>::new()
            // Very short timeout so passivation fires quickly.
            .passivation_timeout(Duration::from_millis(10))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build(),
    );

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("counter", "p1", handler());

    // Send a command before passivation fires — must be replied.
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        entity_ref.send_command(TestCommand::Increment(1), ctx()).await;
    assert!(result.is_ok(), "pre-passivation command should succeed");

    // Wait for the actor to passivate.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if runtime.passivated_count() >= 1 {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("actor did not passivate within 2 s");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    assert_eq!(
        runtime.passivated_count(),
        1,
        "registry must record one passivated entity"
    );
}

// ---------------------------------------------------------------------------
// Recovery failure: actor must not hang, active_count stays zero
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_recovery_failure_returns_error() {
    let failing_snapshot_store = Arc::new(Mutex::new(FailingSnapshotStore));

    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_snapshot_store(failing_snapshot_store)
        .build();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("counter", "fail-1", handler());

    // The actor fails recovery synchronously inside its spawned task.
    // send_command must get a reply (Err) rather than hanging forever.
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        tokio::time::timeout(
            Duration::from_secs(2),
            entity_ref.send_command(TestCommand::Increment(1), ctx()),
        )
        .await
        .expect("send_command must complete within 2 s (no hang)");

    assert!(
        result.is_err(),
        "command after recovery failure must return Err"
    );

    // The actor must have exited without marking itself active.
    // Give the task a moment to finalise.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        runtime.active_count(),
        0,
        "failed actor must not appear as active"
    );
}
