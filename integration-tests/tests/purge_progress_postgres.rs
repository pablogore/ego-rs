//! **Guarantee:** a purge worker whose batch could be filled from unlocked
//! eligible rows fills it, instead of waiting behind rows another worker is
//! already holding.
//!
//! **Layers traversed:** `OperationReservationStore::purge_completed_before` →
//! `PostgresOperationReservationStore` → the batched `DELETE` and its row
//! selection, against a real PostgreSQL with real migrations, with a second
//! transaction genuinely holding two of the eligible rows.
//!
//! # This is a progress guarantee, not a correctness one
//!
//! Worth being exact, because the surrounding task text used to call this
//! "concurrency safety" and that would have been false.
//!
//! The pre-existing query was **not** unsafe. Measured directly: two workers
//! cannot delete the same row twice, because the second `DELETE` blocks on the row
//! lock, re-evaluates under `READ COMMITTED`, finds the row gone and removes zero.
//! PostgreSQL guarantees that, not our SQL. So "no row is deleted twice" and "the
//! counts sum to the rows actually removed" hold with or without the change here —
//! which is exactly why this file does **not** assert them as its evidence. An
//! assertion that cannot distinguish the two implementations is not evidence about
//! either.
//!
//! What the pre-existing query did do is **stall**. Its selection subquery takes
//! whatever rows the scan hands it, so a worker could pick rows another transaction
//! held and then wait on them while unlocked eligible rows sat untouched. Measured
//! before the fix, with two of four eligible rows locked and a batch of two:
//!
//! ```text
//! ERROR: canceling statement due to statement timeout
//! CONTEXT: while deleting tuple (0,2) in relation "operation_reservations"
//! ```
//!
//! That is head-of-line blocking. It is **not** a deadlock, and this file does not
//! claim one: no circular wait was reproduced, and "it could happen" is not a
//! property worth asserting.
//!
//! # Why the window is forced rather than raced
//!
//! Running two purge workers concurrently and hoping the scheduler produces an
//! overlap would make this test a lottery. Instead a transaction the test owns
//! locks a known subset with `SELECT … FOR UPDATE` and holds it, so the contended
//! state is arranged rather than waited for. The purge then runs under a deadline:
//! finishing inside it is the guarantee, and exceeding it is the failure the
//! pre-existing query produced.
//!
//! Run: `cargo test --manifest-path integration-tests/Cargo.toml`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, TimeZone, Utc};
use ego_domain::operation::OperationReservationStore;
use ego_domain::time::SystemClock;
use ego_persistence::postgres::migrations;
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

/// Fixed, so a failure is reproducible and nothing depends on when the suite ran.
fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

/// The cutoff. Four rows complete before it; one completes after.
fn cutoff() -> DateTime<Utc> {
    t0() + Duration::seconds(10)
}

/// How long the purge is given to fill its batch from unlocked rows.
///
/// Generous on purpose: exceeding it means the worker waited on rows it could have
/// skipped, which is the defect. A tighter bound would risk blaming a slow machine
/// for a stall, and a looser one would let a real stall pass as slowness.
const PURGE_DEADLINE: StdDuration = StdDuration::from_secs(10);

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the container accepts connections")
}

/// Inserts one completed reservation directly.
///
/// Written as SQL rather than through `reserve` + `complete` because the scenario
/// needs a specific `completed_at` per row and nothing about how the row was
/// produced matters here — only which rows the purge selects.
async fn completed_row(pool: &PgPool, key: &str, completed_at: DateTime<Utc>) {
    sqlx::query(
        r#"INSERT INTO operation_reservations
               (tenant_id, operation_key, fingerprint, owner_id, fencing_token,
                lease_until, state, completed_at, response)
           VALUES (NULL, $1, $2, 'owner-a', 1, $3, 'completed', $4, $5)"#,
    )
    .bind(key)
    .bind("f".repeat(64))
    .bind(cutoff() + Duration::seconds(3_600))
    .bind(completed_at)
    .bind(b"stored".to_vec())
    .execute(pool)
    .await
    .expect("the row inserts");
}

