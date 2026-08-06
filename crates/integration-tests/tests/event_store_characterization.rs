//! Characterization tests for the synchronous PostgreSQL `EventStore::append`
//! path, run against a real Postgres container.
//!
//! These tests pin down today's observable behavior before that contract
//! changes. They are deliberately not testing the desired future behavior —
//! they document what the code does right now, so a later behavior change is
//! visible as an intentional, reviewed diff to this file rather than a silent
//! regression.
//!
//! One gap this file used to document — since closed: the `events` table had
//! only a non-unique index over the stream identity, so the database provided no
//! uniqueness guarantee for it, and two concurrent appends with the same expected
//! version could both pass the in-process check and both insert. The
//! error-mapping code path for a unique violation existed in `append` and nothing
//! in the schema could trigger it.
//!
//! The gap test carried an instruction in its own failure message: rewrite it to
//! assert the opposite the day a real guarantee arrives, rather than delete it.
//! That happened, and the rewrite is at the bottom of this file. The behaviour
//! that violation now produces is pinned in `stream_identity_uniqueness.rs`, and
//! the index shape in `schema_index_assertion.rs`.
//!
//! Another gap this file discovered rather than assumed — since closed: the
//! append/load tests below use a concrete tenant, not the NULL-tenant
//! ("systemwide") mode that the rest of this store's API otherwise treats as a
//! first-class case. Against a real database, comparing a column to a bound
//! `NULL` parameter with plain `=` never matches — SQL's three-valued logic
//! makes `tenant_id = NULL` evaluate to unknown, not true, for every row — so
//! the version-check `SELECT` inside `append` and the `SELECT` inside `load`
//! both silently behaved as if the aggregate had no prior history whenever the
//! caller passed `tenant_id: None`.
//!
//! That gap is now fixed: all three queries use `tenant_id IS NOT DISTINCT
//! FROM`, which compares two NULLs as equal while keeping NULL distinct from
//! any concrete tenant. The behaviour is pinned in `systemwide_streams.rs`
//! rather than here, because this file characterizes what the store did before
//! the change and those tests assert what it does after. These tests still use
//! a concrete tenant, which is now a statement about their scope rather than an
//! avoidance of a defect.

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

    let store = PostgreSQLEventStore::open(pool.clone(), deserialize as Deserializer)
        .await
        .expect("the store must open once every row carries its aggregate type");
    (store, pool, container)
}

// These ran on `flavor = "multi_thread"` because the store bridged async to sync
// with `block_in_place`, which panics outright on a current-thread runtime — a
// storage detail that had leaked into a test attribute. The trait is asynchronous
// now, the bridge is gone, and these run on the default current-thread runtime.
// The change of attribute is itself part of what this file characterizes.
#[tokio::test]
async fn append_advances_the_version_and_load_returns_events_in_order() {
    let (mut store, _pool, _container) = start_store().await;
    let aggregate_type = "order";
    let aggregate_id = "1";
    let aggregate = "order-1";
    // A concrete tenant, deliberately — see the module-level note above these
    // tests about the NULL-tenant ("systemwide") mode against a real database.
    let tenant = Some("tenant-1");

    let events = vec![
        StoredEvent::without_correlation(recorded_event(aggregate, "OrderCreated")),
        StoredEvent::without_correlation(recorded_event(aggregate, "OrderLineAdded")),
    ];

    let new_version = store
        .append(aggregate_type, aggregate_id, tenant, 0, events)
        .await
        .expect("appending to a fresh aggregate at the correct expected version must succeed");
    assert_eq!(
        new_version, 2,
        "two appended events must advance the version by two"
    );

    let loaded = store
        .load(aggregate_type, aggregate_id, tenant)
        .await
        .expect("a stream that was just appended to must be loadable");
    assert_eq!(
        loaded.len(),
        2,
        "load must return exactly the two appended events"
    );
    assert_eq!(loaded[0].event.event_type(), "OrderCreated");
    assert_eq!(loaded[1].event.event_type(), "OrderLineAdded");
}

#[tokio::test]
async fn append_rejects_a_stale_expected_version_via_the_explicit_version_check() {
    let (mut store, _pool, _container) = start_store().await;
    let aggregate_type = "order";
    let aggregate_id = "2";
    let aggregate = "order-2";
    // A concrete tenant, deliberately — see the module-level note above these
    // tests about the NULL-tenant ("systemwide") mode against a real database.
    let tenant = Some("tenant-1");

    let first_version = store
        .append(
            aggregate_type,
            aggregate_id,
            tenant,
            0,
            vec![StoredEvent::without_correlation(recorded_event(
                aggregate,
                "OrderCreated",
            ))],
        )
        .await
        .expect("the first append at expected_version 0 must succeed");
    assert_eq!(first_version, 1);

    // Retrying with the same stale `expected_version = 0` must be rejected.
    // Today that rejection comes entirely from the explicit
    // `SELECT COALESCE(MAX(version), 0)` read-then-compare inside `append` —
    // there is no unique index or constraint on the stream identity that
    // could produce this outcome instead (see the schema assertion below).
    let retried = store
        .append(
            aggregate_type,
            aggregate_id,
            tenant,
            0,
            vec![StoredEvent::without_correlation(recorded_event(
                aggregate,
                "OrderCreatedAgain",
            ))],
        )
        .await;

    assert_eq!(
        retried,
        Err(PersistenceError::Conflict {
            aggregate_id: aggregate.to_string(),
            expected: 0,
            actual: 1,
        })
    );
}

/// The gap this file used to document is closed: the database now enforces
/// uniqueness of the stream identity.
///
/// This test was originally written to assert the *absence* of that guarantee,
/// with an instruction in its own failure message to rewrite it — not delete it —
/// the day the guarantee arrived. That day is this slice, and this is the rewrite.
/// Keeping it means the file still records the transition: what the store used to
/// rely on, and what now backs it.
///
/// It deliberately asserts only that a unique guarantee over the identity exists.
/// The precise shape — columns, order, NULL treatment, index names — is pinned in
/// `schema_index_assertion.rs`, and duplicating it here would create two places
/// to update and one to forget.
#[tokio::test]
async fn the_events_table_now_enforces_uniqueness_of_the_stream_identity() {
    let (_store, pool, _container) = start_store().await;

    // From the live schema, not from reading the migration source, so this
    // reflects what Postgres actually enforces.
    let indexes: Vec<(String, String)> =
        sqlx::query_as("SELECT indexname, indexdef FROM pg_indexes WHERE tablename = 'events'")
            .fetch_all(&pool)
            .await
            .expect("pg_indexes must be queryable against the migrated schema");

    let unique_identity_indexes: Vec<&(String, String)> = indexes
        .iter()
        .filter(|(_, def)| def.contains("aggregate_id") && def.to_uppercase().contains("UNIQUE"))
        .collect();

    assert!(
        !unique_identity_indexes.is_empty(),
        "the stream identity must be protected by at least one unique index; without one, two \
         concurrent appends can both pass the in-process version check and both insert, which is \
         the state this test used to document. Found: {indexes:?}"
    );

    // The in-process version check is no longer the only thing standing between a
    // retry and a duplicate row, which is what the append tests above rely on.
    let non_unique_identity_only = indexes
        .iter()
        .filter(|(_, def)| def.contains("aggregate_id"))
        .all(|(_, def)| !def.to_uppercase().contains("UNIQUE"));
    assert!(
        !non_unique_identity_only,
        "every index over the identity is non-unique, so the database enforces nothing"
    );
}
