//! **Guarantee (IS-2):** an N-way concurrent append race on one stream leaves
//! exactly one winner, and each loser's conflict reports the real, winning
//! current version — obtained only after the store's own transaction has
//! already aborted on the unique-constraint violation, by re-reading the
//! stream on a different connection.
//!
//! **Scope, exactly:** the post-`23505` re-read branch
//! (`crates/persistence/src/postgres/event_store.rs:198-247`), never the
//! single-caller stale-expected-version pre-check — that branch is already
//! exercised by IS-1's conformance run
//! (`crates/testkit/src/event_store.rs:124-149`), and re-asserting it here
//! would duplicate coverage rather than add it.
//!
//! **Guarantee (IS-5, same file):** `Option::None` tenant identity is
//! verified behaviorally under SQL's three-valued comparison (`NULL = NULL`
//! is not true), not only from the catalog: NULL-tenant uniqueness is
//! genuinely enforced, the systemwide and tenant-scoped partial indexes never
//! collide with each other, and two distinct systemwide streams never
//! silently collide or merge.
//!
//! # Why this needs a real transaction abort, not a mock
//!
//! A scripted store has no unique-constraint violation at all — a fake
//! collision would have to be programmed in, which proves nothing about the
//! real index. Forcing four real transactions to genuinely collide on
//! `ux_events_identity_tenant` (or `ux_events_identity_systemwide`, for the
//! NULL-tenant case), past the point of a real transaction abort, needs a
//! real PostgreSQL unique index and a real second connection to re-read from.
//!
//! `schema_index_assertion.rs` already pins the catalog shape of both
//! indexes. Only a real insert against real three-valued NULL comparison
//! proves the *behavior* that shape implies.
//!
//! # Determinism without sleeping
//!
//! Four racers could resolve serially by scheduling accident, with the
//! assertions below still passing while proving nothing about a real race.
//! `wait_until_blocked(observer, "%INSERT INTO events%", 4)` polls under an
//! explicit deadline and fails the test outright if all four are never
//! observed genuinely blocked on the holder's table lock before it releases
//! (AD-3, T-00.1).
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use ego_domain::event::DomainEvent;
use ego_domain::persistence::{EventStore, PersistenceError, StoredEvent};
use ego_integration_tests::{isolated_database, wait_until_blocked};
use ego_persistence::postgres::PostgreSQLEventStore;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

const RACERS: usize = 4;
const AGGREGATE_TYPE: &str = "race-aggregate";

