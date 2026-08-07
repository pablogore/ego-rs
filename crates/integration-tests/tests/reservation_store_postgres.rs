//! The durable reservation store, judged against the same shared contract as the
//! in-memory one.
//!
//! The scenarios are not written here and are not mirrored here. They live in
//! `ego-testkit`, and both implementations run the identical definitions — which is
//! the whole point of extracting them before this file existed. Four divergences
//! between the two `EventStore` implementations were found by *not* having done
//! that, and lease ownership, expiry and fencing are where a divergence would be a
//! hole in the idempotency guarantee rather than an inconvenience.
//!
//! Each scenario gets a store over a **freshly truncated table**, matching the
//! isolation the in-memory factory gives by allocating a new map. Truncating rather
//! than creating a new schema per scenario keeps one container and one migration run
//! for the whole file.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ego_persistence::postgres::migrations;
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use ego_testkit::{
    assert_lease_mutation_conformance, assert_purge_conformance,
    assert_reservation_store_conformance, assert_reserve_conformance, TestClock,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// Pinned explicitly, matching the framework's declared PostgreSQL 14 floor. Never
/// a floating tag — see `event_store_characterization.rs`.
const POSTGRES_IMAGE_TAG: &str = "14-alpine";

/// One index as `pg_index` yields it: name, uniqueness, the partial predicate, and
/// the ordered column list.
type CatalogRow = (String, bool, Option<String>, Option<Vec<String>>);

/// The instant the shared scenarios start from. Must match the harness's own epoch:
/// the factory hands back a clock, and a clock positioned elsewhere would make the
/// scenarios' absolute instants meaningless.
fn epoch() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

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
        .max_connections(8)
        .connect(&url)
        .await
        .expect("must be able to connect to the freshly started container");

    migrations::run(&pool)
        .await
        .expect("the framework's own migrations must apply cleanly");

    (pool, container)
}

/// Hands the harness a store over an empty table and a clock at the shared epoch.
///
/// `TRUNCATE` rather than `DELETE`: the scenarios care only that the table is empty,
/// and truncation cannot leave a partially-cleared table behind if it fails.
async fn fresh(pool: &PgPool) -> (PostgresOperationReservationStore, Arc<TestClock>) {
    sqlx::query("TRUNCATE operation_reservations")
        .execute(pool)
        .await
        .expect("truncating between scenarios must succeed");
    let clock = Arc::new(TestClock::new(epoch()));
    let store = PostgresOperationReservationStore::new(pool.clone(), clock.clone());
    (store, clock)
}

/// The durable store satisfies the whole shared contract.
///
/// The aggregate rather than the three groups, because that is what a production
/// adapter has to satisfy. The group-level tests below exist so a failure names
/// which part of the contract broke without needing a backtrace.
#[tokio::test(flavor = "multi_thread")]
async fn the_postgres_reservation_store_conforms() {
    let (pool, _container) = start_pool().await;
    assert_reservation_store_conformance(|| fresh(&pool)).await;
}

/// `reserve`'s six outcomes and the atomic takeover, in isolation.
#[tokio::test(flavor = "multi_thread")]
async fn the_postgres_reservation_store_conforms_on_reserve() {
    let (pool, _container) = start_pool().await;
    assert_reserve_conformance(|| fresh(&pool)).await;
}

/// The fence-verifying mutators, in isolation.
#[tokio::test(flavor = "multi_thread")]
async fn the_postgres_reservation_store_conforms_on_lease_mutation() {
    let (pool, _container) = start_pool().await;
    assert_lease_mutation_conformance(|| fresh(&pool)).await;
}

/// Purge eligibility, the batch limit, the returned count and drainage, in isolation.
#[tokio::test(flavor = "multi_thread")]
async fn the_postgres_reservation_store_conforms_on_purge() {
    let (pool, _container) = start_pool().await;
    assert_purge_conformance(|| fresh(&pool)).await;
}

