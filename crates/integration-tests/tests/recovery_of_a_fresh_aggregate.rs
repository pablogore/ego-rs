//! Recovering an aggregate that has never been persisted, through the facade the
//! actor uses, against both implementations of the event store.
//!
//! An aggregate with no events yet is the ordinary state of every entity the first
//! time it is addressed. Whether the store reports that as an absent stream or as
//! an empty one is an implementation choice; whether *recovery* treats it as a
//! failure is not.
//!
//! This file exists because the two stores answered differently and only the
//! forgiving one was ever wired into a recovery test.

use chrono::{DateTime, Utc};
use ego_domain::event::DomainEvent;
use ego_domain::persistence::{EventStore, PersistenceError};
use ego_persistence::postgres::migrations;
use ego_persistence::PostgreSQLEventStore;
use persistent_entity::persistence::{
    InMemoryEventStore, InMemorySnapshotStore, PersistenceFacade,
};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tokio::sync::Mutex as AsyncMutex;

const POSTGRES_IMAGE_TAG: &str = "14-alpine";

#[derive(Debug, Clone)]
struct RecoveredEvent {
    kind: String,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

impl DomainEvent for RecoveredEvent {
    fn aggregate_id(&self) -> &str {
        ""
    }

    fn event_type(&self) -> &str {
        &self.kind
    }

    fn payload(&self) -> &serde_json::Value {
        &self.payload
    }

    fn occurred_at(&self) -> &DateTime<Utc> {
        &self.occurred_at
    }
}

fn deserialize(
    event_type: &str,
    payload: serde_json::Value,
) -> Result<RecoveredEvent, PersistenceError> {
    Ok(RecoveredEvent {
        kind: event_type.to_string(),
        payload,
        occurred_at: Utc::now(),
    })
}

type Deserializer = fn(&str, serde_json::Value) -> Result<RecoveredEvent, PersistenceError>;

async fn postgres_store() -> (
    PostgreSQLEventStore<RecoveredEvent, Deserializer>,
    ContainerAsync<Postgres>,
) {
    let container = Postgres::default()
        .with_tag(POSTGRES_IMAGE_TAG)
        .start()
        .await
        .expect("the Postgres testcontainer must start; this test cannot run without Docker");

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("the container must publish its mapped Postgres port");
    let host = container
        .get_host()
        .await
        .expect("the container must report a reachable host address")
        .to_string();
    let host = if host == "localhost" {
        "127.0.0.1".to_string()
    } else {
        host
    };

    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .connect(&url)
        .await
        .expect("must be able to connect to the freshly started container");
    migrations::run(&pool)
        .await
        .expect("the framework's own migrations must apply cleanly");

    let store = PostgreSQLEventStore::open(pool, deserialize as Deserializer)
        .await
        .expect("the store must open against a freshly migrated schema");
    (store, container)
}

/// The durable store reports an aggregate with no events as absent.
///
/// Characterizing the store's own answer before asserting anything about
/// recovery, so a later change to either is visible as a change to the right one.
#[tokio::test]
async fn the_durable_store_reports_a_never_written_aggregate_as_absent() {
    let (store, _container) = postgres_store().await;

    match store
        .load("counter", "never-written", Some("tenant-1"))
        .await
    {
        Err(PersistenceError::NotFound { .. }) => {}
        other => panic!("expected the stream to read as absent, got {other:?}"),
    }
}

/// Recovering a never-persisted aggregate through the facade succeeds, against the
/// durable store, and yields no snapshot and no events.
///
/// This is the first activation of every entity. Before this slice the facade
/// propagated the store's `NotFound` as a recovery failure, so no entity could be
/// activated for the first time against the durable store — a defect no test could
/// see, because every recovery test used the in-memory store, and that one returned
/// an empty stream instead of reporting absence.
#[tokio::test]
async fn recovery_of_a_fresh_aggregate_succeeds_against_the_durable_store() {
    let (store, _container) = postgres_store().await;

    let facade: PersistenceFacade<RecoveredEvent> = PersistenceFacade::with_stores(
        Arc::new(AsyncMutex::new(store)),
        Arc::new(parking_lot::Mutex::new(InMemorySnapshotStore::new())),
    );

    let (snapshot, events) = facade
        .load_for_recovery("counter", "never-written", Some("tenant-1"))
        .await
        .expect(
            "an aggregate with no events yet is the ordinary first state of every entity, not a \
             recovery failure",
        );

    assert!(snapshot.is_none(), "there is no snapshot to find");
    assert!(events.is_empty(), "there are no events to replay");
}

/// The in-memory store reaches the identical outcome through the facade.
///
/// The point is not that both work — it is that both work *the same way*. The two
/// stores disagree about how to report an absent stream, and this asserts that the
/// disagreement is invisible to recovery, which is the only caller that has to
/// care.
#[tokio::test]
async fn recovery_of_a_fresh_aggregate_succeeds_against_the_in_memory_store() {
    let facade: PersistenceFacade<RecoveredEvent> = PersistenceFacade::with_stores(
        Arc::new(AsyncMutex::new(InMemoryEventStore::<RecoveredEvent>::new())),
        Arc::new(parking_lot::Mutex::new(InMemorySnapshotStore::new())),
    );

    let (snapshot, events) = facade
        .load_for_recovery("counter", "never-written", Some("tenant-1"))
        .await
        .expect("recovery of a fresh aggregate must succeed against the in-memory store too");

    assert!(snapshot.is_none());
    assert!(events.is_empty());
}
