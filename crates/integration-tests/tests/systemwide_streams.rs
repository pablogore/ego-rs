//! Real-Postgres tests for the NULL-tenant ("systemwide") mode of the
//! PostgreSQL event store.
//!
//! `resolve_tenant(None)` resolves to SQL NULL, and every query in the store
//! compared `tenant_id` to its bound parameter with plain `=`. Against a real
//! database that never matches: SQL's three-valued logic makes `tenant_id =
//! NULL` evaluate to unknown rather than true, for every row including the ones
//! whose `tenant_id` genuinely is NULL. So a systemwide stream was invisible to
//! its own reads — the version check always saw an empty stream, `load` always
//! reported the aggregate as absent, and `list_aggregate_ids` never listed it.
//!
//! The characterization suite discovered this gap and deliberately declined to
//! pin it either way, deferring the query change to the uniqueness work rather
//! than freezing behaviour it considered a defect. This file is where that gap
//! gets closed and pinned.
//!
//! The tests use a concrete tenant alongside the systemwide one wherever the
//! distinction matters, because the null-safe comparison has two failure modes
//! and only one of them is "no rows match". The other is a comparison that
//! matches *too much* — and a systemwide read that returned a tenant's events
//! would be a tenant-isolation breach, which is strictly worse than the bug
//! being fixed.

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
/// Never a floating tag — see `event_store_characterization.rs`.
const POSTGRES_IMAGE_TAG: &str = "14-alpine";

