//! Real-Postgres tests for the event store's unit-of-work semantics.
//!
//! The property under test is the one that cannot be checked without a real
//! transaction: that appends made inside a unit of work are invisible until it
//! commits, and vanish entirely if it does not.
//!
//! An in-memory double can be made to pass a test like this by construction —
//! stage into a buffer, apply on commit — which is exactly why the durable store
//! needs its own. The shared conformance harness runs the same assertions against
//! both, so neither implementation is judged only against itself.

use chrono::{DateTime, Utc};
use ego_domain::event::DomainEvent;
use ego_domain::persistence::{EventStore, PersistenceError, StoredEvent};
use ego_persistence::postgres::migrations;
use ego_persistence::PostgreSQLEventStore;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// Pinned explicitly, matching the framework's declared PostgreSQL 14 floor.
const POSTGRES_IMAGE_TAG: &str = "14-alpine";

#[derive(Debug, Clone)]
struct RecordedEvent {
    kind: String,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

impl DomainEvent for RecordedEvent {
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

fn event(kind: &str) -> StoredEvent<RecordedEvent> {
    StoredEvent::without_correlation(RecordedEvent {
        kind: kind.to_string(),
        payload: serde_json::json!({ "kind": kind }),
        occurred_at: Utc::now(),
    })
}

fn deserialize(
    event_type: &str,
    payload: serde_json::Value,
) -> Result<RecordedEvent, PersistenceError> {
    Ok(RecordedEvent {
        kind: event_type.to_string(),
        payload,
        occurred_at: Utc::now(),
    })
}

type Deserializer = fn(&str, serde_json::Value) -> Result<RecordedEvent, PersistenceError>;

async fn start_store() -> (
    PostgreSQLEventStore<RecordedEvent, Deserializer>,
    PgPool,
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
        .max_connections(8)
        .connect(&url)
        .await
        .expect("must be able to connect to the freshly started container");

    migrations::run(&pool)
        .await
        .expect("the framework's own migrations must apply cleanly");

    let store = PostgreSQLEventStore::open(pool.clone(), deserialize as Deserializer)
        .await
        .expect("the store must open against a freshly migrated schema");
    (store, pool, container)
}

/// Counts rows straight from the table, bypassing the store, so the assertion is
/// about what is durable rather than about what the store chooses to report.
async fn durable_rows(pool: &PgPool, aggregate_id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE aggregate_type = 'order' AND aggregate_id = $1",
    )
    .bind(aggregate_id)
    .fetch_one(pool)
    .await
    .expect("counting must succeed")
}

/// Dropping a unit of work without committing leaves nothing behind.
///
/// This is the whole reason the trait has no `rollback` method: the safe outcome
/// is the one that happens when a caller returns early, is cancelled, or panics —
/// the paths where an explicit call is exactly what gets missed.
#[tokio::test(flavor = "multi_thread")]
async fn dropping_a_unit_of_work_without_committing_persists_nothing() {
    let (store, pool, _container) = start_store().await;

    {
        let mut uow = store.begin().await.expect("beginning must succeed");
        let version = uow
            .append(
                "order",
                "dropped",
                Some("tenant-1"),
                0,
                vec![event("Staged")],
            )
            .await
            .expect("appending inside the unit of work must succeed");
        assert_eq!(
            version, 1,
            "the unit of work must report the version it advanced to, provisional though it is"
        );
        // No commit. The unit of work goes out of scope here.
    }

    assert_eq!(
        durable_rows(&pool, "dropped").await,
        0,
        "an uncommitted unit of work must leave no durable row"
    );
    // And the stream must still read as absent through the store's own API.
    match store.load("order", "dropped", Some("tenant-1")).await {
        Err(PersistenceError::NotFound { .. }) => {}
        other => panic!("the abandoned stream must read as absent, got {other:?}"),
    }
}

/// Committing makes everything appended in the unit of work durable.
///
/// The other half of the pair: without this, "persists nothing" would also be
/// satisfied by a unit of work that never writes at all.
#[tokio::test(flavor = "multi_thread")]
async fn committing_a_unit_of_work_makes_every_append_durable() {
    let (store, pool, _container) = start_store().await;

    let mut uow = store.begin().await.expect("beginning must succeed");
    uow.append(
        "order",
        "committed",
        Some("tenant-1"),
        0,
        vec![event("First"), event("Second")],
    )
    .await
    .expect("appending must succeed");
    uow.commit().await.expect("committing must succeed");

    assert_eq!(
        durable_rows(&pool, "committed").await,
        2,
        "both appended events must be durable after the commit"
    );

    let loaded = store
        .load("order", "committed", Some("tenant-1"))
        .await
        .expect("the committed stream must load");
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].event.event_type(), "First");
    assert_eq!(loaded[1].event.event_type(), "Second");
}