/// The identity indexes are the AD-1 partial pair, read from the catalog.
///
/// The behavioural scenarios above would keep passing if someone replaced the pair
/// with a single conventional `UNIQUE`, because none of them creates two systemwide
/// reservations under one key concurrently — the store's own logic returns an
/// outcome before the index is consulted. This asserts the shape that stops the
/// database from admitting such a row at all.
#[tokio::test(flavor = "multi_thread")]
async fn the_reservation_identity_is_a_complementary_partial_pair() {
    let (pool, _container) = start_pool().await;

    let rows: Vec<CatalogRow> = sqlx::query_as(
        "SELECT c.relname, \
                i.indisunique, \
                pg_get_expr(i.indpred, i.indrelid), \
                (SELECT array_agg(a.attname ORDER BY k.ord) \
                   FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) \
                   JOIN pg_attribute a \
                     ON a.attrelid = i.indrelid AND a.attnum = k.attnum) \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indexrelid \
         WHERE i.indrelid = 'operation_reservations'::regclass \
         ORDER BY c.relname",
    )
    .fetch_all(&pool)
    .await
    .expect("the index catalog must be queryable");

    let find = |name: &str| {
        rows.iter()
            .find(|(n, _, _, _)| n == name)
            .unwrap_or_else(|| {
                panic!(
                    "missing index {name}; present: {:?}",
                    rows.iter().map(|(n, _, _, _)| n).collect::<Vec<_>>()
                )
            })
    };

    let (_, tenant_unique, tenant_pred, tenant_cols) =
        find("ux_operation_reservations_identity_tenant");
    assert!(*tenant_unique, "the tenant half must be UNIQUE");
    assert_eq!(
        tenant_pred.as_deref(),
        Some("(tenant_id IS NOT NULL)"),
        "the tenant half must be partial over a non-null tenant"
    );
    assert_eq!(
        tenant_cols.as_deref(),
        Some(["tenant_id".to_string(), "operation_key".to_string()].as_slice()),
        "the tenant half must cover the tenant and the key, in that order"
    );

    let (_, sys_unique, sys_pred, sys_cols) = find("ux_operation_reservations_identity_systemwide");
    assert!(*sys_unique, "the systemwide half must be UNIQUE");
    assert_eq!(
        sys_pred.as_deref(),
        Some("(tenant_id IS NULL)"),
        "the systemwide half must be partial over a null tenant — this is what makes two \
         systemwide reservations under one key collide, which a conventional UNIQUE would \
         permit without limit"
    );
    assert_eq!(
        sys_cols.as_deref(),
        Some(["operation_key".to_string()].as_slice()),
        "the systemwide half covers the key alone: its predicate already fixes the tenant to \
         NULL, so including that column would index a constant"
    );
}

/// The table refuses a completed row with no completion timestamp, and an
/// in-progress row that carries one.
///
/// Purge eligibility is measured from `completed_at`, so a completed row without one
/// would be unpurgeable forever and an in-progress row with one would be purgeable
/// while still held. The store never writes either shape; this asserts the database
/// would refuse them anyway, which is what makes the invariant hold against anything
/// that writes to the table.
#[tokio::test(flavor = "multi_thread")]
async fn the_table_refuses_an_inconsistent_completion() {
    let (pool, _container) = start_pool().await;

    let completed_without_timestamp = sqlx::query(
        "INSERT INTO operation_reservations \
           (tenant_id, operation_key, fingerprint, owner_id, fencing_token, lease_until, state) \
         VALUES (NULL, 'op-bad-1', 'fp', 'owner', 1, NOW(), 'completed')",
    )
    .execute(&pool)
    .await;
    assert!(
        completed_without_timestamp.is_err(),
        "a completed reservation with no completed_at must be refused"
    );

    let in_progress_with_timestamp = sqlx::query(
        "INSERT INTO operation_reservations \
           (tenant_id, operation_key, fingerprint, owner_id, fencing_token, lease_until, \
            state, completed_at, response) \
         VALUES (NULL, 'op-bad-2', 'fp', 'owner', 1, NOW(), 'in_progress', NOW(), '\\x00')",
    )
    .execute(&pool)
    .await;
    assert!(
        in_progress_with_timestamp.is_err(),
        "an in-progress reservation carrying a completion must be refused"
    );

    let unknown_state = sqlx::query(
        "INSERT INTO operation_reservations \
           (tenant_id, operation_key, fingerprint, owner_id, fencing_token, lease_until, state) \
         VALUES (NULL, 'op-bad-3', 'fp', 'owner', 1, NOW(), 'abandoned')",
    )
    .execute(&pool)
    .await;
    assert!(
        unknown_state.is_err(),
        "a state outside the two the contract defines must be refused"
    );
}