/// A local fixture, not a re-derived contract — `crates/infrastructure`'s copy
/// is private to another crate's test target (AD-4, mirrored from
/// `durable_store_conformance_postgres.rs`'s `ConformanceEvent`).
#[derive(Debug, Clone)]
struct RaceEvent {
    kind: String,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

impl DomainEvent for RaceEvent {
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

fn race_event(kind: &str) -> StoredEvent<RaceEvent> {
    StoredEvent::new(RaceEvent {
        kind: kind.to_string(),
        payload: serde_json::json!({ "kind": kind }),
        occurred_at: Utc::now(),
    })
}

fn deserialize_race_event(
    kind: &str,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
) -> Result<RaceEvent, PersistenceError> {
    Ok(RaceEvent {
        kind: kind.to_string(),
        payload,
        occurred_at,
    })
}

async fn connect(url: &str, max: u32) -> PgPool {
    PgPoolOptions::new()
        .max_connections(max)
        .connect(url)
        .await
        .expect("the container accepts connections")
}

/// Forces `RACERS` real transactions to collide on one stream identity and
/// returns their raw `append` results, in racer-index order.
async fn race(
    store: Arc<PostgreSQLEventStore<RaceEvent, fn(&str, serde_json::Value, DateTime<Utc>) -> Result<RaceEvent, PersistenceError>>>,
    test_pool: &PgPool,
    aggregate_id: &str,
    tenant: Option<&str>,
) -> Vec<Result<i64, PersistenceError>> {
    // A table-level lock, not a row lock: no row exists yet for a stream this
    // race is about to create for the first time, so there is nothing to
    // lock a row on. EXCLUSIVE MODE conflicts with the ROW EXCLUSIVE mode
    // `INSERT` takes, but not with the ACCESS SHARE mode a plain `SELECT`
    // takes — exactly the asymmetry this race depends on: every racer's
    // version read must succeed unblocked, and only the write may queue.
    let mut holder = test_pool.begin().await.expect("a transaction begins");
    sqlx::query("LOCK TABLE events IN EXCLUSIVE MODE")
        .execute(&mut *holder)
        .await
        .expect("the table lock is acquired");

    let racers: Vec<_> = (0..RACERS)
        .map(|i| {
            let store = store.clone();
            let aggregate_id = aggregate_id.to_string();
            let tenant = tenant.map(|t| t.to_string());
            tokio::spawn(async move {
                store
                    .append(
                        AGGREGATE_TYPE,
                        &aggregate_id,
                        tenant.as_deref(),
                        0,
                        vec![race_event(&format!("racer-{i}"))],
                    )
                    .await
            })
        })
        .collect();

    wait_until_blocked(test_pool, "%INSERT INTO events%", RACERS).await;

    holder.commit().await.expect("the holder commits");

    let mut results = Vec::with_capacity(RACERS);
    for racer in racers {
        results.push(racer.await.expect("the racer task completes"));
    }
    results
}

#[tokio::test]
async fn an_n_way_append_race_leaves_one_winner_and_reports_the_real_version_after_abort() {
    let db = isolated_database().await;
    let url = db.url().to_string();
    // At least `RACERS`, or a racer would be starved on the pool rather than
    // genuinely blocked on the table lock; design pins 6.
    let store_pool = connect(&url, 6).await;
    let test_pool = connect(&url, 2).await;

    let deserialize: fn(&str, serde_json::Value, DateTime<Utc>) -> Result<RaceEvent, PersistenceError> =
        deserialize_race_event;
    let store = Arc::new(
        PostgreSQLEventStore::open(store_pool, deserialize)
            .await
            .expect("the store opens against a freshly migrated, empty database"),
    );

    // A concrete tenant, so the collision lands on `ux_events_identity_tenant`
    // rather than the systemwide partition IS-5 exercises below.
    let results = race(store, &test_pool, "race-key", Some("tenant-race")).await;

    let winners: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
    let losers: Vec<_> = results.iter().filter(|r| r.is_err()).collect();

    assert_eq!(
        winners.len(),
        1,
        "exactly one of {RACERS} racers appending to the same fresh stream must \
         win; the unique index genuinely serializes them. Got: {results:?}"
    );
    assert_eq!(losers.len(), RACERS - 1);

    for loser in &losers {
        match loser {
            Err(PersistenceError::Conflict {
                expected, actual, ..
            }) => {
                assert_eq!(*expected, 0, "every racer requested version 0");
                assert_eq!(
                    *actual, 1,
                    "the reported actual version must come from the post-abort \
                     re-read on a different connection, reflecting the one \
                     winner's committed write — not the pre-race version every \
                     racer's own aborted transaction read"
                );
            }
            other => panic!("a losing racer must report Conflict, got {other:?}"),
        }
    }

    db.close().await;
}

#[tokio::test]
async fn null_tenant_identity_is_genuinely_unique_not_exempt_and_does_not_collide_with_a_concrete_tenant() {
    let db = isolated_database().await;
    let url = db.url().to_string();
    let store_pool = connect(&url, 6).await;
    let test_pool = connect(&url, 2).await;

    let deserialize: fn(&str, serde_json::Value, DateTime<Utc>) -> Result<RaceEvent, PersistenceError> =
        deserialize_race_event;
    let store = Arc::new(
        PostgreSQLEventStore::open(store_pool.clone(), deserialize)
            .await
            .expect("the store opens against a freshly migrated, empty database"),
    );

    // --- The race itself, under a NULL tenant: loads `ux_events_identity_systemwide` ---
    let results = race(store.clone(), &test_pool, "systemwide-key", None).await;
    let winners = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(
        winners, 1,
        "NULL-tenant identity is not exempt from uniqueness: exactly one of \
         {RACERS} racers on the same systemwide stream must win. Got: {results:?}"
    );

    // --- Direct-SQL duplicate under NULL tenant: refused with 23505 ---------
    let occurred_at = Utc::now();
    let duplicate = sqlx::query(
        r#"INSERT INTO events (aggregate_type, aggregate_id, tenant_id, version, event_type, payload, created_at)
           VALUES ($1, $2, NULL, 1, 'dup', '{}', $3)"#,
    )
    .bind(AGGREGATE_TYPE)
    .bind("systemwide-key")
    .bind(occurred_at)
    .execute(&store_pool)
    .await;
    match duplicate {
        Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {}
        other => panic!(
            "a direct-SQL duplicate under the identical NULL-tenant identity must be \
             refused by ux_events_identity_systemwide with 23505, proving NULL-tenant \
             identity is not exempt from uniqueness. Got: {other:?}"
        ),
    }

    // --- The identical (aggregate_type, aggregate_id, version) under a concrete
    // tenant must succeed: the tenant-scoped and systemwide partial indexes do
    // not collide with each other. -------------------------------------------
    let concrete_tenant_insert = sqlx::query(
        r#"INSERT INTO events (aggregate_type, aggregate_id, tenant_id, version, event_type, payload, created_at)
           VALUES ($1, $2, $3, 1, 'not-a-collision', '{}', $4)"#,
    )
    .bind(AGGREGATE_TYPE)
    .bind("systemwide-key")
    .bind("tenant-not-systemwide")
    .bind(occurred_at)
    .execute(&store_pool)
    .await;
    assert!(
        concrete_tenant_insert.is_ok(),
        "the identical (aggregate_type, aggregate_id, version) under a concrete \
         tenant must succeed — the systemwide and tenant-scoped partial indexes \
         are disjoint partitions of the table, not one shared uniqueness space. \
         Got: {concrete_tenant_insert:?}"
    );

    // --- Two distinct systemwide streams resolve independently, with no false
    // collision and no false merge -------------------------------------------
    store
        .append(
            AGGREGATE_TYPE,
            "systemwide-alpha",
            None,
            0,
            vec![race_event("alpha-1")],
        )
        .await
        .expect("a fresh systemwide stream accepts its first event");
    store
        .append(
            AGGREGATE_TYPE,
            "systemwide-beta",
            None,
            0,
            vec![race_event("beta-1")],
        )
        .await
        .expect("a second, distinct systemwide stream accepts its first event, \
                 independently of the first");

    let alpha = store
        .load(AGGREGATE_TYPE, "systemwide-alpha", None)
        .await
        .expect("alpha's stream is readable back");
    let beta = store
        .load(AGGREGATE_TYPE, "systemwide-beta", None)
        .await
        .expect("beta's stream is readable back");

    assert_eq!(alpha.len(), 1, "alpha's stream carries only its own event");
    assert_eq!(beta.len(), 1, "beta's stream carries only its own event");
    assert_eq!(
        alpha[0].event.kind, "alpha-1",
        "alpha's stream is not contaminated by beta's write"
    );
    assert_eq!(
        beta[0].event.kind, "beta-1",
        "beta's stream is not contaminated by alpha's write"
    );

    db.close().await;
}