#[derive(Debug, Clone)]
struct RecordedEvent {
    aggregate: String,
    kind: String,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

impl DomainEvent for RecordedEvent {
    fn aggregate_id(&self) -> &str {
        &self.aggregate
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

fn recorded_event(aggregate: &str, kind: &str) -> RecordedEvent {
    RecordedEvent {
        aggregate: aggregate.to_string(),
        kind: kind.to_string(),
        payload: serde_json::json!({ "aggregate": aggregate, "kind": kind }),
        occurred_at: Utc::now(),
    }
}

fn deserialize(
    event_type: &str,
    payload: serde_json::Value,
) -> Result<RecordedEvent, PersistenceError> {
    let aggregate = payload
        .get("aggregate")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(RecordedEvent {
        aggregate,
        kind: event_type.to_string(),
        payload,
        occurred_at: Utc::now(),
    })
}

type Deserializer = fn(&str, serde_json::Value) -> Result<RecordedEvent, PersistenceError>;

/// Starts a fresh container, applies the framework's migrations, and opens a
/// store. No Docker-less fallback: an unavailable daemon fails loudly rather
/// than silently skipping.
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

// The default current-thread runtime is enough: the store's methods are
// asynchronous, so nothing here bridges async to sync and no test in this file
// needs concurrency.

/// A systemwide stream is visible to its own reads: the version check sees the
/// history it just wrote, so a second append advances rather than restarting.
///
/// This is the core of the defect. With the broken comparison the second append
/// read an empty stream, so it rejected `expected_version = 1` as stale while
/// reporting the stream's version as 0 — and an append at `expected_version = 0`
/// would have inserted a *second* version-1 row, duplicating history silently.
#[tokio::test]
async fn a_systemwide_stream_advances_its_version_across_appends() {
    let (mut store, _pool, _container) = start_store().await;

    let first = store
        .append(
            "order",
            "1",
            None,
            0,
            vec![StoredEvent::without_correlation(recorded_event(
                "order-1",
                "OrderCreated",
            ))],
        )
        .await
        .expect("the first systemwide append at expected version 0 must succeed");
    assert_eq!(first, 1);

    let second = store
        .append(
            "order",
            "1",
            None,
            1,
            vec![StoredEvent::without_correlation(recorded_event(
                "order-1",
                "OrderLineAdded",
            ))],
        )
        .await
        .expect("the second systemwide append must see the first one's version");
    assert_eq!(
        second, 2,
        "a systemwide stream must advance, not restart from an apparently empty history"
    );
}

/// `load` returns a systemwide stream's events, in order.
#[tokio::test]
async fn load_returns_a_systemwide_stream_rather_than_reporting_it_absent() {
    let (mut store, _pool, _container) = start_store().await;

    store
        .append(
            "order",
            "1",
            None,
            0,
            vec![
                StoredEvent::without_correlation(recorded_event("order-1", "OrderCreated")),
                StoredEvent::without_correlation(recorded_event("order-1", "OrderLineAdded")),
            ],
        )
        .await
        .expect("appending a systemwide stream must succeed");

    let loaded = store
        .load("order", "1", None)
        .await
        .expect("a systemwide stream that was just appended to must be loadable");
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].event.event_type(), "OrderCreated");
    assert_eq!(loaded[1].event.event_type(), "OrderLineAdded");
}

/// Fixing the comparison must not weaken the version check: a stale expected
/// version is still rejected, and the reported actual version is the real one.
///
/// Without this, "the systemwide read sees its own history" could be satisfied by
/// a comparison that matches every row, which would pass the two tests above for
/// entirely the wrong reason.
#[tokio::test]
async fn a_stale_expected_version_is_still_rejected_for_a_systemwide_stream() {
    let (mut store, _pool, _container) = start_store().await;

    store
        .append(
            "order",
            "1",
            None,
            0,
            vec![StoredEvent::without_correlation(recorded_event(
                "order-1",
                "OrderCreated",
            ))],
        )
        .await
        .expect("the first systemwide append must succeed");

    let retried = store
        .append(
            "order",
            "1",
            None,
            0,
            vec![StoredEvent::without_correlation(recorded_event(
                "order-1",
                "OrderCreated",
            ))],
        )
        .await;
    assert_eq!(
        retried,
        Err(PersistenceError::Conflict {
            aggregate_id: "order-1".to_string(),
            expected: 0,
            actual: 1,
        }),
        "the retry must be rejected against the version the systemwide stream really has"
    );
}

/// The systemwide partition and a tenant's partition are separate streams even
/// when they share a type and an id.
///
/// This is the test that keeps the fix honest. `IS NOT DISTINCT FROM` compares
/// two NULLs as equal, which is the whole point — but it must not compare NULL
/// as equal to a concrete tenant. If it did, a systemwide read would return
/// another tenant's events: an isolation breach, strictly worse than the
/// invisibility being fixed.
#[tokio::test]
async fn a_systemwide_stream_and_a_tenant_stream_with_the_same_identity_stay_separate() {
    let (mut store, _pool, _container) = start_store().await;

    store
        .append(
            "order",
            "1",
            None,
            0,
            vec![StoredEvent::without_correlation(recorded_event(
                "order-1",
                "SystemwideEvent",
            ))],
        )
        .await
        .expect("the systemwide append must succeed");

    // Same type, same id, different partition — so this is a fresh stream and
    // its own expected version is 0, not 1.
    store
        .append(
            "order",
            "1",
            Some("tenant-1"),
            0,
            vec![StoredEvent::without_correlation(recorded_event(
                "order-1",
                "TenantEvent",
            ))],
        )
        .await
        .expect("a tenant stream sharing the identity must be independent of the systemwide one");

    let systemwide = store
        .load("order", "1", None)
        .await
        .expect("the systemwide stream must load");
    assert_eq!(
        systemwide.len(),
        1,
        "the systemwide read must not see the tenant's event"
    );
    assert_eq!(systemwide[0].event.event_type(), "SystemwideEvent");

    let tenant = store
        .load("order", "1", Some("tenant-1"))
        .await
        .expect("the tenant stream must load");
    assert_eq!(
        tenant.len(),
        1,
        "the tenant read must not see the systemwide event"
    );
    assert_eq!(tenant[0].event.event_type(), "TenantEvent");
}

/// `list_aggregate_ids` lists systemwide aggregates, and lists them only in the
/// partition they belong to.
#[tokio::test]
async fn list_aggregate_ids_covers_the_systemwide_partition() {
    let (mut store, _pool, _container) = start_store().await;

    store
        .append(
            "order",
            "1",
            None,
            0,
            vec![StoredEvent::without_correlation(recorded_event(
                "order-1",
                "SystemwideEvent",
            ))],
        )
        .await
        .expect("the systemwide append must succeed");
    store
        .append(
            "invoice",
            "9",
            Some("tenant-1"),
            0,
            vec![StoredEvent::without_correlation(recorded_event(
                "invoice-9",
                "TenantEvent",
            ))],
        )
        .await
        .expect("the tenant append must succeed");

    let systemwide = store
        .list_aggregate_ids(None)
        .await
        .expect("listing the systemwide partition must succeed");
    assert_eq!(
        systemwide,
        vec![("order".to_string(), "1".to_string())],
        "the systemwide listing must hold exactly its own aggregate"
    );

    let tenant = store
        .list_aggregate_ids(Some("tenant-1"))
        .await
        .expect("listing a tenant partition must succeed");
    assert_eq!(
        tenant,
        vec![("invoice".to_string(), "9".to_string())],
        "the tenant listing must hold exactly its own aggregate"
    );
}
