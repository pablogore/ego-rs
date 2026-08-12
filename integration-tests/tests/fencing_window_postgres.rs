//! **Guarantee:** a takeover whose `UPDATE` waits on a row lock is judged against
//! the lease that exists when it finally writes, not the lease it read before
//! waiting. A reservation renewed during that wait is not stolen.
//!
//! **Layers traversed:** `OperationReservationStore` (the port) →
//! `PostgresOperationReservationStore` → the takeover `UPDATE` and its
//! `lease_until <= $N` predicate, against a real PostgreSQL with real migrations,
//! with a second transaction genuinely holding the row lock.
//!
//! # Why this is not one of the four end-to-end scenarios
//!
//! It is deliberately a **store-level** test, and that is not a compromise — the
//! evidence *is* precise control of a transaction. The window this exercises
//! exists between two statements inside one `reserve()` call, and forcing it open
//! requires holding `SELECT … FOR UPDATE` on the row from outside while the
//! contender's `UPDATE` blocks. HTTP cannot express that, and dressing it up as an
//! end-to-end test would mean giving up the only mechanism that makes it a test at
//! all.
//!
//! It is still a complete guarantee of the durable protocol. Its boundary is the
//! store rather than the transport, so the suite's ledger counts it separately:
//! four end-to-end scenarios, plus this one PostgreSQL concurrency invariant.
//!
//! # Why it exists at all
//!
//! `reserve()` reads the row, decides the lease has expired, and then issues a
//! conditional `UPDATE`. Those are two statements, so between them another caller
//! can take the reservation over or its owner can renew it. The `UPDATE` therefore
//! re-checks `lease_until <= $N` itself, which means a caller that waited on the
//! row lock is judged against the row that exists rather than the row it
//! remembers.
//!
//! Until this test, nothing in the repository checked that. Two independent
//! sources said so, and both have been updated to point here instead:
//!
//! - `crates/persistence/src/postgres/reservation.rs` stated in its own comment
//!   that the predicate was "currently unguarded by any test here".
//! - `docs/integration-test-backlog.md` named it the highest-value missing
//!   guarantee, and recorded that neutralising the predicate left the whole
//!   conformance suite green.
//!
//! Re-measured here rather than inherited: neutralising the predicate fails this
//! test and leaves the suite's other three tests green, so it really is the only
//! check on the guarantee.
//!
//! The four end-to-end scenarios do not close it, and that was traced rather than
//! assumed: two replicas racing a *fresh* key never reach this code. The winner
//! inserts, the loser's read finds `state = 'in_progress'` with `now` still before
//! `lease_until`, and it returns `OtherInProgress` without evaluating the takeover
//! `UPDATE` at all. So this is a distinct infrastructure risk, which is the
//! condition the README's budget already set for a test beyond the four.
//!
//! # Determinism without sleeping
//!
//! The contender must be *provably* blocked before the lease is renewed, or the
//! test could pass by accident with the renewal landing first. So it polls
//! `pg_stat_activity` for a backend blocked on a lock while running against
//! `operation_reservations`, under an explicit deadline. The short waits inside
//! that loop are a poll interval, not a timeout standing in for a condition — the
//! loop's exit is the condition itself, and its deadline **fails the test** rather
//! than continuing on an unproven assumption.
//!
//! That last property earned its keep immediately: the first version of the poll
//! queried the wrong catalog view and matched nothing, and the hard deadline turned
//! that into a loud failure instead of a green run over a window that was never
//! opened. See `wait_until_contender_is_blocked` for the specifics.
//!
//! Run: `cargo test --manifest-path integration-tests/Cargo.toml`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use chrono::{Duration, Utc};
use ego_domain::operation::{
    OperationFingerprint, OperationKey, OperationReservationStore, OwnerId, ReservationOutcome,
    ReserveRequest,
};
use ego_persistence::postgres::migrations;
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use ego_testkit::TestClock;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

const KEY: &str = "op-fencing-window-under-test";

/// How long the contender is given to reach its blocked `UPDATE`.
///
/// Generous, because exceeding it is a hard failure rather than a slow pass: if
/// the contender never blocks, the window was never forced and the test would be
/// asserting something it did not arrange.
const BLOCK_DEADLINE: StdDuration = StdDuration::from_secs(20);

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

