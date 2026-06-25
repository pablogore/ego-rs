//! Runtime verification of persistence and actor lifecycle behavior.
//!
//! Exercises each changed behavior path through the public EntityRuntime surface.

use std::sync::{Arc, Mutex};
use std::time::Duration;

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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn handler() -> Arc<dyn persistent_entity::persistent_entity::PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>> {
    Arc::new(TestEntity::new())
}

fn ctx() -> CommandContext {
    CommandContext::new("counter".to_string())
}

// ---------------------------------------------------------------------------
// FailingEventStore: load() always returns an error (Fix 1 probe)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// AppendFailingStore: load() succeeds, append() always fails (Fix 4 probe)
// ---------------------------------------------------------------------------

struct AppendFailingStore {
    inner: InMemoryEventStore<TestEvent>,
}

impl AppendFailingStore {
    fn new() -> Self {
        Self { inner: InMemoryEventStore::new() }
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
        Err(PersistenceError::Internal("injected append failure".to_string()))
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

// ---------------------------------------------------------------------------
// Scenario 1 — Happy path: increment, state persists, active_count tracks
// ---------------------------------------------------------------------------

#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("=== Persistence and actor lifecycle verification ===\n");

    // --- Scenario 1: happy path ----------------------------------------------
    print!("[1] Happy path (increment + verify state): ");
    {
        let runtime = EntityRuntimeBuilder::<TestEvent>::new()
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build();

        let r = runtime.entity_ref("counter", "s1", handler());
        let res: Result<CommandResult<TestEvent, TestState>, EntityError> =
            r.send_command(TestCommand::Increment(10), ctx()).await;
        match res {
            Ok(CommandResult::Events { new_state, .. }) if new_state.value == 10 => {
                println!("PASS (state.value={})", new_state.value);
            }
            other => {
                println!("FAIL: {:?}", other);
                std::process::exit(1);
            }
        }
    }

    // --- Scenario 2: load error propagates to caller -------------------------
    print!("[2] event_store.load() error propagates to caller: ");
    {
        let event_store = Arc::new(Mutex::new(FailingEventStore));
        let snapshot_store = Arc::new(Mutex::new(InMemorySnapshotStore::new()));

        let runtime = EntityRuntimeBuilder::<TestEvent>::new()
            .with_event_store(event_store)
            .with_snapshot_store(snapshot_store)
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build();

        let r = runtime.entity_ref("counter", "s2", handler());
        // Give actor time to attempt recovery
        tokio::time::sleep(Duration::from_millis(50)).await;

        let res: Result<CommandResult<TestEvent, TestState>, EntityError> =
            r.send_command(TestCommand::Increment(1), ctx()).await;
        match res {
            Err(EntityError::EntityNotActive) | Err(EntityError::MailboxClosed) => {
                println!("PASS (got error, not silent success)");
            }
            Ok(_) => {
                println!("FAIL: expected error, got success — load error was swallowed");
                std::process::exit(1);
            }
            Err(other) => {
                println!("PASS (error: {:?})", other);
            }
        }
    }

    // --- Scenario 3: snapshot round-trip recovery ----------------------------
    print!("[3] Snapshot recovery with version-offset store: ");
    {
        // InMemoryEventStore with a version offset of 5 — simulates 5 events that
        // were already persisted and captured by the snapshot below.
        let event_store = Arc::new(Mutex::new(
            InMemoryEventStore::<TestEvent>::new().with_version_offset("counter-s3", 5),
        ));
        let snapshot_store = Arc::new(Mutex::new(InMemorySnapshotStore::new()));

        // Pre-seed a snapshot at version 5, value=42.
        {
            let mut snap = snapshot_store.lock().unwrap();
            snap.save_snapshot(
                "counter-s3",
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

        let r = runtime.entity_ref("counter", "s3", handler());

        // First command — actor should recover from snapshot (value=42) and increment
        let res: Result<CommandResult<TestEvent, TestState>, EntityError> =
            r.send_command(TestCommand::Increment(8), ctx()).await;
        match res {
            Ok(CommandResult::Events { new_state, .. }) if new_state.value == 50 => {
                println!("PASS (snapshot v5 value=42 + incr 8 = {})", new_state.value);
            }
            Ok(CommandResult::Events { new_state, .. }) => {
                println!(
                    "FAIL: expected value=50 after snapshot recovery, got {}",
                    new_state.value
                );
                std::process::exit(1);
            }
            other => {
                println!("FAIL: {:?}", other);
                std::process::exit(1);
            }
        }
    }

    // --- Scenario 4: persist failure → actor drains mailbox ------------------
    print!("[4] persist_events failure → subsequent commands get error, not hang: ");
    {
        let event_store = Arc::new(Mutex::new(AppendFailingStore::new()));
        let snapshot_store = Arc::new(Mutex::new(InMemorySnapshotStore::new()));

        let runtime = EntityRuntimeBuilder::<TestEvent>::new()
            .with_event_store(event_store)
            .with_snapshot_store(snapshot_store)
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build();

        let r = runtime.entity_ref("counter", "s4", handler());

        // First command — persist will fail, actor transitions to Failed
        let first: Result<CommandResult<TestEvent, TestState>, EntityError> =
            r.send_command(TestCommand::Increment(1), ctx()).await;

        // Second command — should get an error promptly (not hang indefinitely)
        let second: Result<CommandResult<TestEvent, TestState>, EntityError> =
            r.send_command(TestCommand::Increment(2), ctx()).await;

        let first_is_err = first.is_err();
        let second_is_err = second.is_err();

        if first_is_err && second_is_err {
            println!("PASS (first={:?}, second={:?})", first.unwrap_err(), second.unwrap_err());
        } else {
            println!(
                "FAIL: first_is_err={}, second_is_err={}",
                first_is_err, second_is_err
            );
            std::process::exit(1);
        }
    }

    // --- Scenario 5: active_count tracks correctly after passivation (SpawnGuard) ---
    print!("[5] active_count returns to 0 after entity passivates (SpawnGuard + registry): ");
    {
        let runtime = EntityRuntimeBuilder::<TestEvent>::new()
            .snapshot_strategy(Arc::new(NoSnapshot))
            .passivation_timeout(Duration::from_millis(100))
            .build();

        let r = runtime.entity_ref("counter", "s5", handler());
        let _: Result<CommandResult<TestEvent, TestState>, EntityError> =
            r.send_command(TestCommand::Increment(1), ctx()).await;

        let active_after_cmd = runtime.active_count();

        // Wait for passivation
        tokio::time::sleep(Duration::from_millis(400)).await;
        let active_after_passivation = runtime.active_count();
        let passivated_after = runtime.passivated_count();

        if active_after_cmd == 1 && active_after_passivation == 0 && passivated_after == 1 {
            println!(
                "PASS (active after cmd={}, after passivation={}, passivated={})",
                active_after_cmd, active_after_passivation, passivated_after
            );
        } else {
            println!(
                "FAIL: active_after_cmd={}, active_after_passivation={}, passivated={}",
                active_after_cmd, active_after_passivation, passivated_after
            );
            std::process::exit(1);
        }
    }

    println!("\n=== All scenarios PASS ===");
}
