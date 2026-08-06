//! The PostgreSQL event store, judged against the same shared `EventStore`
//! conformance contract as the in-memory one.
//!
//! The in-memory store runs this identical harness in
//! `crates/infrastructure/tests/in_memory_event_store_conformance.rs`. That
//! pairing is the whole point: both implementations satisfied the trait's
//! signature while disagreeing about the tenant-less ("systemwide") partition,
//! and nothing in the workspace compared them. One contract, two adapters, one
//! harness.

use chrono::{DateTime, Utc};
use ego_domain::event::DomainEvent;
use ego_domain::persistence::PersistenceError;
use ego_persistence::postgres::migrations;
use ego_persistence::PostgreSQLEventStore;
use ego_testkit::assert_event_store_conformance;
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::postgres::Postgres;

/// Pinned explicitly, matching the framework's declared PostgreSQL 14 floor.
const POSTGRES_IMAGE_TAG: &str = "14-alpine";

#[derive(Debug, Clone)]
struct ConformanceEvent {
    kind: String,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

impl DomainEvent for ConformanceEvent {
    fn aggregate_id(&self) -> &str {
        // The store takes the aggregate identity as explicit arguments to
        // `append`, never from the event, so nothing in the contract reads this.
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
) -> Result<ConformanceEvent, PersistenceError> {
    Ok(ConformanceEvent {
        kind: event_type.to_string(),
        payload,
        occurred_at: Utc::now(),
    })
}

type Deserializer = fn(&str, serde_json::Value) -> Result<ConformanceEvent, PersistenceError>;

// `append`/`load` bridge into async code via `block_in_place`, which panics on a
// current-thread runtime, so the multi-thread flavor is load-bearing.
#[tokio::test(flavor = "multi_thread")]
async fn the_postgres_event_store_conforms() {
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

    let mut store = PostgreSQLEventStore::open(pool, deserialize as Deserializer)
        .await
        .expect("the store must open against a freshly migrated schema");

    assert_event_store_conformance(&mut store, |kind| ConformanceEvent {
        kind: kind.to_string(),
        payload: serde_json::json!({ "kind": kind }),
        occurred_at: Utc::now(),
    });

    // Keep the container alive until the assertions are done.
    drop(container);
}