/// Blocks until a backend is waiting on a lock while running a statement against
/// the reservations table, or fails at the deadline.
///
/// # Why this reads `pg_stat_activity` and not `pg_locks.relation`
///
/// The first version of this poll joined `pg_locks` to `pg_class` on
/// `l.relation`, which matched **nothing** — a statement waiting for a *row* lock
/// waits on the holder's transaction id, so its `pg_locks` row has
/// `locktype = 'transactionid'` and a NULL `relation`. The join filtered out
/// precisely the wait it was looking for.
///
/// That mistake surfaced because the deadline below is a hard assertion rather
/// than a timeout the test continues past. A poll that gives up quietly would have
/// let the renewal land first and reported a pass for a window it never opened.
///
/// `wait_event_type = 'Lock'` is the direct statement of "this backend is blocked
/// on a lock", and matching the statement text keeps unrelated cluster activity
/// from satisfying it.
async fn wait_until_contender_is_blocked(observer: &PgPool) {
    let started = Instant::now();
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE wait_event_type = 'Lock' \
               AND state = 'active' \
               AND query ILIKE '%operation_reservations%' \
               AND pid <> pg_backend_pid()",
        )
        .fetch_one(observer)
        .await
        .expect("pg_stat_activity is readable");

        if waiting > 0 {
            return;
        }
        assert!(
            started.elapsed() < BLOCK_DEADLINE,
            "the contender never blocked on the row lock within {BLOCK_DEADLINE:?}, so the \
             read/write window was never forced open and this test would prove nothing"
        );
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
}

/// The row as it stands: who owns it, under which token, until when.
async fn owner_token_lease(pool: &PgPool) -> (String, i64, chrono::DateTime<Utc>) {
    sqlx::query_as(
        "SELECT owner_id, fencing_token, lease_until \
         FROM operation_reservations WHERE operation_key = $1",
    )
    .bind(KEY)
    .fetch_one(pool)
    .await
    .expect("exactly one reservation row for this key")
}

#[tokio::test]
async fn a_takeover_waiting_on_the_row_lock_rechecks_the_lease_it_finds_not_the_one_it_read() {
    let container = Postgres::default()
        .start()
        .await
        .expect("a PostgreSQL container starts");
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.expect("a host"),
        container
            .get_host_port_ipv4(5432)
            .await
            .expect("the mapped port"),
    );

    // Two pools on purpose. The store owns one; the test drives the blocking
    // transaction and the lock observation through another, so the contender can
    // never be starved of a connection by the test's own bookkeeping.
    let store_pool = connect(&url, 4).await;
    let test_pool = connect(&url, 4).await;
    migrations::run(&store_pool)
        .await
        .expect("the real migrations apply");

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

    // Moved past `lease_until`, not slept through. From here the store will read
    // this row as expired and therefore takeable.
    clock.advance(Duration::seconds(31));

    // --- Force the window open ----------------------------------------------
    //
    // A transaction takes the row lock and holds it. The contender's read is a
    // plain `SELECT`, which MVCC lets through unblocked, so it will still observe
    // the expired lease — and then block on this lock when it tries to write.
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

    // --- The contender starts, reads an expired lease, and blocks ------------
    let contender = tokio::spawn({
        let store = PostgresOperationReservationStore::new(store_pool.clone(), clock.clone());
        async move {
            store
                .reserve(request("owner-b", Utc::now() + Duration::seconds(60)))
                .await
        }
    });

    wait_until_contender_is_blocked(&test_pool).await;

    // --- While it waits, the lease is renewed into the future ----------------
    //
    // This is the event the predicate exists for. The contender has already
    // decided, from its own read, that the lease had expired. That decision is now
    // stale, and only the `UPDATE`'s own re-check can notice.
    //
    // Done through the transaction that holds the lock, which is what makes the
    // ordering airtight: the renewal is committed before the contender's `UPDATE`
    // can acquire the row.
    let renewed_until = t0 + Duration::seconds(300);
    let renewed =
        sqlx::query("UPDATE operation_reservations SET lease_until = $1 WHERE operation_key = $2")
            .bind(renewed_until)
            .bind(KEY)
            .execute(&mut *holder)
            .await
            .expect("the renewal applies");
    assert_eq!(
        renewed.rows_affected(),
        1,
        "the renewal must actually land, or the contender would be re-checking the \
         same expired lease it read and the test would pass for the wrong reason"
    );
    holder.commit().await.expect("the holder commits");

    // --- What the contender concludes once it gets the row -------------------
    let outcome = contender
        .await
        .expect("the contender task completes")
        .expect("the store answers");

    assert!(
        matches!(outcome, ReservationOutcome::OtherInProgress),
        "the takeover must be refused: by the time its UPDATE ran, the lease it \
         read as expired had been renewed and belongs to another owner. Got \
         {outcome:?} — a TakenOver here means the write was judged against the row \
         the caller remembered instead of the row that existed, which is exactly \
         how two replicas end up both believing they hold the same operation"
    );

    // --- and the row is untouched by the refused takeover --------------------
    let (owner, token, lease_until) = owner_token_lease(&store_pool).await;
    assert_eq!(
        owner, "owner-a",
        "the reservation still belongs to the owner whose lease was renewed"
    );
    assert_eq!(
        token, a_token,
        "the token did not advance — a refused takeover mints nothing, so a later \
         legitimate takeover still sees the version it expects"
    );
    assert_eq!(
        lease_until, renewed_until,
        "and the renewed lease stands, unshortened by the attempt"
    );
}