/// Racing takeovers of one expired lease: exactly one wins, and the winner's token
/// is strictly greater than the one it displaced.
///
/// This is the property the shared scenarios cannot reach. Every one of them is
/// sequential, so the store's Rust-side expiry check decides whether a takeover is
/// even attempted, and the `UPDATE`'s own `lease_until <= $N` and `fencing_token =
/// $N` guards are never the thing that rejects anything. Neutralising either of them
/// leaves the whole conformance suite green — verified, not assumed — which is why
/// this test exists rather than being folded into the harness.
///
/// What the guards actually buy is the window between the read and the write. Under
/// contention two callers both read the same expired row and both compute the same
/// next token; the `fencing_token = $N` predicate is what makes exactly one `UPDATE`
/// match, and the loser observes the winner rather than overwriting it.
///
/// The assertion is deterministic despite the race: it does not care which caller
/// wins, only that exactly one does, that the survivor's token advanced by exactly
/// one, and that the loser was told it is not the owner.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_takeovers_of_one_expired_lease_produce_exactly_one_winner() {
    use chrono::Duration;
    use ego_domain::operation::{
        OperationFingerprint, OperationKey, OperationReservationStore, OwnerId, ReservationOutcome,
        ReserveRequest,
    };
    use ego_domain::Clock;

    let (pool, _container) = start_pool().await;
    let (store, clock) = fresh(&pool).await;
    let store = Arc::new(store);

    let key = OperationKey::parse("op-contested-takeover").expect("valid key");
    let req = |owner: &str, lease_until| ReserveRequest {
        tenant: None,
        operation_key: key.clone(),
        fingerprint: OperationFingerprint::new("fp-1"),
        owner_id: OwnerId::new(owner),
        lease_until,
    };

    let original = store
        .reserve(req("owner-original", clock.now() + Duration::seconds(30)))
        .await
        .expect("the first reservation must succeed");
    let displaced_token = match original {
        ReservationOutcome::Fresh(lease) => lease.fencing_token,
        other => panic!("expected Fresh, got {other:?}"),
    };

    // The lease lapses. Every contender below is now entitled to try.
    clock.advance(Duration::seconds(31));
    let lease_until = clock.now() + Duration::seconds(30);

    const CONTENDERS: usize = 6;
    let barrier = Arc::new(tokio::sync::Barrier::new(CONTENDERS));
    let mut handles = Vec::with_capacity(CONTENDERS);
    for i in 0..CONTENDERS {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let request = req(&format!("owner-{i}"), lease_until);
        handles.push(tokio::spawn(async move {
            // Released together, so the takeovers genuinely overlap instead of
            // queueing behind each other's startup cost.
            barrier.wait().await;
            store.reserve(request).await
        }));
    }

    let mut winners = Vec::new();
    let mut losers = 0;
    for handle in handles {
        match handle.await.expect("no contender may panic") {
            Ok(ReservationOutcome::TakenOver(lease)) => winners.push(lease),
            Ok(ReservationOutcome::OtherInProgress) => losers += 1,
            other => panic!(
                "a contender must either take the lease over or be told another owner holds \
                 it, got {other:?}"
            ),
        }
    }

    assert_eq!(
        winners.len(),
        1,
        "exactly one takeover may succeed; {} did",
        winners.len()
    );
    assert_eq!(
        losers,
        CONTENDERS - 1,
        "every contender that did not win must observe the winner"
    );
    assert_eq!(
        winners[0].fencing_token,
        displaced_token
            .next()
            .expect("the sequence is not exhausted"),
        "the winner's token must be exactly one greater than the one it displaced — not two, \
         which is what several takeovers landing in sequence would produce"
    );

    // And the table holds one row, not one per contender.
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM operation_reservations WHERE operation_key = 'op-contested-takeover'",
    )
    .fetch_one(&pool)
    .await
    .expect("counting must succeed");
    assert_eq!(
        rows, 1,
        "the contested reservation must remain a single row"
    );
}

