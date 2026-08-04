//! Characterization tests for the synchronous PostgreSQL `EventStore::append`
//! path, run against a real Postgres container.
//!
//! These tests pin down today's observable behavior before that contract
//! changes. They are deliberately not testing the desired future behavior —
//! they document what the code does right now, so a later behavior change is
//! visible as an intentional, reviewed diff to this file rather than a silent
//! regression.
//!
//! One thing this file explicitly does NOT assert: a unique-constraint
//! violation being mapped to a conflict error. The `events` table today has
//! only a non-unique index over the stream identity, so the database itself
//! provides no uniqueness guarantee for it — two concurrent appends with the
//! same expected version can both pass the in-process check and both insert.
//! The corresponding error-mapping code path exists in `append`, but nothing
//! in the current schema can trigger it. The test below asserts that gap
//! directly by inspecting the live schema, so it fails loudly (and correctly)
//! the day a real uniqueness guarantee is added — at which point it needs to
//! be rewritten to assert the opposite, not deleted.
//!
//! Another gap this file discovered rather than assumed: the append/load
//! tests below use a concrete tenant, not the NULL-tenant ("systemwide")
//! mode that the rest of this store's API otherwise treats as a first-class
//! case. Against a real database, comparing a column to a bound `NULL`
//! parameter with plain `=` never matches — SQL's three-valued logic makes
//! `tenant_id = NULL` evaluate to unknown, not true, for every row — so the
//! version-check `SELECT` inside `append` and the `SELECT` inside `load`
//! both silently behave as if the aggregate has no prior history whenever
//! the caller passes `tenant_id: None`. That is a real, currently-unpinned
//! behavior gap in the systemwide mode. The null-safe form is
//! `tenant_id IS NOT DISTINCT FROM $2`, which compares equal when both sides
//! are NULL, but changing the query belongs with the uniqueness work rather
//! than here. So this file does not pin the gap either way; it only avoids
//! exercising it, to keep these characterization tests honestly describing
//! what they actually tested.

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

/// The container image tag is pinned explicitly. Never floating `latest` —
/// a floating tag would make these tests non-reproducible across runs and
/// could silently change which server version characterizes this suite.
/// PostgreSQL 14 is the declared minimum supported version (see README.md).
const POSTGRES_IMAGE_TAG: &str = "14-alpine";

/// A minimal domain event used only to exercise the store. It carries just
/// enough shape to round-trip through `append`/`load`.
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

/// Reconstructs a `RecordedEvent` from a stored row. `occurred_at` is not
/// round-tripped by the store today (the column is written but never read
/// back into the deserialized event), so this rebuilds it from the current
/// time rather than claiming a value the store does not actually return.
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

/// Starts a fresh, isolated Postgres container, applies the same migrations
/// the framework ships, and returns a store plus a raw pool for direct
/// schema assertions. The container is returned too — dropping it tears the
/// container down, so callers must keep it alive for the test's duration.
///
/// There is no fallback path here. If Docker is unavailable, `start()`
/// panics with a clear message instead of silently skipping the test — a
/// test that quietly does nothing is worse than a test that fails loudly.
async fn start_store() -> (
    PostgreSQLEventStore<RecordedEvent, Deserializer>,
    PgPool,
    ContainerAsync<Postgres>,
) {
    let container = Postgres::default()
        .with_tag(POSTGRES_IMAGE_TAG)
        .start()
        .await
        .expect(
            "the Postgres testcontainer must start; if Docker is not running \
             this test cannot run and must fail loudly, not be skipped",
        );

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("the container must publish its mapped Postgres port");

    // Ask the container where it is reachable rather than assuming loopback: a
    // hard-coded address works only when the Docker daemon is local and
    // publishes on the loopback interface, and breaks against a remote or
    // VM-backed daemon that exposes a different one.
    //
    // `localhost` is then normalised to its literal address. The two are
    // equivalent by definition, and going straight to the literal removes a
    // name-resolution step that is not always available — a host whose
    // /etc/hosts lacks a plain `localhost` entry fails to connect with an
    // opaque "nodename nor servname provided" error that says nothing about
    // the real cause. A genuinely remote host is still honoured as reported.
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

    let store = PostgreSQLEventStore::new(pool.clone(), deserialize as Deserializer);
    (store, pool, container)
}

