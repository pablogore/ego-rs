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

/// Migration SQL for the pair of partial unique indexes that enforce the event
/// stream identity.
const MIGRATION_008_STREAM_IDENTITY_UNIQUE: &str =
    include_str!("migrations/008_events_stream_identity_unique.sql");

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
        (
            "008_events_stream_identity_unique",
            MIGRATION_008_STREAM_IDENTITY_UNIQUE,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every `.sql` file in the migrations directory is registered above, and
    /// every registered name has a file.
    ///
    /// This exists because the repository already carried three migration files
    /// — numbered into the applied sequence, complete with primary keys and
    /// indexes — that no code path ever executed. Nothing failed, because
    /// nothing was checking: `include_str!` only binds the files that are named,
    /// so a file nobody names is silently inert while still looking, to anyone
    /// reading the directory, exactly like a migration that ships.
    ///
    /// The check is deliberately bidirectional. The forward direction catches a
    /// file added without being registered, which is the failure that already
    /// happened. The reverse direction catches a name registered for a file that
    /// was renamed or removed — that one cannot compile today, since
    /// `include_str!` would fail, but stating both keeps the invariant true
    /// rather than relying on a compiler error to cover half of it.
    ///
    /// Reading the source directory from a test is the point: the registry is a
    /// hand-written list, so the only way to verify it is complete is to compare
    /// it against the filesystem it claims to describe.
    #[test]
    fn every_migration_file_is_registered_and_every_registration_has_a_file() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("postgres")
            .join("migrations");

        let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
            .expect("the migrations directory must be readable from the crate source tree")
            .map(|entry| entry.expect("each directory entry must be readable").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
            .map(|path| {
                path.file_stem()
                    .expect("a .sql file always has a stem")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();

        let registered: BTreeSet<String> = migrations()
            .into_iter()
            .map(|(name, _)| name.to_string())
            .collect();

        let unregistered: Vec<&String> = on_disk.difference(&registered).collect();
        assert!(
            unregistered.is_empty(),
            "these migration files exist but no code runs them: {unregistered:?}. \
             A file that is not registered never executes, so the table it creates \
             never exists — either register it in `migrations()` or delete it, but \
             do not leave it looking applied."
        );

        let missing: Vec<&String> = registered.difference(&on_disk).collect();
        assert!(
            missing.is_empty(),
            "these migrations are registered but have no file on disk: {missing:?}"
        );
    }

    /// Registration order is the execution order, so it must be ascending by the
    /// numeric prefix.
    ///
    /// Order does not matter for the current set — every statement is
    /// `IF NOT EXISTS` or an `ALTER` against a table an earlier file creates —
    /// but a migration that depends on an earlier one running first is the normal
    /// case, not the exception, and it would fail in a way that points at the SQL
    /// rather than at the list that misordered it.
    #[test]
    fn registration_order_ascends_by_numeric_prefix() {
        let prefixes: Vec<&str> = migrations()
            .iter()
            .map(|(name, _)| {
                name.split('_')
                    .next()
                    .expect("a migration name always has a prefix segment")
            })
            .collect();

        let mut ascending = prefixes.clone();
        ascending.sort_unstable();
        assert_eq!(
            prefixes, ascending,
            "migrations run in registration order, so the list must ascend by prefix"
        );
    }
}
