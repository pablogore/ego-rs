//! **Guarantee (IS-9):** PostgreSQL 14 remains a verified, real compatibility
//! floor for exactly the version-sensitive invariants named below (T0–T3),
//! never a second full run of the main suite.
//!
//! Every table in `README.md`'s "The PostgreSQL concurrency invariants" and
//! "Migration transactional behaviour" sections above uses `NULLS NOT
//! DISTINCT` avoidance — two partial unique indexes over complementary
//! predicates — precisely because that syntax arrived in PostgreSQL 15 and
//! this workspace declares 14 as its floor. The four cases here are the ones
//! that could not otherwise be told apart from a run against the main
//! suite's PostgreSQL 16 container:
//!
//! - **T0 — anti-vacuity guard.** The same "three-empty-sets" discipline the
//!   ledger guard already applies: without a real assertion on the target's
//!   own reported version, a container-tag typo would silently run this
//!   whole file against PG16 and prove nothing about PG14 at all.
//! - **T1 — the full migration set applies cleanly.** [`pg14_database`]
//!   already ran every registered migration in place (not cloned from an
//!   already-migrated template, the way [`isolated_database`] deliberately
//!   is for the main suite) before this test body ever runs, and a
//!   migration failure there panics inside the fixture. This test adds an
//!   explicit, positive catalog assertion — every version-sensitive artifact
//!   from migrations 008, 010, 011 and 012 genuinely exists — rather than
//!   relying only on "the fixture did not panic."
//! - **T2 — a systemwide duplicate is refused with `23505`.** Exercises
//!   `ux_events_identity_systemwide` (migration 008) directly: this is the
//!   index whose own comment records that `CREATE UNIQUE INDEX ...
//!   NULLS NOT DISTINCT` is a syntax error on the pinned 14-alpine image.
//! - **T3 — migration 007's backfill/revert round trip.** Mirrors
//!   `aggregate_type_backfill_postgres.rs`'s C4 case against the PG14
//!   database, proving the same offline tool that Group 4 verified against
//!   PG16 also round-trips on the version floor.
//!
//! # Why none of this can be shown in process
//!
//! A compatibility floor can only be demonstrated against the real target
//! engine version — an in-memory double has no engine version to be
//! compatible with.
//!
//! # What this file deliberately does not run on PG14
//!
//! Per `design.md` AD-6's own "Explicitly not on PG14" list: IS-1, IS-2,
//! IS-3, IS-6, and IS-4's C1/C2 abort/rollback cases, plus all sixteen
//! pre-existing tests. This file targets exactly T0–T3, nothing else — a
//! second full run of the main suite against PG14 would cost real minutes
//! for no additional coverage over what T0–T3 already prove about the
//! version floor.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use ego_persistence::postgres::aggregate_type_backfill::{
    backfill_aggregate_type, revert_aggregate_type_column, BackfillOutcome,
};
use ego_integration_tests::pg14_database;

#[tokio::test]
async fn t0_the_pg14_container_genuinely_reports_a_pg14_server_version() {
    let db = pg14_database().await;
    let pool = db.pool().await;

    let version_num: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
        .fetch_one(&pool)
        .await
        .expect("server_version_num is readable");

    assert!(
        (140000..150000).contains(&version_num),
        "expected a PostgreSQL 14.x server (server_version_num in \
         [140000, 150000)), got {version_num} — a container-tag typo would \
         silently run this whole file against the wrong version and prove \
         nothing about the PG14 floor"
    );

    db.close().await;
}