/// Two appends to different streams in one unit of work share a single fate.
///
/// This is the capability `EventStore::append` cannot offer, and the reason the
/// trait exists: `append` commits before returning, so nothing can be made to
/// land atomically alongside it. Here the second append fails its own
/// optimistic-concurrency check, the unit of work is abandoned, and the *first*
/// stream — which succeeded — must be gone too.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_on_the_second_stream_discards_the_first() {
    let (store, pool, _container) = start_store().await;

    // A competing writer already holds version 1 of the second stream, so an
    // append there at expected version 0 must be rejected.
    sqlx::query(
        "INSERT INTO events (aggregate_type, aggregate_id, tenant_id, version, event_type, payload) \
         VALUES ('order', 'second', 'tenant-1', 1, 'Existing', '{}'::jsonb)",
    )
    .execute(&pool)
    .await
    .expect("the competing row must insert");

    {
        let mut uow = store.begin().await.expect("beginning must succeed");
        uow.append("order", "first", Some("tenant-1"), 0, vec![event("Ok")])
            .await
            .expect("the first stream must accept the append");

        let rejected = uow
            .append(
                "order",
                "second",
                Some("tenant-1"),
                0,
                vec![event("Doomed")],
            )
            .await;
        assert!(
            matches!(rejected, Err(PersistenceError::Conflict { .. })),
            "the second append must be rejected by the version check, got {rejected:?}"
        );
        // The unit of work is abandoned rather than committed.
    }

    assert_eq!(
        durable_rows(&pool, "first").await,
        0,
        "the append that succeeded must be discarded along with the one that failed — that \
         shared fate is the entire point of a unit of work"
    );
    assert_eq!(
        durable_rows(&pool, "second").await,
        1,
        "the competing writer's row, committed outside this unit of work, must be untouched"
    );
}

/// Appends inside an open unit of work are invisible to readers outside it.
///
/// Isolation, asserted rather than assumed: without this, the durability tests
/// above would also pass for an implementation that wrote each append
/// immediately and deleted on drop.
#[tokio::test(flavor = "multi_thread")]
async fn an_open_unit_of_work_is_invisible_to_other_readers() {
    let (store, pool, _container) = start_store().await;

    let mut uow = store.begin().await.expect("beginning must succeed");
    uow.append(
        "order",
        "pending",
        Some("tenant-1"),
        0,
        vec![event("Staged")],
    )
    .await
    .expect("appending must succeed");

    assert_eq!(
        durable_rows(&pool, "pending").await,
        0,
        "another connection must not see the uncommitted row"
    );
    match store.load("order", "pending", Some("tenant-1")).await {
        Err(PersistenceError::NotFound { .. }) => {}
        other => panic!("the store's own reader must not see the uncommitted row, got {other:?}"),
    }

    uow.commit().await.expect("committing must succeed");
    assert_eq!(
        durable_rows(&pool, "pending").await,
        1,
        "the row becomes visible only once the unit of work commits"
    );
}
