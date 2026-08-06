//! Real-Postgres tests for the database-enforced uniqueness of the event stream
//! identity, and for the store's translation of the violation it now makes
//! reachable.
//!
//! Until migration 008 the only thing standing between two concurrent appends
//! and two rows at the same version was an in-process read-then-compare: both
//! could read the same `MAX(version)` and both could insert. The error-mapping
//! branch for a unique violation existed in `append` and no schema could trigger
//! it — the characterization suite asserted that gap directly.
//!
//! These tests exercise the guarantee, the translation, and the catalog shape
//! that backs both.

use std::sync::Arc;
use std::time::Duration;

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
/// Load-bearing for this file beyond reproducibility: 14 is the version that
/// lacks `NULLS NOT DISTINCT`, which is why the schema uses a complementary pair
/// of partial indexes instead of one index.
const POSTGRES_IMAGE_TAG: &str = "14-alpine";

const TENANT_INDEX: &str = "ux_events_identity_tenant";
const SYSTEMWIDE_INDEX: &str = "ux_events_identity_systemwide";

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

async fn start_pool() -> (PgPool, ContainerAsync<Postgres>) {
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
        .max_connections(16)
        .connect(&url)
        .await
        .expect("must be able to connect to the freshly started container");

    migrations::run(&pool)
        .await
        .expect("the framework's own migrations must apply cleanly");

    (pool, container)
}

async fn insert_raw_event(
    pool: &PgPool,
    aggregate_type: &str,
    aggregate_id: &str,
    tenant_id: Option<&str>,
    version: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO events (aggregate_type, aggregate_id, tenant_id, version, event_type, payload) \
         VALUES ($1, $2, $3, $4, 'Probed', '{}'::jsonb) RETURNING id",
    )
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(version)
    .fetch_one(pool)
    .await
}

fn sqlstate(err: &sqlx::Error) -> Option<String> {
    match err {
        sqlx::Error::Database(db) => db.code().map(|c| c.to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The database refuses a duplicate identity — in both partitions
// ---------------------------------------------------------------------------

/// A duplicate identity is rejected by the database for a concrete tenant, with
/// SQLSTATE 23505.
///
/// The insert goes in as raw SQL rather than through `append`, deliberately:
/// `append`'s own version check would reject the second row before the database
/// ever saw it, so routing through it would test the in-process guard and prove
/// nothing about the schema.
#[tokio::test(flavor = "multi_thread")]
async fn the_database_refuses_a_duplicate_identity_for_a_tenant() {
    let (pool, _container) = start_pool().await;

    insert_raw_event(&pool, "order", "1", Some("tenant-1"), 1)
        .await
        .expect("the first row must insert");

    let duplicate = insert_raw_event(&pool, "order", "1", Some("tenant-1"), 1).await;
    let err = duplicate.expect_err("a duplicate stream identity must be refused by the database");
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("23505"),
        "the refusal must be a unique-violation, not some other failure: {err}"
    );
    assert!(
        format!("{err}").contains(TENANT_INDEX),
        "the violation must name the tenant-partition index: {err}"
    );
}

/// A duplicate identity is rejected in the tenant-less partition too.
///
/// This is the case a single conventional `UNIQUE` would have missed entirely.
/// Postgres treats every NULL as distinct from every other NULL, so a four-column
/// unique index would have permitted unlimited duplicates here — the very
/// partition where silent history duplication was already found. The partial
/// index over `tenant_id IS NULL` is what closes it on a server that has no
/// `NULLS NOT DISTINCT`.
#[tokio::test(flavor = "multi_thread")]
async fn the_database_refuses_a_duplicate_identity_in_the_systemwide_partition() {
    let (pool, _container) = start_pool().await;

    insert_raw_event(&pool, "order", "1", None, 1)
        .await
        .expect("the first systemwide row must insert");

    let duplicate = insert_raw_event(&pool, "order", "1", None, 1).await;
    let err =
        duplicate.expect_err("a duplicate systemwide identity must be refused by the database");
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some("23505"),
        "the refusal must be a unique-violation: {err}"
    );
    assert!(
        format!("{err}").contains(SYSTEMWIDE_INDEX),
        "the violation must name the systemwide-partition index: {err}"
    );
}

/// Two different tenants may hold the identical type, id and version.
///
/// The negative test. Uniqueness is scoped to a partition, not global: without
/// this, an index that merely rejected everything would satisfy the two tests
/// above. A tenant whose writes fail because another tenant wrote first would be
/// a far worse defect than the duplication being prevented.
#[tokio::test(flavor = "multi_thread")]
async fn two_tenants_may_hold_the_same_identity() {
    let (pool, _container) = start_pool().await;

    insert_raw_event(&pool, "order", "1", Some("tenant-a"), 1)
        .await
        .expect("tenant-a's row must insert");
    insert_raw_event(&pool, "order", "1", Some("tenant-b"), 1)
        .await
        .expect("tenant-b must be allowed the identical type, id and version");
    // And the systemwide partition is a third, independent one.
    insert_raw_event(&pool, "order", "1", None, 1)
        .await
        .expect("the systemwide partition must be allowed it too");

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE aggregate_type = 'order' AND aggregate_id = '1'",
    )
    .fetch_one(&pool)
    .await
    .expect("counting must succeed");
    assert_eq!(
        count, 3,
        "all three partitions must coexist at the same identity"
    );
}

