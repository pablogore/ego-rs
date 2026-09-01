//! **Guarantee:** six real contenders racing one expired lease leave exactly
//! one `TakenOver` winner, and the fencing token advances by exactly one —
//! never by the contender count.
//!
//! **Layers traversed:** `OperationReservationStore` (the port) →
//! `PostgresOperationReservationStore` → the takeover `UPDATE`'s
//! `fencing_token = $N AND lease_until <= $N` predicates, against a real
//! PostgreSQL with real migrations, with a second transaction genuinely
//! holding the row lock.
//!
//! # Why this needs six real contenders, not two
//!
//! `fencing_window_postgres.rs` already proves the lease-recheck predicate
//! with one contender against a renewed lease. This test proves a different
//! property that one contender cannot: that when N callers' `UPDATE`s all
//! genuinely block on the same row lock and are released together, real
//! PostgreSQL row-level locking serializes them so that only the first to
//! acquire the lock finds `fencing_token = $6` still true — every later
//! `UPDATE` re-reads a row whose token has already moved and affects zero
//! rows. A scripted or in-memory double has no row lock to serialize contenders
//! on, so it cannot misrepresent this the way six statements genuinely queued
//! behind one lock can be trusted to prove it.
//!
//! # Determinism without sleeping
//!
//! Six contenders could resolve serially by scheduling accident, with every
//! assertion below still passing while proving nothing — the whole point is
//! that they must be *forced* to queue behind one lock at the same instant.
//! `wait_until_blocked(observer, "%UPDATE operation_reservations%", 6)` polls
//! under an explicit deadline and fails the test outright if six backends are
//! never observed blocked on that statement before the holder releases (AD-3,
//! T-00.1).
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;

use chrono::{Duration, Utc};
use ego_domain::operation::{
    OperationFingerprint, OperationKey, OperationReservationStore, OwnerId, ReservationOutcome,
    ReserveRequest,
};
use ego_integration_tests::{isolated_database, wait_until_blocked};
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use ego_testkit::TestClock;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

const KEY: &str = "op-lease-contention-under-test";
const CONTENDERS: usize = 6;

fn request(owner: &str, lease_until: chrono::DateTime<Utc>) -> ReserveRequest {
    ReserveRequest {
        tenant: None,
        operation_key: OperationKey::parse(KEY).expect("a non-empty key parses"),
        fingerprint: OperationFingerprint::new("f".repeat(64)),
        owner_id: OwnerId::new(owner),
        lease_until,
    }
}

async fn connect(url: &str, max: u32) -> PgPool {
    PgPoolOptions::new()
        .max_connections(max)
        .connect(url)
        .await
        .expect("the container accepts connections")
}