/// # Correction from this task's literal text
///
/// The registered migrations carry no tracking table at all —
/// `crates/persistence/src/postgres/migrations.rs`'s `run()` re-applies
/// every migration's idempotent (`IF NOT EXISTS`) SQL on every call, with
/// nothing recording which names have "already run." There is no
/// `schema_migrations`-shaped table to query. What is verifiable, and what
/// this asserts instead, is that every version-sensitive schema artifact
/// those migrations create genuinely exists on the PG14 target: the two
/// partial unique indexes from 008 (the one this workspace's whole
/// `NULLS NOT DISTINCT`-avoidance pattern exists for), the operation-key
/// column from 009, the `operation_reservations` and `operation_receipts`
/// tables and their own partial unique indexes from 010/011, and the
/// snapshots table's post-012 partial unique indexes. A migration that had
/// silently failed partway would leave at least one of these absent.
#[tokio::test]
async fn t1_the_full_migration_set_applies_cleanly_and_leaves_every_version_sensitive_artifact_present() {
    let db = pg14_database().await;
    let pool = db.pool().await;

    for index_name in [
        "ux_events_identity_tenant",
        "ux_events_identity_systemwide",
        "ux_operation_reservations_identity_tenant",
        "ux_operation_reservations_identity_systemwide",
        "ux_operation_receipts_identity_tenant",
        "ux_operation_receipts_identity_systemwide",
        "ux_snapshots_identity_tenant",
        "ux_snapshots_identity_systemwide",
    ] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = $1)",
        )
        .bind(index_name)
        .fetch_one(&pool)
        .await
        .expect("pg_indexes is readable");
        assert!(exists, "expected index {index_name} to exist on PG14 after migrations::run()");
    }

    for (table_name, column_name) in [("events", "aggregate_type"), ("events", "operation_key")] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_name = $1 AND column_name = $2)",
        )
        .bind(table_name)
        .bind(column_name)
        .fetch_one(&pool)
        .await
        .expect("information_schema.columns is readable");
        assert!(exists, "expected {table_name}.{column_name} to exist on PG14 after migrations::run()");
    }

    for table_name in ["operation_reservations", "operation_receipts"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(table_name)
        .fetch_one(&pool)
        .await
        .expect("information_schema.tables is readable");
        assert!(exists, "expected table {table_name} to exist on PG14 after migrations::run()");
    }

    db.close().await;
}

#[tokio::test]
async fn t2_a_systemwide_duplicate_identity_is_refused_with_23505_on_pg14() {
    let db = pg14_database().await;
    let pool = db.pool().await;

    sqlx::query(
        "INSERT INTO events (aggregate_id, tenant_id, aggregate_type, version, event_type, payload) \
         VALUES ('id-1', NULL, 'order', 1, 'seeded', '{}')",
    )
    .execute(&pool)
    .await
    .expect("the first systemwide row inserts cleanly");

    let duplicate = sqlx::query(
        "INSERT INTO events (aggregate_id, tenant_id, aggregate_type, version, event_type, payload) \
         VALUES ('id-1', NULL, 'order', 1, 'seeded', '{}')",
    )
    .execute(&pool)
    .await;

    match duplicate {
        Err(sqlx::Error::Database(db_err)) => {
            assert_eq!(
                db_err.code().as_deref(),
                Some("23505"),
                "expected a unique-violation (23505), got {:?}",
                db_err.code()
            );
        }
        other => panic!(
            "expected a database error carrying 23505 (unique_violation) from \
             ux_events_identity_systemwide, got {other:?} — PostgreSQL 15's \
             NULLS NOT DISTINCT is unavailable on the declared 14 floor, so \
             this partial index is what must refuse the duplicate instead"
        ),
    }

    db.close().await;
}

#[tokio::test]
async fn t3_the_backfill_and_its_revert_round_trip_cleanly_on_pg14() {
    let db = pg14_database().await;
    let pool = db.pool().await;

    sqlx::query(
        "INSERT INTO events (aggregate_id, tenant_id, version, event_type, payload) \
         VALUES ('user-7', 'tenant-a', 1, 'seeded', '{}')",
    )
    .execute(&pool)
    .await
    .expect("the seed row inserts cleanly");

    let before: String = sqlx::query_scalar(
        "SELECT COALESCE(md5(string_agg(\
            (id, aggregate_id, tenant_id, version, event_type, payload, created_at, operation_key)::text, \
            '|' ORDER BY id)), 'empty') FROM events",
    )
    .fetch_one(&pool)
    .await
    .expect("the digest query runs");

    let report = backfill_aggregate_type(&pool, &[String::from("user")])
        .await
        .expect("the backfill runs without a database error");
    assert_eq!(
        report.outcome,
        BackfillOutcome::Committed,
        "the seeded row must split unambiguously and commit, or this case \
         proves nothing about a real revert on PG14"
    );

    revert_aggregate_type_column(&pool)
        .await
        .expect("the revert runs without a database error");

    let after: String = sqlx::query_scalar(
        "SELECT COALESCE(md5(string_agg(\
            (id, aggregate_id, tenant_id, version, event_type, payload, created_at, operation_key)::text, \
            '|' ORDER BY id)), 'empty') FROM events",
    )
    .fetch_one(&pool)
    .await
    .expect("the digest query runs");

    assert_eq!(
        before, after,
        "a revert on PG14 must rejoin exactly the state that preceded the backfill"
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
        "the reverted column must no longer exist on PG14"
    );

    db.close().await;
}