// ---------------------------------------------------------------------------
// The store translates the violation it now makes reachable
// ---------------------------------------------------------------------------

/// Polls the catalog until some backend is waiting on a lock it has not been
/// granted, so the caller knows the competing statement is genuinely in flight.
///
/// This is what makes the test below deterministic rather than timing-based. The
/// alternative — sleep and hope — would pass or fail depending on machine load,
/// and a race test that sometimes exercises nothing is worse than no test,
/// because it reports success either way.
async fn wait_until_a_statement_is_blocked(pool: &PgPool) {
    for _ in 0..200 {
        let blocked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pg_locks WHERE NOT granted")
            .fetch_one(pool)
            .await
            .expect("the lock catalog must be queryable");
        if blocked > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "no statement ever blocked on a lock. The premise of this test is that the append's \
         INSERT waits for the uncommitted competing row, so if nothing blocks then either the \
         unique index is absent or the append never reached its INSERT — both of which are \
         failures, not flakes"
    );
}

/// A unique violation inside `append` becomes a domain conflict reporting the
/// version the stream really has — and the branch is reached deterministically.
///
/// Within a single transaction the violation is unreachable by construction: the
/// version check reads `MAX(version)`, so no existing row sits where the insert is
/// about to write. It becomes reachable only when a competing writer commits
/// between the read and the insert. This test creates exactly that window without
/// depending on timing:
///
/// 1. Another connection inserts version 1 and **does not commit**. The row is
///    invisible to everyone else, but the unique index already holds its slot.
/// 2. `append` runs at expected version 0. Its version check cannot see the
///    uncommitted row, so it reads 0, agrees, and issues its `INSERT`.
/// 3. That `INSERT` **blocks**, waiting for the other transaction to end. The
///    catalog poll above confirms the wait rather than assuming it.
/// 4. The competing transaction commits. The blocked insert now fails with 23505.
///
/// The blocking observation is also what proves *which* guard fired. Had the
/// in-process check caught this, `append` would have returned immediately and
/// nothing would ever have waited on a lock.
///
/// The assertion that matters is `actual`. Before this slice the branch reported
/// `current`, which the version check has already proven equal to
/// `expected_version` — a conflict claiming expected and actual are the same
/// number, which is self-contradictory and useless to act on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_unique_violation_becomes_a_conflict_reporting_the_real_version() {
    let (pool, _container) = start_pool().await;

    // (1) The competing writer holds the slot without committing.
    let mut competing = pool
        .begin()
        .await
        .expect("the competing transaction must begin");
    sqlx::query(
        "INSERT INTO events (aggregate_type, aggregate_id, tenant_id, version, event_type, payload) \
         VALUES ('order', 'blocked', 'tenant-1', 1, 'Competing', '{}'::jsonb)",
    )
    .execute(&mut *competing)
    .await
    .expect("the competing insert must succeed inside its own transaction");

    let mut store = PostgreSQLEventStore::open(pool.clone(), deserialize as Deserializer)
        .await
        .expect("the store must open");

    // (2, 3) The append's check reads 0, its insert blocks.
    let appending = tokio::spawn(async move {
        store.append(
            "order",
            "blocked",
            Some("tenant-1"),
            0,
            vec![event("LosesTheRace")],
        )
    });
    wait_until_a_statement_is_blocked(&pool).await;

    // (4) Releasing the competing row turns the wait into a refusal.
    competing
        .commit()
        .await
        .expect("the competing transaction must commit");

    match appending.await.expect("the appending task must not panic") {
        Err(PersistenceError::Conflict {
            aggregate_id,
            expected,
            actual,
        }) => {
            assert_eq!(aggregate_id, "order-blocked");
            assert_eq!(expected, 0, "the caller expected an empty stream");
            assert_eq!(
                actual, 1,
                "the conflict must report the version the competing writer left behind, never \
                 the caller's own expected value"
            );
        }
        other => panic!(
            "a unique violation must surface as a domain conflict, not a generic error: {other:?}"
        ),
    }

    // The database held the line: one row, not two.
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE aggregate_type = 'order' AND aggregate_id = 'blocked'",
    )
    .fetch_one(&pool)
    .await
    .expect("counting must succeed");
    assert_eq!(rows, 1);
}