/// A takeover whose `UPDATE` waits on a row lock must re-check the lease it read,
/// not the lease it remembers.
///
/// This is the property the previous test cannot establish. There, six contenders
/// race, but each one re-reads the row before updating, so after the first takeover
/// commits the others see a future `lease_until` and reject in Rust — the `UPDATE`'s
/// own guards never decide anything. Neutralising either of them leaves that test
/// green, which was verified rather than assumed.
///
/// What the guards buy is the window between the read and the write, and it is
/// forced open here rather than raced for:
///
/// 1. The lease lapses, so a takeover is legitimate.
/// 2. Another transaction locks the row with `SELECT ... FOR UPDATE` and holds it.
/// 3. The takeover runs; its `UPDATE` blocks on that lock.
/// 4. The holding transaction **extends the lease** and commits — the row the
///    takeover read is no longer the row that exists.
/// 5. The blocked `UPDATE` re-evaluates its predicate against the committed row.
///    `lease_until <= $N` is now false, so it matches nothing, and the store
///    re-reads and reports the current holder instead of seizing a live lease.
///
/// Without the guard the `UPDATE` would match and the caller would take over a
/// lease that was renewed while it waited — two owners believing they hold it, which
/// is the exact failure the fencing mechanism exists to prevent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_takeover_that_waits_on_a_row_lock_rechecks_the_lease() {
    use chrono::Duration;
    use ego_domain::operation::{
        OperationFingerprint, OperationKey, OperationReservationStore, OwnerId, ReservationOutcome,
        ReserveRequest,
    };
    use ego_domain::Clock;

    let (pool, _container) = start_pool().await;
    let (store, clock) = fresh(&pool).await;
    let store = Arc::new(store);

    let key = OperationKey::parse("op-lock-recheck").expect("valid key");
    let req = |owner: &str, lease_until| ReserveRequest {
        tenant: None,
        operation_key: key.clone(),
        fingerprint: OperationFingerprint::new("fp-1"),
        owner_id: OwnerId::new(owner),
        lease_until,
    };

    store
        .reserve(req("owner-holder", clock.now() + Duration::seconds(30)))
        .await
        .expect("the first reservation must succeed");

    // (1) The lease lapses.
    clock.advance(Duration::seconds(31));
    let now = clock.now();

    // (2) Lock the row and hold it.
    let mut holding = pool
        .begin()
        .await
        .expect("the holding transaction must begin");
    sqlx::query("SELECT id FROM operation_reservations WHERE operation_key = $1 FOR UPDATE")
        .bind(key.as_str())
        .fetch_one(&mut *holding)
        .await
        .expect("locking the row must succeed");

    // (3) The takeover blocks on that lock.
    let taking_over = {
        let store = Arc::clone(&store);
        let request = req("owner-contender", now + Duration::seconds(30));
        tokio::spawn(async move { store.reserve(request).await })
    };
    wait_until_a_statement_is_blocked(&pool).await;

    // (4) The holder extends its own lease and commits.
    sqlx::query("UPDATE operation_reservations SET lease_until = $1 WHERE operation_key = $2")
        .bind(now + Duration::seconds(600))
        .bind(key.as_str())
        .execute(&mut *holding)
        .await
        .expect("extending the lease must succeed");
    holding.commit().await.expect("the holder must commit");

    // (5) The blocked update re-checks and finds the lease alive.
    let outcome = taking_over
        .await
        .expect("the contender task must not panic")
        .expect("the contender's reserve must not error");
    assert_eq!(
        outcome,
        ReservationOutcome::OtherInProgress,
        "a takeover whose update waited must re-check the lease and report the current holder, \
         not seize a lease that was renewed during the wait"
    );

    // The row still belongs to the original holder, with its original token.
    let (owner, token): (String, i64) = sqlx::query_as(
        "SELECT owner_id, fencing_token FROM operation_reservations WHERE operation_key = $1",
    )
    .bind(key.as_str())
    .fetch_one(&pool)
    .await
    .expect("reading the row must succeed");
    assert_eq!(owner, "owner-holder", "the holder must still own the lease");
    assert_eq!(token, 1, "no takeover happened, so no token was minted");
}

