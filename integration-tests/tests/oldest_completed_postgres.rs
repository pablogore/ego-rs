//! **Guarantee:** `oldest_completed` reports the *earliest* `completed_at` still
//! held, `Empty` when nothing is completed, and never `Unsupported`.
//!
//! **Layers traversed:** `OperationReservationStore::oldest_completed` →
//! `PostgresOperationReservationStore` → the `MIN(completed_at)` aggregate,
//! against a real PostgreSQL with real migrations.
//!
//! # Why this needs a container
//!
//! The in-memory store's own tests pin the same contract, and they cannot pin
//! this one. The two implementations answer the question by different means —
//! `Iterator::min` over a map versus a SQL aggregate — so a `MIN` written as
//! `MAX`, or a predicate that admits in-progress rows, is invisible to every
//! test that does not execute the statement. That is a one-token error that ships
//! silently and reports a backlog age that is wrong in the reassuring direction.
//!
//! The retention worker's gauge tests are equally blind to it: they drive the
//! in-memory store, so they prove the worker reads and converts an answer
//! correctly, never that this adapter produces the right one.
//!
//! # `Empty` is asserted, and it is not `Unsupported`
//!
//! Both make the gauge emit nothing, so no test of the worker can separate them.
//! Here the difference is directly observable, which is the point of the port
//! carrying three states rather than an `Option`.
//!
//! Run: `cargo test --manifest-path integration-tests/Cargo.toml`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use ego_domain::operation::{OldestCompleted, OperationReservationStore};
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

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the container accepts connections")
}

/// Inserts one completed reservation with an exact `completed_at`.
///
/// SQL rather than `reserve` + `complete` because the scenario needs a specific
/// timestamp per row, and how the row was produced is irrelevant to which one the
/// aggregate picks.
async fn completed_row(pool: &PgPool, key: &str, completed_at: DateTime<Utc>) {
    sqlx::query(
        r#"INSERT INTO operation_reservations
               (tenant_id, operation_key, fingerprint, owner_id, fencing_token,
                lease_until, state, completed_at, response)
           VALUES (NULL, $1, $2, 'owner-a', 1, $3, 'completed', $4, $5)"#,
    )
    .bind(key)
    .bind("f".repeat(64))
    .bind(t0() + Duration::seconds(3_600))
    .bind(completed_at)
    .bind(b"stored".to_vec())
    .execute(pool)
    .await
    .expect("the row inserts");
}

/// Inserts one reservation that is still running.
///
/// It has no `completed_at` and must not influence the answer however long it has
/// been open: the backlog this gauge describes is completed work awaiting
/// retention, not work in flight.
async fn in_progress_row(pool: &PgPool, key: &str, lease_until: DateTime<Utc>) {
    sqlx::query(
        r#"INSERT INTO operation_reservations
               (tenant_id, operation_key, fingerprint, owner_id, fencing_token,
                lease_until, state)
           VALUES (NULL, $1, $2, 'owner-b', 1, $3, 'in_progress')"#,
    )
    .bind(key)
    .bind("e".repeat(64))
    .bind(lease_until)
    .execute(pool)
    .await
    .expect("the row inserts");
}

async fn fresh_store() -> (PostgresOperationReservationStore, PgPool, impl Sized) {
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
    let pool = connect(&url).await;
    migrations::run(&pool).await.expect("the migrations apply");
    let store = PostgresOperationReservationStore::new(pool.clone(), Arc::new(SystemClock));
    (store, pool, container)
}

/// An empty table answers `Empty` — a real answer, not `Unsupported`.
#[tokio::test]
async fn an_empty_table_answers_empty_and_never_unsupported() {
    let (store, _pool, _container) = fresh_store().await;

    assert_eq!(
        store.oldest_completed().await,
        Ok(OldestCompleted::Empty),
        "this adapter supports the query, so an empty table is a statement about the \
         backlog — reporting Unsupported would claim it cannot look"
    );
}

/// The answer is the earliest `completed_at`, not the latest.
///
/// Three rows spread over an hour, inserted out of order so the answer cannot come
/// from insertion order, physical order, or the row the scan happens to reach
/// first. `MAX` instead of `MIN` returns `t0 + 3600s` and fails here.
#[tokio::test]
async fn the_answer_is_the_earliest_completion_not_the_latest() {
    let (store, pool, _container) = fresh_store().await;

    completed_row(&pool, "op-middle", t0() + Duration::seconds(1_800)).await;
    completed_row(&pool, "op-newest", t0() + Duration::seconds(3_600)).await;
    completed_row(&pool, "op-oldest", t0()).await;

    assert_eq!(
        store.oldest_completed().await,
        Ok(OldestCompleted::At(t0())),
        "three completions exist and the aggregate must return the earliest"
    );
}

