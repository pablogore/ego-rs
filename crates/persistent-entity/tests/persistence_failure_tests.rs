use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use ego_domain::persistence::{EventStore, PersistenceError, Snapshot, StoredEvent};
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::CommandResult;
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::test_entity::TestEntity;
use persistent_entity::testing::{
    InMemoryEventStore, InMemorySnapshotStore, TestCommand, TestEvent, TestState,
};

fn handler(
) -> Arc<dyn persistent_entity::persistent_entity::PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>>
{
    Arc::new(TestEntity::new())
}

fn ctx() -> CommandContext {
    CommandContext::new("counter".to_string())
}

struct FailingEventStore;

impl EventStore<TestEvent> for FailingEventStore {
    fn append(
        &mut self,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _expected_version: i64,
        _events: Vec<StoredEvent<TestEvent>>,
    ) -> Result<i64, PersistenceError> {
        Ok(0)
    }

    fn load(
        &self,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<TestEvent>>, PersistenceError> {
        Err(PersistenceError::Internal("injected load failure".to_string()))
    }

    fn list_aggregate_ids(
        &self,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<String>, PersistenceError> {
        Ok(Vec::new())
    }
}

struct AppendFailingStore {
    inner: InMemoryEventStore<TestEvent>,
}

impl AppendFailingStore {
    fn new() -> Self {
        Self {
            inner: InMemoryEventStore::new(),
        }
    }
}

impl EventStore<TestEvent> for AppendFailingStore {
    fn append(
        &mut self,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _expected_version: i64,
        _events: Vec<StoredEvent<TestEvent>>,
    ) -> Result<i64, PersistenceError> {
        Err(PersistenceError::Internal(
            "injected append failure".to_string(),
        ))
    }

    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<TestEvent>>, PersistenceError> {
        self.inner.load(aggregate_id, tenant_id)
    }

    fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<String>, PersistenceError> {
        self.inner.list_aggregate_ids(tenant_id)
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_load_error_propagates_to_caller() {
    let event_store = Arc::new(Mutex::new(FailingEventStore));
    let snapshot_store = Arc::new(Mutex::new(InMemorySnapshotStore::new()));

    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .with_event_store(event_store)
        .with_snapshot_store(snapshot_store)
        .snapshot_strategy(Arc::new(NoSnapshot))
        .build();

    let r = runtime.entity_ref("counter", "load-fail-1", handler()).unwrap();

    let res: Result<CommandResult<TestEvent, TestState>, EntityError> =
        r.send_command(TestCommand::Increment(1), ctx()).await;

    assert!(
        res.is_err(),
        "command after load failure must return Err, got {:?}",
        res
    );
}

#[tokio::test(flavor = "current_thread")]
async fn test_snapshot_recovery_with_version_offset() {
    let event_store = Arc::new(Mutex::new(
        InMemoryEventStore::<TestEvent>::new().with_version_offset("counter-snap-1", 5),
    ));
    let snapshot_store = Arc::new(Mutex::new(InMemorySnapshotStore::new()));

    {
        let mut snap = snapshot_store.lock();
        snap.save_snapshot(
            "counter-snap-1",
            None,
            5,
            serde_json::json!({ "value": 42, "version": 5 }),
        )
        .unwrap();
    }

    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .with_event_store(event_store)
        .with_snapshot_store(snapshot_store)
        .snapshot_strategy(Arc::new(NoSnapshot))
        .build();

    let r = runtime.entity_ref("counter", "snap-1", handler()).unwrap();

    let res: Result<CommandResult<TestEvent, TestState>, EntityError> =
        r.send_command(TestCommand::Increment(8), ctx()).await;

    match res {
        Ok(CommandResult::Events { new_state, .. }) => {
            assert_eq!(
                new_state.value, 50,
                "snapshot value=42 + increment 8 must equal 50, got {}",
                new_state.value
            );
        }
        other => panic!("expected Events result, got {:?}", other),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_persist_failure_drains_mailbox() {
    let event_store = Arc::new(Mutex::new(AppendFailingStore::new()));
    let snapshot_store = Arc::new(Mutex::new(InMemorySnapshotStore::new()));

    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .with_event_store(event_store)
        .with_snapshot_store(snapshot_store)
        .snapshot_strategy(Arc::new(NoSnapshot))
        .build();

    let r = runtime.entity_ref("counter", "persist-fail-1", handler()).unwrap();

    let first: Result<CommandResult<TestEvent, TestState>, EntityError> =
        r.send_command(TestCommand::Increment(1), ctx()).await;

    let second: Result<CommandResult<TestEvent, TestState>, EntityError> =
        r.send_command(TestCommand::Increment(2), ctx()).await;

    assert!(
        first.is_err(),
        "first command after persist failure must return Err"
    );
    assert!(
        second.is_err(),
        "second command to a failed actor must return Err, not hang"
    );
}

#[tokio::test]
async fn test_active_count_after_passivation() {
    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .snapshot_strategy(Arc::new(NoSnapshot))
        .passivation_timeout(Duration::from_millis(50))
        .build();

    let r = runtime.entity_ref("counter", "passive-1", handler()).unwrap();
    let _: Result<CommandResult<TestEvent, TestState>, EntityError> =
        r.send_command(TestCommand::Increment(1), ctx()).await;

    assert_eq!(runtime.active_count(), 1, "entity must be active after a command");

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if runtime.passivated_count() >= 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "entity did not passivate within 3 s"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(runtime.active_count(), 0, "active_count must be 0 after passivation");
    assert_eq!(runtime.passivated_count(), 1, "passivated_count must be 1");
}

#[tokio::test]
async fn test_reactivation_after_passivation() {
    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .snapshot_strategy(Arc::new(NoSnapshot))
        .passivation_timeout(Duration::from_millis(30))
        .build();

    let r1 = runtime.entity_ref("counter", "reactivate-1", handler()).unwrap();
    let first: Result<CommandResult<TestEvent, TestState>, EntityError> =
        r1.send_command(TestCommand::Increment(10), ctx()).await;
    assert!(first.is_ok(), "first command must succeed");

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if runtime.passivated_count() >= 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "entity did not passivate within 3 s"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let r2 = runtime.entity_ref("counter", "reactivate-1", handler()).unwrap();
    let second: Result<CommandResult<TestEvent, TestState>, EntityError> =
        r2.send_command(TestCommand::Increment(5), ctx()).await;

    match second {
        Ok(CommandResult::Events { new_state, .. }) => {
            assert_eq!(
                new_state.value, 15,
                "recovered state (10) + new increment (5) must equal 15, got {}",
                new_state.value
            );
        }
        other => panic!("expected Events result after re-activation, got {:?}", other),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_multiple_entity_ids_active_simultaneously() {
    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .snapshot_strategy(Arc::new(NoSnapshot))
        .passivation_timeout(Duration::from_secs(3600))
        .build();

    let r1 = runtime.entity_ref("counter", "multi-reg-1", handler()).unwrap();
    let r2 = runtime.entity_ref("counter", "multi-reg-2", handler()).unwrap();

    let _: Result<CommandResult<TestEvent, TestState>, EntityError> =
        r1.send_command(TestCommand::Increment(1), ctx()).await;
    let _: Result<CommandResult<TestEvent, TestState>, EntityError> =
        r2.send_command(TestCommand::Increment(2), ctx()).await;

    assert_eq!(
        runtime.active_count(),
        2,
        "both entity IDs must appear in the registry independently"
    );
}
