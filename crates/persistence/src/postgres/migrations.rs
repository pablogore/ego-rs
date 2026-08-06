//! Database migrations for PostgreSQL persistence backends.

use sqlx::PgPool;

/// Migration SQL for creating the events table.
const MIGRATION_001_CREATE_EVENTS: &str = include_str!("migrations/001_create_events.sql");

/// Migration SQL for creating the aggregates table.
const MIGRATION_002_CREATE_AGGREGATES: &str = include_str!("migrations/002_create_aggregates.sql");

/// Migration SQL for creating the snapshots table.
const MIGRATION_003_CREATE_SNAPSHOTS: &str = include_str!("migrations/003_create_snapshots.sql");

/// Migration SQL for adding the (nullable) `aggregate_type` column to `events`.
const MIGRATION_007_ADD_AGGREGATE_TYPE: &str =
    include_str!("migrations/007_add_aggregate_type_to_events.sql");

/// Run all migrations against the database.
///
/// Creates the events, aggregates, and snapshots tables if they don't exist.
pub async fn run(pool: &PgPool) -> Result<(), sqlx::Error> {
    for (name, sql) in migrations() {
        log::debug!("Running migration: {}", name);
        // Each migration file may contain more than one statement (e.g. a
        // `CREATE TABLE` followed by a `CREATE INDEX`). `sqlx::query` prepares
        // its argument as a single statement through the extended protocol,
        // which Postgres rejects outright when the string holds more than
        // one command. `raw_sql` uses the simple query protocol instead,
        // which allows exactly this — multiple semicolon-separated statements
        // in one round trip, with no prepared-statement caching needed for a
        // one-shot schema migration anyway.
        sqlx::raw_sql(sql).execute(pool).await?;
    }
    Ok(())
}

/// Returns an iterator over all migration pairs (name, sql).
fn migrations() -> Vec<(&'static str, &'static str)> {
    vec![
        ("001_create_events", MIGRATION_001_CREATE_EVENTS),
        ("002_create_aggregates", MIGRATION_002_CREATE_AGGREGATES),
        ("003_create_snapshots", MIGRATION_003_CREATE_SNAPSHOTS),
        (
            "007_add_aggregate_type_to_events",
            MIGRATION_007_ADD_AGGREGATE_TYPE,
        ),
    ]
}