#[tokio::test]
async fn six_contenders_racing_one_expired_lease_leave_exactly_one_winner() {
    // This test's own database, cloned from the run's already-migrated
    // template. No container starts here and no migration runs; the guard is
    // held for the test's life because dropping it releases the
    // connection-budget permit.
    let db = isolated_database().await;
    let url = db.url().to_string();
    // The store's pool must hold at least one connection per contender plus
    // headroom, or a contender would be starved on the pool rather than
    // genuinely blocked on the row lock this test forces open; design pins 8.
    let store_pool = connect(&url, 8).await;
    // Test-owned, separate from the store's pool, so the holder transaction
    // and the lock observation can never be starved by the contenders'
    // own connection use.
    let test_pool = connect(&url, 4).await;

    let t0 = Utc::now();
    let clock = Arc::new(TestClock::new(t0));
    let store = PostgresOperationReservationStore::new(store_pool.clone(), clock.clone());

    // --- Owner A holds the reservation, and its lease lapses -----------------
    let a_lease = match store
        .reserve(request("owner-a", t0 + Duration::seconds(30)))
        .await
        .expect("the store answers")
    {
        ReservationOutcome::Fresh(lease) => lease,
        other => panic!("a first reservation must be Fresh, got {other:?}"),
    };
    let a_token = a_lease.fencing_token.value() as i64;

    // Moved past `lease_until`, not slept through. From here every contender
    // will read this row as expired and therefore takeable.
    clock.advance(Duration::seconds(31));

    // --- Force the window open ----------------------------------------------
    //
    // A transaction takes the row lock and holds it. Each contender's read is
    // a plain `SELECT`, which MVCC lets through unblocked, so all six will
    // still observe the expired lease — and then block on this lock when they
    // try to write.
    let mut holder = test_pool.begin().await.expect("a transaction begins");
    let locked: (String,) = sqlx::query_as(
        "SELECT owner_id FROM operation_reservations \
         WHERE operation_key = $1 FOR UPDATE",
    )
    .bind(KEY)
    .fetch_one(&mut *holder)
    .await
    .expect("the row is locked");
    assert_eq!(locked.0, "owner-a", "the lock is held over A's reservation");

    // --- Six contenders start, each reads the expired lease, and blocks ------
    let contenders: Vec<_> = (0..CONTENDERS)
        .map(|i| {
            let store = PostgresOperationReservationStore::new(store_pool.clone(), clock.clone());
            let owner = format!("owner-b-{i}");
            tokio::spawn(async move { store.reserve(request(&owner, t0 + Duration::seconds(91))).await })
        })
        .collect();

    // Proves all six contenders reached the point of blocking on the takeover
    // UPDATE before the holder releases — not that six resolved serially by
    // accident (AD-3's own stated rationale for this poll's existence).
    wait_until_blocked(&test_pool, "%UPDATE operation_reservations%", CONTENDERS).await;

    // --- Release the lock, changing nothing -----------------------------------
    //
    // The holder only ever read the row; committing releases the lock and lets
    // the six queued `UPDATE`s run one at a time, each seeing whatever the
    // previous one left behind.
    holder.commit().await.expect("the holder commits");

    let mut outcomes = Vec::with_capacity(CONTENDERS);
    for contender in contenders {
        outcomes.push(
            contender
                .await
                .expect("the contender task completes")
                .expect("the store answers"),
        );
    }

    let winners: Vec<_> = outcomes
        .iter()
        .filter(|o| matches!(o, ReservationOutcome::TakenOver(_)))
        .collect();
    let losers: Vec<_> = outcomes
        .iter()
        .filter(|o| matches!(o, ReservationOutcome::OtherInProgress))
        .collect();

    assert_eq!(
        winners.len(),
        1,
        "exactly one of six contenders racing the same expired lease must win the \
         takeover; the row lock genuinely serializes them, and only the first to \
         acquire it can still find the fencing token it read. Got outcomes: \
         {outcomes:?}"
    );
    assert_eq!(
        losers.len(),
        CONTENDERS - 1,
        "every contender that lost the takeover must re-read the row after the \
         winner's write and, since none of them share the winner's owner id, \
         report OtherInProgress rather than any other outcome. Got outcomes: \
         {outcomes:?}"
    );

    let winning_lease = match &winners[0] {
        ReservationOutcome::TakenOver(lease) => lease,
        _ => unreachable!("filtered to TakenOver above"),
    };
    assert_eq!(
        winning_lease.fencing_token.value() as i64,
        a_token + 1,
        "the token must advance by exactly one across all six contenders, never \
         by the contender count — each of the five later UPDATEs affects zero \
         rows and mints no token of its own"
    );

    // --- and the row agrees with the single declared winner ------------------
    let (owner, token): (String, i64) = sqlx::query_as(
        "SELECT owner_id, fencing_token FROM operation_reservations WHERE operation_key = $1",
    )
    .bind(KEY)
    .fetch_one(&store_pool)
    .await
    .expect("exactly one reservation row for this key");
    assert_eq!(
        owner, winning_lease.owner_id.as_str(),
        "the row's owner must match the single TakenOver result's owner"
    );
    assert_eq!(
        token, a_token + 1,
        "the row's stored token must match the winner's, confirming no other \
         contender's UPDATE landed"
    );

    // The database, and every pool taken from it, released here rather than
    // left for the runner's container teardown — the semaphore counts live
    // databases, and that is only true if they are actually dropped.
    db.close().await;
}
