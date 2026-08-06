//! Real-Postgres tests for the offline `events.aggregate_type` backfill.
//!
//! These tests write directly to the `events` table with raw SQL rather than
//! through `EventStore::append`, so each scenario can construct exactly the
//! stored shape it needs — including shapes the current store's own API
//! would never produce (an ambiguous joined identifier, a genuine duplicate
//! version under today's non-unique index) but that a real, already-running
//! deployment could already contain.

use ego_persistence::postgres::aggregate_type_backfill::{self, AbortReason, BackfillOutcome};
use ego_persistence::postgres::migrations;
use ego_persistence::postgres::PostgreSQLEventStore;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// Pinned explicitly, matching the framework's declared PostgreSQL 14 floor —
/// see `event_store_characterization.rs` for why this is never `latest`.
const POSTGRES_IMAGE_TAG: &str = "14-alpine";

/// Starts a fresh Postgres container, applies the framework's migrations
/// (including the nullable `aggregate_type` column this slice adds), and
/// returns a pool plus the container guard. No fallback: a Docker-less
/// environment fails loudly rather than silently skipping the test.
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
        .connect(&url)
        .await
        .expect("must be able to connect to the freshly started container");

    migrations::run(&pool)
        .await
        .expect("the framework's own migrations must apply cleanly");

    (pool, container)
}

/// Inserts one row directly into `events`, bypassing `EventStore::append` so
/// the row's `aggregate_id` can hold shapes the store's own API would never
/// produce. Returns the inserted row's primary key.
async fn insert_raw_event(
    pool: &PgPool,
    aggregate_id: &str,
    tenant_id: Option<&str>,
    version: i64,
) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO events (aggregate_id, tenant_id, version, event_type, payload) \
         VALUES ($1, $2, $3, 'Probed', '{}'::jsonb) RETURNING id",
    )
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(version)
    .fetch_one(pool)
    .await
    .expect("inserting a raw probe row must succeed")
}

async fn row_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(pool)
        .await
        .expect("counting rows must succeed")
}

async fn row_aggregate_id(pool: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT aggregate_id FROM events WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("reading the row's aggregate_id must succeed")
}

async fn row_aggregate_type(pool: &PgPool, id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT aggregate_type FROM events WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("reading the row's aggregate_type must succeed")
}

async fn aggregate_type_column_is_not_null(pool: &PgPool) -> bool {
    let is_nullable: String = sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns \
         WHERE table_name = 'events' AND column_name = 'aggregate_type'",
    )
    .fetch_one(pool)
    .await
    .expect("the aggregate_type column must exist");
    is_nullable == "NO"
}

/// A minimal event type, present only so the store's generic parameters can be
/// named. This test never appends or loads — it exercises the constructor's
/// refusal — so the type needs no behaviour beyond satisfying the trait.
#[derive(Debug, Clone)]
struct ProbeEvent {
    aggregate: String,
    kind: String,
    payload: serde_json::Value,
    occurred_at: chrono::DateTime<chrono::Utc>,
}

impl ego_domain::event::DomainEvent for ProbeEvent {
    fn aggregate_id(&self) -> &str {
        &self.aggregate
    }

    fn event_type(&self) -> &str {
        &self.kind
    }

    fn payload(&self) -> &serde_json::Value {
        &self.payload
    }

    fn occurred_at(&self) -> &chrono::DateTime<chrono::Utc> {
        &self.occurred_at
    }
}

fn probe_deserialize(
    kind: &str,
    payload: serde_json::Value,
) -> Result<ProbeEvent, ego_domain::persistence::PersistenceError> {
    Ok(ProbeEvent {
        aggregate: String::new(),
        kind: kind.to_string(),
        payload,
        occurred_at: chrono::Utc::now(),
    })
}

