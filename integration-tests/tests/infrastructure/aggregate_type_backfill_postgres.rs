//! **Guarantee (IS-4):** `backfill_aggregate_type` and its exact reverse,
//! `revert_aggregate_type_column`, leave the `events` table exactly as they
//! found it on every refusal path, and rejoin exactly what they split apart
//! on the one path where a real revert is possible.
//!
//! Four cases, one per stage of the tool's own two-judgment design
//! (`crates/persistence/src/postgres/aggregate_type_backfill.rs`):
//!
//! - **C1 — `Aborted`.** A preflight refusal, before the first `UPDATE`.
//!   Proves statement *ordering*: nothing was ever written, so there is
//!   nothing to undo.
//! - **C2 — `RolledBack`.** A post-verification refusal, after at least one
//!   completed `UPDATE`. Proves a genuine transaction rollback, not merely
//!   ordering — this is the only case where rows are written and then
//!   discarded.
//! - **C3 — `Committed` over zero rows.** Proves the run commits the
//!   schema-level `SET NOT NULL` even when it rewrites nothing, so a report
//!   of "committed" cannot be confused with "committed nothing at all."
//! - **C4 — revert round trip.** Proves `revert_aggregate_type_column`
//!   rejoins exactly the state a successful backfill split apart.
//!
//! # Why none of this can be shown in process
//!
//! Every case needs a real migrated table: the preflight/post-verification
//! split lives entirely in SQL executed inside one real transaction, and C2's
//! whole point — that a rollback, not statement ordering, is what discards
//! the writes — has no equivalent in an in-memory double that never opens a
//! transaction to roll back in the first place.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use ego_persistence::postgres::aggregate_type_backfill::{
    backfill_aggregate_type, revert_aggregate_type_column, AbortReason, AbortReport,
    BackfillOutcome,
};
use ego_integration_tests::isolated_database;
use sqlx::PgPool;

/// Seeds one row in the pre-backfill shape: a joined `aggregate_id` (for
/// example `"user-7"`) and no separate `aggregate_type`, which is exactly
/// what every row already in the table looks like before this operator step
/// has ever run. Returns the row's real `id`, since `BIGSERIAL` values are
/// not otherwise predictable.
async fn seed_row(pool: &PgPool, joined_aggregate_id: &str, tenant_id: Option<&str>, version: i64) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO events (aggregate_id, tenant_id, version, event_type, payload) \
         VALUES ($1, $2, $3, 'seeded', '{}') RETURNING id",
    )
    .bind(joined_aggregate_id)
    .bind(tenant_id)
    .bind(version)
    .fetch_one(pool)
    .await
    .expect("the seed row inserts cleanly")
}

/// Byte-identical proof over every column of `events`, in row order.
///
/// `events::text` renders the whole row as one composite literal, so a
/// column added to the table later is covered automatically — the digest
/// changes with it — without editing this test (AD-5). `COALESCE` handles
/// the empty-table case, where `string_agg` over zero rows is `NULL`.
async fn table_digest(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "SELECT COALESCE(md5(string_agg(events::text, '|' ORDER BY id)), 'empty') FROM events",
    )
    .fetch_one(pool)
    .await
    .expect("the digest query runs")
}

/// A digest over every column of `events` **except** `aggregate_type`.
///
/// C4 is the one case where `table_digest` above cannot express "unchanged":
/// `revert_aggregate_type_column` drops the column entirely, so a composite
/// built from `events::text` afterward has one fewer field than it did
/// before the backfill ever ran — a shape mismatch no amount of content
/// fidelity could satisfy. Naming the surviving columns explicitly is the
/// only way to state "the revert rejoined exactly what was split apart"
/// without the column whose *existence*, not content, is what this case is
/// about.
async fn digest_excluding_aggregate_type(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "SELECT COALESCE(md5(string_agg(\
            (id, aggregate_id, tenant_id, version, event_type, payload, created_at, operation_key)::text, \
            '|' ORDER BY id)), 'empty') FROM events",
    )
    .fetch_one(pool)
    .await
    .expect("the digest query runs")
}

async fn aggregate_type_is_nullable(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns \
         WHERE table_name = 'events' AND column_name = 'aggregate_type'",
    )
    .fetch_one(pool)
    .await
    .expect("the catalog reports the column")
}