// `PostgreSQLEventStore::append`/`load` bridge into async code internally via
// `tokio::task::block_in_place` (see `crates/persistence/src/postgres/event_store.rs`),
// which panics outright on a current-thread runtime. `flavor = "multi_thread"`
// is therefore load-bearing here, not a style choice — it is part of what
// this file characterizes about the store's current (synchronous-looking,
// actually-blocking) API.
#[tokio::test(flavor = "multi_thread")]
async fn append_advances_the_version_and_load_returns_events_in_order() {
    let (mut store, _pool, _container) = start_store().await;
    let aggregate = "order-1";
    // A concrete tenant, deliberately — see the module-level note above these
    // tests about the NULL-tenant ("systemwide") mode against a real database.
    let tenant = Some("tenant-1");

    let events = vec![
        StoredEvent::without_correlation(recorded_event(aggregate, "OrderCreated")),
        StoredEvent::without_correlation(recorded_event(aggregate, "OrderLineAdded")),
    ];

    let new_version = store
        .append(aggregate, tenant, 0, events)
        .expect("appending to a fresh aggregate at the correct expected version must succeed");
    assert_eq!(
        new_version, 2,
        "two appended events must advance the version by two"
    );

    let loaded = store
        .load(aggregate, tenant)
        .expect("a stream that was just appended to must be loadable");
    assert_eq!(
        loaded.len(),
        2,
        "load must return exactly the two appended events"
    );
    assert_eq!(loaded[0].event.event_type(), "OrderCreated");
    assert_eq!(loaded[1].event.event_type(), "OrderLineAdded");
}

#[tokio::test(flavor = "multi_thread")]
async fn append_rejects_a_stale_expected_version_via_the_explicit_version_check() {
    let (mut store, _pool, _container) = start_store().await;
    let aggregate = "order-2";
    // A concrete tenant, deliberately — see the module-level note above these
    // tests about the NULL-tenant ("systemwide") mode against a real database.
    let tenant = Some("tenant-1");

    let first_version = store
        .append(
            aggregate,
            tenant,
            0,
            vec![StoredEvent::without_correlation(recorded_event(
                aggregate,
                "OrderCreated",
            ))],
        )
        .expect("the first append at expected_version 0 must succeed");
    assert_eq!(first_version, 1);

    // Retrying with the same stale `expected_version = 0` must be rejected.
    // Today that rejection comes entirely from the explicit
    // `SELECT COALESCE(MAX(version), 0)` read-then-compare inside `append` —
    // there is no unique index or constraint on the stream identity that
    // could produce this outcome instead (see the schema assertion below).
    let retried = store.append(
        aggregate,
        tenant,
        0,
        vec![StoredEvent::without_correlation(recorded_event(
            aggregate,
            "OrderCreatedAgain",
        ))],
    );

    assert_eq!(
        retried,
        Err(PersistenceError::Conflict {
            aggregate_id: aggregate.to_string(),
            expected: 0,
            actual: 1,
        })
    );
}

#[tokio::test]
async fn events_table_provides_no_uniqueness_guarantee_for_the_stream_identity_today() {
    let (_store, pool, _container) = start_store().await;

    // Every index on `events`, from the live schema — not from reading the
    // migration source, so this reflects what Postgres actually enforces.
    let indexes: Vec<(String, String)> =
        sqlx::query_as("SELECT indexname, indexdef FROM pg_indexes WHERE tablename = 'events'")
            .fetch_all(&pool)
            .await
            .expect("pg_indexes must be queryable against the migrated schema");

    assert!(
        !indexes.is_empty(),
        "the events table must have at least its primary key index"
    );

    let identity_indexes: Vec<&(String, String)> = indexes
        .iter()
        .filter(|(_, def)| def.contains("aggregate_id"))
        .collect();

    assert_eq!(
        identity_indexes.len(),
        1,
        "expected exactly the one non-unique idx_events_aggregate index today; \
         if this changed, the uniqueness gap this test documents may have closed"
    );

    for (name, def) in &identity_indexes {
        assert!(
            !def.to_uppercase().contains("UNIQUE"),
            "index {name} unexpectedly enforces uniqueness on the stream identity \
             ({def}); the database now provides a uniqueness guarantee it did not \
             provide when this test was written — update this test to assert the \
             new guarantee instead of the gap"
        );
    }

    // Belt-and-suspenders: no unique *constraint* (as opposed to a plain
    // index) exists either. A unique constraint always creates a backing
    // index, so this and the check above cover both roads to the same fact.
    let unique_constraints: Vec<(String,)> = sqlx::query_as(
        "SELECT conname FROM pg_constraint WHERE conrelid = 'events'::regclass AND contype = 'u'",
    )
    .fetch_all(&pool)
    .await
    .expect("pg_constraint must be queryable against the migrated schema");

    assert!(
        unique_constraints.is_empty(),
        "no unique constraint should exist on the events table today; found: {unique_constraints:?}"
    );
}