/// An in-progress reservation is not backlog, however old.
///
/// Its row carries no `completed_at`, and the `state = 'completed'` predicate is
/// what keeps it out — the same predicate the purge uses, for the same reason.
#[tokio::test]
async fn an_in_progress_reservation_is_never_the_oldest_completion() {
    let (store, pool, _container) = fresh_store().await;

    in_progress_row(&pool, "op-running", t0() + Duration::seconds(3_600)).await;

    assert_eq!(
        store.oldest_completed().await,
        Ok(OldestCompleted::Empty),
        "nothing has completed, so there is no oldest completion — an in-progress \
         reservation is work in flight, not a retention backlog"
    );

    completed_row(&pool, "op-done", t0() + Duration::seconds(600)).await;

    assert_eq!(
        store.oldest_completed().await,
        Ok(OldestCompleted::At(t0() + Duration::seconds(600))),
        "the completed row is the answer; the in-progress one is still excluded"
    );
}

/// After a purge removes the oldest rows, the answer moves forward.
///
/// This is the sequence the retention worker actually produces, and it is what
/// makes querying after the purge meaningful: asked before, the answer would still
/// name a row the batch was about to delete.
#[tokio::test]
async fn the_answer_advances_once_the_oldest_rows_are_purged() {
    let (store, pool, _container) = fresh_store().await;

    completed_row(&pool, "op-a", t0()).await;
    completed_row(&pool, "op-b", t0() + Duration::seconds(600)).await;
    completed_row(&pool, "op-c", t0() + Duration::seconds(1_200)).await;

    assert_eq!(
        store.oldest_completed().await,
        Ok(OldestCompleted::At(t0())),
        "before the purge the oldest row is the earliest inserted"
    );

    let removed = store
        .purge_completed_before(t0() + Duration::seconds(900), 10)
        .await
        .expect("the purge runs");
    assert_eq!(removed, 2, "op-a and op-b are older than the cutoff");

    assert_eq!(
        store.oldest_completed().await,
        Ok(OldestCompleted::At(t0() + Duration::seconds(1_200))),
        "the remaining backlog is what survived, so the age an operator reads \
         describes rows that are still there"
    );
}

/// The `state = 'completed'` predicate is defensive, and the schema is why.
///
/// Removing it from the aggregate changes nothing observable, and this test is
/// why that is a *proof of equivalence* rather than a gap in the ones above.
/// `MIN` ignores NULLs, and the table's CHECK ties `completed_at` to `state`:
///
/// ```sql
/// (state = 'in_progress' AND completed_at IS NULL     AND response IS NULL)
///   OR (state = 'completed' AND completed_at IS NOT NULL AND response IS NOT NULL)
/// ```
///
/// So the only row that could make the predicate matter — one that is not
/// completed yet carries a `completed_at` — is a row the database refuses to
/// store. Asserted here rather than reasoned about, because the predicate's whole
/// justification rests on a constraint living in a different file, and a later
/// migration that relaxed the CHECK would silently make the aggregate depend on a
/// clause someone could then delete as redundant.
#[tokio::test]
async fn the_schema_forbids_the_row_that_would_make_the_state_predicate_matter() {
    let (_store, pool, _container) = fresh_store().await;

    let refused = sqlx::query(
        r#"INSERT INTO operation_reservations
               (tenant_id, operation_key, fingerprint, owner_id, fencing_token,
                lease_until, state, completed_at, response)
           VALUES (NULL, 'op-impossible', $1, 'owner-c', 1, $2, 'in_progress', $3, $4)"#,
    )
    .bind("d".repeat(64))
    .bind(t0() + Duration::seconds(3_600))
    .bind(t0() - Duration::seconds(3_600))
    .bind(b"stored".to_vec())
    .execute(&pool)
    .await;

    let error = refused.expect_err(
        "an in-progress row carrying a completed_at must be refused: it is the only \
         shape that could distinguish the aggregate's state predicate from its absence",
    );
    assert!(
        error.to_string().contains("operation_reservations")
            || error.to_string().to_lowercase().contains("check"),
        "the refusal must come from the table's own CHECK, not from something \
         incidental: {error}"
    );
}