#[tokio::test]
async fn c1_an_abort_before_any_write_leaves_the_table_byte_identical() {
    let db = isolated_database().await;
    let pool = db.pool().await;

    // "orphan-123" matches no registered type's `"{type}-"` prefix.
    seed_row(&pool, "orphan-123", Some("tenant-a"), 1).await;

    let before = table_digest(&pool).await;
    let report = backfill_aggregate_type(&pool, &[String::from("user")])
        .await
        .expect("the backfill runs without a database error");
    let after = table_digest(&pool).await;

    assert_eq!(
        before, after,
        "an abort before the first UPDATE must leave the table byte-identical"
    );
    assert_eq!(report.rows_rewritten, 0);
    match report.outcome {
        BackfillOutcome::Aborted(AbortReport {
            reason: AbortReason::NoRegisteredTypeMatches,
            ..
        }) => {}
        other => panic!("expected Aborted(NoRegisteredTypeMatches), got {other:?}"),
    }

    db.close().await;
}

#[tokio::test]
async fn c2_a_rollback_after_a_completed_write_leaves_the_table_byte_identical() {
    let db = isolated_database().await;
    let pool = db.pool().await;

    // A hole: versions 1 and 3 with no 2, so the split succeeds for both rows
    // (proving rows really were written) and only post-verification refuses.
    seed_row(&pool, "user-42", Some("tenant-a"), 1).await;
    seed_row(&pool, "user-42", Some("tenant-a"), 3).await;

    let before = table_digest(&pool).await;
    let report = backfill_aggregate_type(&pool, &[String::from("user")])
        .await
        .expect("the backfill runs without a database error");
    let after = table_digest(&pool).await;

    assert_eq!(
        before, after,
        "a rollback after at least one completed UPDATE must leave the table \
         byte-identical — the rollback, not statement ordering, is what \
         guarantees this"
    );
    match report.outcome {
        BackfillOutcome::RolledBack(AbortReport {
            reason: AbortReason::StreamVersionsAreNotConsecutiveFromOne,
            ..
        }) => {}
        other => panic!(
            "expected RolledBack(StreamVersionsAreNotConsecutiveFromOne), got {other:?}"
        ),
    }

    assert_eq!(
        aggregate_type_is_nullable(&pool).await,
        "YES",
        "a rolled-back run must not have committed SET NOT NULL"
    );

    db.close().await;
}

#[tokio::test]
async fn c3_a_zero_row_commit_still_commits_the_schema_level_not_null() {
    let db = isolated_database().await;
    let pool = db.pool().await;

    let report = backfill_aggregate_type(&pool, &[])
        .await
        .expect("the backfill runs without a database error");

    assert_eq!(report.outcome, BackfillOutcome::Committed);
    assert_eq!(report.rows_scanned, 0);
    assert_eq!(
        aggregate_type_is_nullable(&pool).await,
        "NO",
        "a run over zero eligible rows must still commit the schema-level \
         SET NOT NULL — a looser reading of \"commits cleanly\" would also be \
         satisfied by a run that committed nothing at all"
    );

    db.close().await;
}

#[tokio::test]
async fn c4_a_revert_rejoins_exactly_the_state_that_preceded_the_backfill() {
    let db = isolated_database().await;
    let pool = db.pool().await;

    seed_row(&pool, "user-7", Some("tenant-a"), 1).await;
    seed_row(&pool, "organization-3", None, 1).await;

    let before = digest_excluding_aggregate_type(&pool).await;

    let report = backfill_aggregate_type(&pool, &[String::from("user"), String::from("organization")])
        .await
        .expect("the backfill runs without a database error");
    assert_eq!(
        report.outcome,
        BackfillOutcome::Committed,
        "the seeded rows must split unambiguously and commit, or this case \
         proves nothing about a real revert"
    );

    revert_aggregate_type_column(&pool)
        .await
        .expect("the revert runs without a database error");

    let after = digest_excluding_aggregate_type(&pool).await;
    assert_eq!(
        before, after,
        "a revert must rejoin exactly the state that preceded the backfill"
    );

    let column_exists: Option<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_name = 'events' AND column_name = 'aggregate_type'",
    )
    .fetch_optional(&pool)
    .await
    .expect("the catalog is readable");
    assert!(
        column_exists.is_none(),
        "the reverted column must no longer exist"
    );

    db.close().await;
}
