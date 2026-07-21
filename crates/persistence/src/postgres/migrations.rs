//! Database migrations for PostgreSQL persistence backends.

use sqlx::PgPool;

/// Migration SQL for creating the events table.
const MIGRATION_001_CREATE_EVENTS: &str = include_str!("migrations/001_create_events.sql");

/// Migration SQL for creating the aggregates table.
const MIGRATION_002_CREATE_AGGREGATES: &str = include_str!("migrations/002_create_aggregates.sql");

/// Migration SQL for creating the snapshots table.
const MIGRATION_003_CREATE_SNAPSHOTS: &str = include_str!("migrations/003_create_snapshots.sql");

/// Migration SQL enforcing NOT NULL on every `tenant_id` column.
const MIGRATION_007_TENANT_ID_NOT_NULL: &str =
    include_str!("migrations/007_tenant_id_not_null.sql");

/// Run all migrations against the database.
///
/// Creates the events, aggregates, and snapshots tables if they don't exist.
pub async fn run(pool: &PgPool) -> Result<(), sqlx::Error> {
    for (name, sql) in migrations() {
        log::debug!("Running migration: {}", name);
        sqlx::query(sql).execute(pool).await?;
    }
    Ok(())
}

/// Returns an iterator over all migration pairs (name, sql).
fn migrations() -> Vec<(&'static str, &'static str)> {
    vec![
        ("001_create_events", MIGRATION_001_CREATE_EVENTS),
        ("002_create_aggregates", MIGRATION_002_CREATE_AGGREGATES),
        ("003_create_snapshots", MIGRATION_003_CREATE_SNAPSHOTS),
        ("007_tenant_id_not_null", MIGRATION_007_TENANT_ID_NOT_NULL),
    ]
}