fn types(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Preflight aborts — nothing is written on any of these paths
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn ambiguous_aggregate_id_aborts_the_whole_run_and_names_the_row() {
    let (pool, _container) = start_pool().await;

    let ambiguous_id = insert_raw_event(&pool, "user-account-7", Some("tenant-1"), 1).await;
    let clean_id = insert_raw_event(&pool, "user-9", Some("tenant-1"), 1).await;

    let report =
        aggregate_type_backfill::backfill_aggregate_type(&pool, &types(&["user-account", "user"]))
            .await
            .expect("the backfill call itself must not error");

    assert_eq!(report.rows_scanned, 2);
    assert_eq!(report.rows_rewritten, 0);
    match report.outcome {
        BackfillOutcome::Aborted(abort) => {
            assert_eq!(abort.reason, AbortReason::MatchesMoreThanOneRegisteredType);
            assert_eq!(abort.offending_row_ids, vec![ambiguous_id]);
        }
        other => panic!("expected an abort, got {other:?}"),
    }

    // Total abort: the clean row must be untouched too, not partially split.
    assert_eq!(
        row_aggregate_id(&pool, ambiguous_id).await,
        "user-account-7"
    );
    assert_eq!(row_aggregate_type(&pool, ambiguous_id).await, None);
    assert_eq!(row_aggregate_id(&pool, clean_id).await, "user-9");
    assert_eq!(row_aggregate_type(&pool, clean_id).await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn aggregate_id_matching_no_registered_type_aborts_and_names_the_row() {
    let (pool, _container) = start_pool().await;

    let orphan_id = insert_raw_event(&pool, "orphan-123", Some("tenant-1"), 1).await;

    let report = aggregate_type_backfill::backfill_aggregate_type(&pool, &types(&["user"]))
        .await
        .expect("the backfill call itself must not error");

    match report.outcome {
        BackfillOutcome::Aborted(abort) => {
            assert_eq!(abort.reason, AbortReason::NoRegisteredTypeMatches);
            assert_eq!(abort.offending_row_ids, vec![orphan_id]);
        }
        other => panic!("expected an abort, got {other:?}"),
    }
    assert_eq!(row_aggregate_type(&pool, orphan_id).await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn whitespace_only_remainder_aborts_and_names_the_row() {
    let (pool, _container) = start_pool().await;

    // "user-   " splits under "user" into a remainder that is whitespace
    // only — a real value the column's NOT NULL constraint alone would not
    // reject, since it is not empty text, only content-free.
    let whitespace_id = insert_raw_event(&pool, "user-   ", Some("tenant-1"), 1).await;

    let report = aggregate_type_backfill::backfill_aggregate_type(&pool, &types(&["user"]))
        .await
        .expect("the backfill call itself must not error");

    match report.outcome {
        BackfillOutcome::Aborted(abort) => {
            assert_eq!(abort.reason, AbortReason::AggregateIdIsEmptyOrWhitespace);
            assert_eq!(abort.offending_row_ids, vec![whitespace_id]);
        }
        other => panic!("expected an abort, got {other:?}"),
    }
    assert_eq!(row_aggregate_type(&pool, whitespace_id).await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn post_split_identity_collision_aborts_and_names_both_rows() {
    let (pool, _container) = start_pool().await;

    // The current schema enforces no uniqueness on the stream identity (see
    // `event_store_characterization.rs`), so two rows sharing the identical
    // tenant, aggregate_id and version already coexist in a real database
    // today. After the split they would still collide — exactly what the
    // eventual unique index would refuse, caught here first.
    let first_id = insert_raw_event(&pool, "user-7", Some("tenant-1"), 1).await;
    let second_id = insert_raw_event(&pool, "user-7", Some("tenant-1"), 1).await;

    let report = aggregate_type_backfill::backfill_aggregate_type(&pool, &types(&["user"]))
        .await
        .expect("the backfill call itself must not error");

    match report.outcome {
        BackfillOutcome::Aborted(abort) => {
            assert_eq!(abort.reason, AbortReason::PostSplitIdentityWouldCollide);
            let mut offending = abort.offending_row_ids.clone();
            offending.sort();
            let mut expected = vec![first_id, second_id];
            expected.sort();
            assert_eq!(offending, expected);
        }
        other => panic!("expected an abort, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Post-verification — the writes ran and were discarded
// ---------------------------------------------------------------------------

/// A stream whose versions are not the consecutive run `1..=n` stops the
/// transition, and nothing is consolidated.
///
/// Every row here splits cleanly, so preflight has no objection and the rewrite
/// does run. The refusal comes from post-verification reading the written rows
/// back: `user-7` holds versions 1 and 3, which is not a stream this migration
/// may consolidate. Per-stream version continuity is the property that proves
/// the transformation did not re-partition history, and the tool cannot
/// distinguish a hole it created from one that was already there — so it refuses
/// either way rather than guessing that the gap was pre-existing.
///
/// Three assertions, because "it refused" is not the same claim as "nothing
/// changed": the outcome must be the rolled-back one and name the rows, the data
/// must still be in its joined pre-split form, and the column must still be
/// nullable. The last one matters most — a column left mandatory would be a
/// consolidated fragment of a transition that was supposed to be all-or-nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_stream_with_a_version_gap_rolls_the_whole_transformation_back() {
    let (pool, _container) = start_pool().await;

    let v1 = insert_raw_event(&pool, "user-7", Some("tenant-1"), 1).await;
    let v3 = insert_raw_event(&pool, "user-7", Some("tenant-1"), 3).await;
    // A well-formed stream alongside it, to pin that the refusal is total rather
    // than per-row: this row splits cleanly and still must not be consolidated.
    let untouched = insert_raw_event(&pool, "organization-org-1", None, 1).await;

    let report =
        aggregate_type_backfill::backfill_aggregate_type(&pool, &types(&["user", "organization"]))
            .await
            .expect("the backfill call itself must not error");

    assert_eq!(report.rows_scanned, 3);
    assert_eq!(
        report.rows_rewritten, 0,
        "a rolled-back run consolidated no rows, whatever it wrote inside the transaction"
    );
    match report.outcome {
        BackfillOutcome::RolledBack(abort) => {
            assert_eq!(
                abort.reason,
                AbortReason::StreamVersionsAreNotConsecutiveFromOne
            );
            let mut offending = abort.offending_row_ids.clone();
            offending.sort();
            let mut expected = vec![v1, v3];
            expected.sort();
            assert_eq!(
                offending, expected,
                "both rows of the discontinuous stream must be named, and only those"
            );
        }
        other => panic!("expected a rolled-back run, got {other:?}"),
    }

    // Nothing was consolidated: the identifiers are still joined and no type was
    // recorded, for the offending stream and the clean one alike.
    let rows: Vec<(i64, String, Option<String>)> =
        sqlx::query_as("SELECT id, aggregate_id, aggregate_type FROM events ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("the table must still be readable");
    assert_eq!(
        rows,
        vec![
            (v1, "user-7".to_string(), None),
            (v3, "user-7".to_string(), None),
            (untouched, "organization-org-1".to_string(), None),
        ],
        "every row must be exactly as it was before the attempt"
    );

    assert!(
        !aggregate_type_column_is_not_null(&pool).await,
        "the column must still be nullable: SET NOT NULL runs only after verification passes"
    );
}

// ---------------------------------------------------------------------------
// Happy path, including the trivial (zero-row) case a fresh environment hits
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn zero_rows_commits_trivially_and_still_sets_the_column_not_null() {
    let (pool, _container) = start_pool().await;

    let report = aggregate_type_backfill::backfill_aggregate_type(&pool, &types(&["user"]))
        .await
        .expect("the backfill call itself must not error");

    assert_eq!(
        report,
        aggregate_type_backfill::BackfillReport {
            rows_scanned: 0,
            rows_rewritten: 0,
            outcome: BackfillOutcome::Committed,
        }
    );
    assert!(
        aggregate_type_column_is_not_null(&pool).await,
        "even with nothing to rewrite, the column must become mandatory once the run commits"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn clean_data_splits_every_row_and_preserves_row_count_and_stream_integrity() {
    let (pool, _container) = start_pool().await;

    // Two streams, two tenants (including the NULL/systemwide tenant),
    // each with a consecutive version sequence starting at 1.
    let user_7_v1 = insert_raw_event(&pool, "user-7", Some("tenant-1"), 1).await;
    let user_7_v2 = insert_raw_event(&pool, "user-7", Some("tenant-1"), 2).await;
    let org_1_v1 = insert_raw_event(&pool, "organization-org-1", None, 1).await;

    let before_count = row_count(&pool).await;

    let report =
        aggregate_type_backfill::backfill_aggregate_type(&pool, &types(&["user", "organization"]))
            .await
            .expect("the backfill call itself must not error");

    assert_eq!(report.rows_scanned, 3);
    assert_eq!(report.rows_rewritten, 3);
    assert_eq!(report.outcome, BackfillOutcome::Committed);

    // Row count identical before and after.
    assert_eq!(row_count(&pool).await, before_count);

    // Every row split into the bare id, not the joined string.
    assert_eq!(row_aggregate_id(&pool, user_7_v1).await, "7");
    assert_eq!(
        row_aggregate_type(&pool, user_7_v1).await,
        Some("user".to_string())
    );
    assert_eq!(row_aggregate_id(&pool, user_7_v2).await, "7");
    assert_eq!(row_aggregate_id(&pool, org_1_v1).await, "org-1");
    assert_eq!(
        row_aggregate_type(&pool, org_1_v1).await,
        Some("organization".to_string())
    );

    assert!(aggregate_type_column_is_not_null(&pool).await);

    // Post-split identity is unique across the table.
    let duplicate_identities: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ( \
             SELECT tenant_id, aggregate_type, aggregate_id, version, COUNT(*) AS c \
             FROM events \
             GROUP BY tenant_id, aggregate_type, aggregate_id, version \
             HAVING COUNT(*) > 1 \
         ) duplicates",
    )
    .fetch_one(&pool)
    .await
    .expect("the grouped duplicate-check query must succeed");
    assert_eq!(
        duplicate_identities, 0,
        "no two rows may share the same post-split identity"
    );

    // Per stream, versions are consecutive and start at 1: for a stream of N
    // rows, the version set is exactly {1, ..., N} — checked by comparing the
    // count of distinct versions against both the row count and the max
    // version for each (tenant_id, aggregate_type, aggregate_id) group.
    let gappy_or_misnumbered_streams: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ( \
             SELECT tenant_id, aggregate_type, aggregate_id, \
                    COUNT(*) AS row_count, \
                    COUNT(DISTINCT version) AS distinct_versions, \
                    MIN(version) AS min_version, \
                    MAX(version) AS max_version \
             FROM events \
             GROUP BY tenant_id, aggregate_type, aggregate_id \
             HAVING COUNT(*) <> COUNT(DISTINCT version) \
                 OR MIN(version) <> 1 \
                 OR MAX(version) <> COUNT(*) \
         ) bad_streams",
    )
    .fetch_one(&pool)
    .await
    .expect("the stream-integrity query must succeed");
    assert_eq!(
        gappy_or_misnumbered_streams, 0,
        "every stream's versions must be gap-free and start at 1"
    );
}

// ---------------------------------------------------------------------------
// The reverse migration — exact and lossless
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn revert_exactly_rejoins_what_the_backfill_split_and_drops_the_column() {
    let (pool, _container) = start_pool().await;

    let a = insert_raw_event(&pool, "user-7", Some("tenant-1"), 1).await;
    let b = insert_raw_event(&pool, "organization-org-1", None, 1).await;

    // Capture the pre-backfill shape directly, rather than trusting memory of
    // what was inserted, so the round-trip assertion below compares against
    // what the database actually held.
    let original_a = row_aggregate_id(&pool, a).await;
    let original_b = row_aggregate_id(&pool, b).await;

    let report =
        aggregate_type_backfill::backfill_aggregate_type(&pool, &types(&["user", "organization"]))
            .await
            .expect("the backfill call itself must not error");
    assert_eq!(report.outcome, BackfillOutcome::Committed);

    // Confirm the split actually happened, so the revert below is proven to
    // undo real work rather than trivially matching an unchanged table.
    assert_ne!(row_aggregate_id(&pool, a).await, original_a);

    aggregate_type_backfill::revert_aggregate_type_column(&pool)
        .await
        .expect("the revert must succeed");

    assert_eq!(row_aggregate_id(&pool, a).await, original_a);
    assert_eq!(row_aggregate_id(&pool, b).await, original_b);

    let column_still_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 FROM information_schema.columns \
             WHERE table_name = 'events' AND column_name = 'aggregate_type' \
         )",
    )
    .fetch_one(&pool)
    .await
    .expect("the column-existence check must succeed");
    assert!(
        !column_still_exists,
        "the reverse migration must drop the column, not merely null it out"
    );
}

/// The store refuses to open while any row still lacks its aggregate type, and
/// opens once the backfill has completed.
///
/// This is the guard that makes the transition safe against being deployed in
/// the wrong order. Every read and the version check filter on the type column,
/// so against a row that predates the split neither filter matches — the
/// comparison against null is never true, and the joined text is not the bare
/// identifier. Such a stream reads as absent, the version check returns zero,
/// and an append writes a second forked stream while the original rows are
/// orphaned. Starting a new process before the backfill has run must therefore
/// be a visible, recoverable failure rather than silent history divergence.
///
/// The third property the guard has to hold — that the refusal happens before
/// any store operation is possible — is proved by the constructor's shape rather
/// than by an assertion: it returns a result, so on the unmigrated path no store
/// value exists at all and there is nothing to call a read or an append on. The
/// match below can only reach the store through the success arm, and the type
/// system is what makes that unbypassable.
#[tokio::test(flavor = "multi_thread")]
async fn the_store_refuses_to_open_until_the_backfill_has_completed() {
    let (pool, _container) = start_pool().await;

    // One row from before the split: joined identifier, no type recorded.
    insert_raw_event(&pool, "user-7", Some("tenant-1"), 1).await;

    let message =
        match PostgreSQLEventStore::<ProbeEvent, _>::open(pool.clone(), probe_deserialize).await {
            Ok(_) => panic!("the store must refuse to open while a row has no aggregate type"),
            Err(refusal) => refusal.to_string(),
        };
    assert!(
        message.contains("fork their history"),
        "the refusal must say why it refuses, not merely that it did: {message}"
    );

    // Run the transition to completion.
    let report = aggregate_type_backfill::backfill_aggregate_type(&pool, &types(&["user"]))
        .await
        .expect("the backfill must be able to evaluate this table");
    assert!(
        matches!(report.outcome, BackfillOutcome::Committed),
        "the backfill must commit for a table whose single row splits cleanly: {report:?}"
    );

    // The identical call now succeeds — same pool, same arguments, only the data
    // has changed.
    let opened = PostgreSQLEventStore::<ProbeEvent, _>::open(pool.clone(), probe_deserialize).await;
    assert!(
        opened.is_ok(),
        "the store must open once every row carries its aggregate type"
    );
}