/// Concurrent appends racing for one version: exactly one wins, every loser gets
/// a domain conflict, and the table holds one row.
///
/// The end-to-end invariant, complementing the deterministic test above. This one
/// does not control which guard catches each loser — the in-process check and the
/// database will each catch some, depending on scheduling — and it does not need
/// to. What it asserts holds either way: one winner, no `Internal` errors, one
/// row. A loser reporting a generic failure is the "not a generic error" half of
/// the requirement; a second row would mean the database did not hold.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_appends_for_one_version_produce_one_winner_and_only_conflicts() {
    let (pool, _container) = start_pool().await;

    const WRITERS: usize = 6;
    // A Tokio barrier, not a std one: awaiting yields the worker thread, where
    // blocking it would risk starving the tasks that have not arrived yet.
    let barrier = Arc::new(tokio::sync::Barrier::new(WRITERS));
    let mut handles = Vec::with_capacity(WRITERS);

    for _ in 0..WRITERS {
        let pool = pool.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let mut store = PostgreSQLEventStore::open(pool, deserialize as Deserializer)
                .await
                .expect("each writer must be able to open the store");
            barrier.wait().await;
            store.append(
                "order",
                "contested",
                Some("tenant-1"),
                0,
                vec![event("Raced")],
            )
        }));
    }

    let mut winners = 0;
    let mut conflicts = 0;
    for handle in handles {
        match handle.await.expect("no writer task may panic") {
            Ok(version) => {
                assert_eq!(version, 1, "the winning append must produce version 1");
                winners += 1;
            }
            Err(PersistenceError::Conflict {
                expected, actual, ..
            }) => {
                assert_eq!(expected, 0, "every loser expected version 0");
                assert_eq!(
                    actual, 1,
                    "a loser must be told the version the stream really reached"
                );
                conflicts += 1;
            }
            Err(other) => panic!(
                "a losing writer must report a domain conflict, not a generic failure: {other:?}"
            ),
        }
    }

    assert_eq!(winners, 1, "exactly one append may succeed");
    assert_eq!(
        conflicts,
        WRITERS - 1,
        "every append that did not win must be a conflict"
    );

    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE aggregate_type = 'order' AND aggregate_id = 'contested'",
    )
    .fetch_one(&pool)
    .await
    .expect("counting must succeed");
    assert_eq!(
        rows, 1,
        "the database must hold exactly one row for the contested version"
    );
}