async fn surviving_keys(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT operation_key FROM operation_reservations ORDER BY operation_key")
        .fetch_all(pool)
        .await
        .expect("the keys come back")
}

#[tokio::test]
async fn a_purge_fills_its_batch_from_unlocked_rows_instead_of_waiting() {
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

    // Separate pools: the store must never be starved of a connection by the
    // transaction the test uses to hold the lock.
    let store_pool = connect(&url).await;
    let test_pool = connect(&url).await;
    migrations::run(&store_pool)
        .await
        .expect("the real migrations apply");

    let store = PostgresOperationReservationStore::new(store_pool.clone(), Arc::new(SystemClock));

    // Four eligible, one not.
    for i in 1..=4 {
        completed_row(&store_pool, &format!("op-{i}"), t0() + Duration::seconds(i)).await;
    }
    completed_row(
        &store_pool,
        "op-ineligible",
        cutoff() + Duration::seconds(60),
    )
    .await;

    // --- A transaction holds two of the eligible rows ------------------------
    //
    // `FOR UPDATE` in ascending key order so which rows are held is known rather
    // than incidental: `op-1` and `op-2`.
    let mut holder = test_pool.begin().await.expect("a transaction begins");
    let held: Vec<String> = sqlx::query_scalar(
        "SELECT operation_key FROM operation_reservations \
         WHERE state = 'completed' AND completed_at < $1 \
         ORDER BY operation_key LIMIT 2 FOR UPDATE",
    )
    .bind(cutoff())
    .fetch_all(&mut *holder)
    .await
    .expect("two eligible rows are locked");
    assert_eq!(
        held,
        vec!["op-1".to_string(), "op-2".to_string()],
        "the scenario depends on knowing which rows are held"
    );

    // --- The purge asks for exactly as many rows as are free -----------------
    //
    // Two of the four eligible rows are locked and two are not, and the batch is
    // two. So the batch is satisfiable without touching anything held — and a
    // worker that waits anyway has stalled behind work it could have skipped.
    let removed = tokio::time::timeout(PURGE_DEADLINE, store.purge_completed_before(cutoff(), 2))
        .await
        .unwrap_or_else(|_| {
            panic!(
                "the purge did not finish within {PURGE_DEADLINE:?}. Two eligible rows \
             were unlocked and the batch was two, so it had enough free work to \
             fill it — waiting instead means its selection took rows another \
             transaction held. That is head-of-line blocking, not a deadlock: \
             nothing here forms a circular wait, and the holder releases on its own"
            )
        })
        .expect("the store answers");

    assert_eq!(
        removed, 2,
        "the batch is filled from the unlocked rows, so the count is the batch"
    );

    // --- What survived -------------------------------------------------------
    assert_eq!(
        surviving_keys(&store_pool).await,
        vec![
            "op-1".to_string(),
            "op-2".to_string(),
            "op-ineligible".to_string()
        ],
        "the two held rows are untouched — skipping them must not mean deleting \
         them later in the same call — and the ineligible row was never a \
         candidate"
    );

    // --- Released, the rest drains -------------------------------------------
    //
    // Skipping is deferral, not exclusion. A row passed over because it was busy
    // has to remain purgeable once it is not, or the suite would be admitting a
    // leak in exchange for progress.
    holder.commit().await.expect("the holder commits");

    let removed_after =
        tokio::time::timeout(PURGE_DEADLINE, store.purge_completed_before(cutoff(), 10))
            .await
            .expect("the second purge finishes once nothing is held")
            .expect("the store answers");

    assert_eq!(
        removed_after, 2,
        "the two previously held rows are still eligible and now free, so a later \
         call removes exactly them"
    );
    assert_eq!(
        surviving_keys(&store_pool).await,
        vec!["op-ineligible".to_string()],
        "only the row that was never eligible remains"
    );
}
