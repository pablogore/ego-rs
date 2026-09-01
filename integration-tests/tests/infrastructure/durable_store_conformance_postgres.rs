//! **Guarantee:** `PostgreSQLEventStore` and `PostgresOperationReservationStore`
//! satisfy the identical `EventStore`/`OperationReservationStore` conformance
//! definitions the in-memory adapters satisfy — the same shared harnesses
//! (`ego_testkit::assert_event_store_conformance`,
//! `ego_testkit::assert_reservation_store_conformance`), never a re-derived or
//! weakened assertion set.
//!
//! **Layers traversed:** `EventStore<E>` / `OperationReservationStore` (the
//! ports) → `PostgreSQLEventStore` / `PostgresOperationReservationStore` →
//! real SQL, against a real PostgreSQL with real migrations.
//!
//! # Why in-process cannot show this
//!
//! Included in what the event-store half proves: a staged, uncommitted append
//! on `PostgreSQLEventStore`'s held transaction is invisible to `store.load()`
//! issued from a distinct pooled connection, and a unit of work dropped without
//! commit persists nothing (IS-6, demonstrated by this same run per D-4 —
//! deliberately no separate test or ledger row for it). No in-memory double has
//! a real transaction, a real second pooled connection, or real `READ
//! COMMITTED` cross-connection visibility — a staging map cannot misrepresent
//! isolation it never had to implement, so nothing hermetic can stand in for
//! this.
//!
//! The reservation-store half proves durable fencing/lease conformance under
//! real conditional `UPDATE`s: the harness's fencing/CAS assertions need a real
//! row and a real conditional comparison, which a scripted store cannot
//! misrepresent in the way this suite is built to catch.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use ego_domain::persistence::PersistenceError;
use ego_domain::event::DomainEvent;
use ego_integration_tests::isolated_database;
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use ego_persistence::postgres::PostgreSQLEventStore;
use ego_testkit::{assert_event_store_conformance, assert_reservation_store_conformance, TestClock};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// A local fixture, not a re-derived contract — `crates/infrastructure`'s copy
/// is private to another crate's test target (AD-4).
#[derive(Debug, Clone)]
struct ConformanceEvent {
    kind: String,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

impl DomainEvent for ConformanceEvent {
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

async fn connect(url: &str, max: u32) -> PgPool {
    PgPoolOptions::new()
        .max_connections(max)
        .connect(url)
        .await
        .expect("the container accepts connections")
}

/// The instant every reservation scenario starts from. Fixed, so a failure is
/// reproducible — mirrors the private `epoch()` the harness itself uses,
/// which is not exported.
fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

#[tokio::test]
async fn postgres_event_store_conformance() {
    // `max_connections >= 2` is load-bearing (AD-4): with a pool of 1,
    // `load()` would starve waiting on the open unit of work's held
    // connection and fail as a pool timeout, not an isolation failure.
    let db = isolated_database().await;
    let pool = connect(db.url(), 4).await;

    let deserialize: fn(
        &str,
        serde_json::Value,
        DateTime<Utc>,
    ) -> Result<ConformanceEvent, PersistenceError> = |kind, payload, occurred_at| {
        Ok(ConformanceEvent {
            kind: kind.to_string(),
            payload,
            occurred_at,
        })
    };

    let mut store = PostgreSQLEventStore::open(pool, deserialize)
        .await
        .expect("the store opens against a freshly migrated, empty database");

    assert_event_store_conformance(&mut store, |kind| ConformanceEvent {
        kind: kind.to_string(),
        payload: serde_json::json!({ "kind": kind }),
        occurred_at: Utc::now(),
    })
    .await;

    db.close().await;
}

#[tokio::test]
async fn postgres_reservation_store_conformance() {
    let db = isolated_database().await;
    let pool = connect(db.url(), 4).await;
    let pool = &pool;

    // `TRUNCATE` is the reset the harness's `fresh()` calls need — the store
    // owns exactly one table, so truncating it is sufficient and cheap
    // (AD-4; a fresh isolated database per call was rejected there as many
    // serialized `CREATE DATABASE`s for no added isolation).
    assert_reservation_store_conformance(|| async move {
        sqlx::query("TRUNCATE operation_reservations")
            .execute(pool)
            .await
            .expect("truncating the reservations table between scenarios must succeed");
        let clock = Arc::new(TestClock::new(epoch()));
        (
            PostgresOperationReservationStore::new(pool.clone(), clock.clone()),
            clock,
        )
    })
    .await;

    db.close().await;
}
