//! Slice 3 (design.md AD-4): proves the real composition root — not a
//! parallel test harness — accepts `Profile::Production` with
//! `StoolapEventStore` + `StoolapSnapshotStore`, survives a drop and reopen
//! of the same file with state and version intact, and still refuses
//! volatile stores under the same profile.
//!
//! Local `Recovery*` domain types exist because `persistent_entity::testing`'s
//! `TestEvent::payload()` returns a process-global, instance-independent
//! `Value::Null` (fine for `InMemoryEventStore`, which never serializes) —
//! unusable for a real Stoolap round trip, which serializes
//! `DomainEvent::payload()` on every append.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tempfile::TempDir;

use ego_persistence_api::event::DomainEvent;
use ego_persistence_api::persistence::PersistenceError;
use ego_persistence_stoolap::{StoolapEventStore, StoolapSnapshotStore};

use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::profile::Profile;
use persistent_entity::snapshot::PeriodicSnapshotStrategy;
use persistent_entity::testing::{InMemoryEventStore, InMemorySnapshotStore};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct RecoveryEvent {
    event_type: String,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

impl RecoveryEvent {
    fn incremented(by: u64) -> Self {
        Self {
            event_type: "Incremented".to_string(),
            payload: json!({ "by": by }),
            occurred_at: Utc::now(),
        }
    }
}

impl DomainEvent for RecoveryEvent {
    fn aggregate_id(&self) -> &str {
        "unused"
    }
    fn event_type(&self) -> &str {
        &self.event_type
    }
    fn payload(&self) -> &serde_json::Value {
        &self.payload
    }
    fn occurred_at(&self) -> &DateTime<Utc> {
        &self.occurred_at
    }
}

fn deserialize_recovery_event(
    event_type: &str,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
) -> Result<RecoveryEvent, PersistenceError> {
    Ok(RecoveryEvent {
        event_type: event_type.to_string(),
        payload,
        occurred_at,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct RecoveryState {
    total: u64,
    version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum RecoveryCommand {
    Increment(u64),
    GetState,
}

#[derive(Debug)]
struct RecoveryEntity;

#[async_trait::async_trait]
impl PersistentEntity for RecoveryEntity {
    type Command = RecoveryCommand;
    type Event = RecoveryEvent;
    type State = RecoveryState;

    fn initial_state(&self) -> RecoveryState {
        RecoveryState {
            total: 0,
            version: 0,
        }
    }

    async fn handle_command(
        &self,
        command: &RecoveryCommand,
        _state: &RecoveryState,
        _context: &CommandContext,
    ) -> Result<Vec<RecoveryEvent>, EntityError> {
        match command {
            RecoveryCommand::Increment(by) => Ok(vec![RecoveryEvent::incremented(*by)]),
            RecoveryCommand::GetState => Ok(vec![]),
        }
    }

    async fn apply_event(
        &self,
        state: &RecoveryState,
        event: &RecoveryEvent,
    ) -> Result<RecoveryState, EntityError> {
        let by = event
            .payload
            .get("by")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| EntityError::Internal("event payload missing \"by\"".to_string()))?;
        Ok(RecoveryState {
            total: state.total + by,
            version: state.version + 1,
        })
    }

    async fn apply_events(
        &self,
        state: &RecoveryState,
        events: &[RecoveryEvent],
    ) -> Result<RecoveryState, EntityError> {
        let mut s = state.clone();
        for event in events {
            s = self.apply_event(&s, event).await?;
        }
        Ok(s)
    }
}

async fn open_stoolap_event_store(
    path: &std::path::Path,
) -> StoolapEventStore<
    RecoveryEvent,
    fn(&str, serde_json::Value, DateTime<Utc>) -> Result<RecoveryEvent, PersistenceError>,
> {
    let deserialize: fn(
        &str,
        serde_json::Value,
        DateTime<Utc>,
    ) -> Result<RecoveryEvent, PersistenceError> = deserialize_recovery_event;
    StoolapEventStore::open(path, deserialize)
        .await
        .expect("StoolapEventStore must open under the production DSN (sync=full)")
}

/// AD-4: inner scope writes across the snapshot threshold and drops; the
/// outer scope reopens the identical path and recovers the same state and
/// version through the real composition root — never a parallel harness.
#[tokio::test]
async fn production_composition_survives_drop_and_reopen_with_recovered_state() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    let written_state = {
        let event_store = open_stoolap_event_store(path).await;
        let snapshot_store = StoolapSnapshotStore::open(path)
            .expect("StoolapSnapshotStore must open under the production DSN (sync=full)");

        let runtime = EntityRuntimeBuilder::<RecoveryEvent>::new()
            .profile(Profile::Production)
            .with_event_store(Arc::new(event_store))
            .with_snapshot_store(Arc::new(Mutex::new(snapshot_store)))
            .snapshot_strategy(Arc::new(PeriodicSnapshotStrategy::new(3)))
            .try_build()
            .expect(
                "Profile::Production + StoolapEventStore + StoolapSnapshotStore + zero \
                 PostgreSQL must satisfy the durability gate",
            );

        let entity_ref = runtime
            .entity_ref::<RecoveryCommand, RecoveryState>(
                "counter",
                "restart-1",
                Arc::new(RecoveryEntity),
            )
            .unwrap();

        let mut state = None;
        for by in 1..=5u64 {
            let result: CommandResult<RecoveryEvent, RecoveryState> = entity_ref
                .send_command(
                    RecoveryCommand::Increment(by),
                    CommandContext::new("counter".to_string()),
                )
                .await
                .expect("command should succeed");
            match result {
                CommandResult::Events { new_state, .. } => state = Some(new_state),
                other => panic!("expected CommandResult::Events, got {other:?}"),
            }
        }
        state.expect("at least one command was sent")
        // `runtime`, its stores, and the underlying Stoolap engine drop here.
    };

    let event_store = open_stoolap_event_store(path).await;
    let snapshot_store = StoolapSnapshotStore::open(path)
        .expect("StoolapSnapshotStore must open under the production DSN (sync=full)");

    let runtime = EntityRuntimeBuilder::<RecoveryEvent>::new()
        .profile(Profile::Production)
        .with_event_store(Arc::new(event_store))
        .with_snapshot_store(Arc::new(Mutex::new(snapshot_store)))
        .snapshot_strategy(Arc::new(PeriodicSnapshotStrategy::new(3)))
        .try_build()
        .expect(
            "reopening the identical path under the same composition must still satisfy the \
             durability gate",
        );
    let entity_ref = runtime
        .entity_ref::<RecoveryCommand, RecoveryState>(
            "counter",
            "restart-1",
            Arc::new(RecoveryEntity),
        )
        .unwrap();

    let result: CommandResult<RecoveryEvent, RecoveryState> = entity_ref
        .send_command(
            RecoveryCommand::GetState,
            CommandContext::new("counter".to_string()),
        )
        .await
        .expect("GetState should succeed against a recovered entity");

    match result {
        CommandResult::NoEvents { state } => {
            assert_eq!(
                state.total, written_state.total,
                "recovered total must match the state written before the drop"
            );
            assert_eq!(
                state.version, written_state.version,
                "recovered version must match the state written before the drop"
            );
        }
        other => panic!("expected CommandResult::NoEvents, got {other:?}"),
    }
}

/// AD-4's negative control: `Profile::Production` refuses explicitly wired
/// volatile stores exactly as `EntityRuntimeBuilder`'s own unit tests
/// establish, even though a durable Stoolap-backed alternative exists.
#[test]
fn production_refuses_volatile_stores_even_when_stoolap_is_available() {
    let result = EntityRuntimeBuilder::<RecoveryEvent>::new()
        .profile(Profile::Production)
        .with_event_store(Arc::new(InMemoryEventStore::new()))
        .with_snapshot_store(Arc::new(Mutex::new(InMemorySnapshotStore::new())))
        .try_build();

    let err = match result {
        Err(err) => err,
        Ok(_) => panic!(
            "Profile::Production must refuse an explicitly wired InMemoryEventStore even when \
             a durable Stoolap-backed alternative exists"
        ),
    };
    assert!(
        err.to_string().contains("event store"),
        "must name the non-durable capability: {err}"
    );
}