/// Polls the catalog until some backend is waiting on a lock it has not been
/// granted, so the caller knows the competing statement is genuinely in flight.
///
/// This is what makes the test above deterministic rather than timing-based. Sleeping
/// instead would pass or fail with machine load, and a race test that sometimes
/// exercises nothing reports success either way.
async fn wait_until_a_statement_is_blocked(pool: &PgPool) {
    for _ in 0..200 {
        let blocked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pg_locks WHERE NOT granted")
            .fetch_one(pool)
            .await
            .expect("the lock catalog must be queryable");
        if blocked > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!(
        "no statement ever blocked on a lock. This test's premise is that the takeover's UPDATE \
         waits for the held row lock, so if nothing blocks then either the lock was not taken or \
         the takeover never reached its UPDATE — both are failures, not flakes"
    );
}

/// A takeover that cannot mint a storable token reports exhaustion and changes
/// nothing.
///
/// The boundary is the **column's**, not the domain counter's, and the two differ.
/// The domain counts in `u64`; `BIGINT` is `i64`. At `i64::MAX` the domain's own
/// increment still succeeds because `u64` has room, so an unchecked conversion would
/// land on `i64::MIN` — a value PostgreSQL accepts, and which is *less* than the
/// token it displaced. The invariant the whole fencing mechanism rests on would be
/// retired silently, at exactly the point the domain believed it had covered.
///
/// The row is seeded directly, because reaching this state through the port would
/// take `i64::MAX` takeovers. That is the same reason the domain exposes a
/// constructor from a raw value.
///
/// Asserting the refusal is only half of it: a call that reports exhaustion must also
/// leave owner, token and lease exactly as they were, because a partial takeover would
/// be worse than a refused one — two callers would believe they hold the lease.
#[tokio::test(flavor = "multi_thread")]
async fn a_takeover_at_the_storable_token_limit_reports_exhaustion_and_changes_nothing() {
    use chrono::Duration;
    use ego_domain::operation::{
        OperationFingerprint, OperationKey, OperationReservationStore, OwnerId, ReservationError,
        ReserveRequest,
    };
    use ego_domain::Clock;

    let (pool, _container) = start_pool().await;
    let (store, clock) = fresh(&pool).await;

    let expired_at = clock.now() - Duration::seconds(1);
    sqlx::query(
        "INSERT INTO operation_reservations \
           (tenant_id, operation_key, fingerprint, owner_id, fencing_token, lease_until, state) \
         VALUES (NULL, 'op-at-the-limit', 'fp-1', 'owner-original', $1, $2, 'in_progress')",
    )
    .bind(i64::MAX)
    .bind(expired_at)
    .execute(&pool)
    .await
    .expect("seeding a reservation at the storable limit must succeed");

    let outcome = store
        .reserve(ReserveRequest {
            tenant: None,
            operation_key: OperationKey::parse("op-at-the-limit").expect("valid key"),
            fingerprint: OperationFingerprint::new("fp-1"),
            owner_id: OwnerId::new("owner-contender"),
            lease_until: clock.now() + Duration::seconds(30),
        })
        .await;

    assert_eq!(
        outcome,
        Err(ReservationError::FencingExhausted),
        "a takeover that cannot mint a storable token must report exhaustion rather than wrap"
    );

    let (owner, token, lease_until): (String, i64, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "SELECT owner_id, fencing_token, lease_until FROM operation_reservations \
         WHERE operation_key = 'op-at-the-limit'",
    )
    .fetch_one(&pool)
    .await
    .expect("the row must still be there");

    assert_eq!(owner, "owner-original", "the owner must be unchanged");
    assert_eq!(token, i64::MAX, "the token must be unchanged");
    assert_eq!(lease_until, expired_at, "the lease must be unchanged");
}

/// The table refuses a non-positive fencing token.
///
/// The adapter never writes one, and no longer reads one either: its stored-token
/// guard rejects anything at or below zero, so a bad row would surface as an error
/// rather than as a token nobody minted. This constraint is the layer *before* that —
/// it stops the row from being written at all, by anything that reaches this table
/// without going through the adapter.
///
/// The two are worth having separately. The constraint keeps the table honest for
/// every writer; the adapter's guard keeps the store honest even against a schema it
/// cannot re-verify on every deployment. Neither makes the other redundant, and the
/// teeth check on the checked conversion showed both firing on the same defect from
/// different sides.
#[tokio::test(flavor = "multi_thread")]
async fn the_table_refuses_a_non_positive_fencing_token() {
    let (pool, _container) = start_pool().await;

    for (label, token) in [("zero", 0i64), ("negative", -1i64), ("i64::MIN", i64::MIN)] {
        let inserted = sqlx::query(
            "INSERT INTO operation_reservations \
               (tenant_id, operation_key, fingerprint, owner_id, fencing_token, lease_until, \
                state) \
             VALUES (NULL, $1, 'fp', 'owner', $2, NOW(), 'in_progress')",
        )
        .bind(format!("op-token-{label}"))
        .bind(token)
        .execute(&pool)
        .await;

        assert!(
            inserted.is_err(),
            "a fencing token of {label} ({token}) must be refused by the table"
        );
    }

    // Triangulation: the constraint rejects non-positive values, not every value.
    sqlx::query(
        "INSERT INTO operation_reservations \
           (tenant_id, operation_key, fingerprint, owner_id, fencing_token, lease_until, state) \
         VALUES (NULL, 'op-token-one', 'fp', 'owner', 1, NOW(), 'in_progress')",
    )
    .execute(&pool)
    .await
    .expect("the first token the sequence mints must be accepted");
}
