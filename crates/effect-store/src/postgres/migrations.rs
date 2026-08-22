//! Database migrations for the PostgreSQL effect-store backend (PROD-002
//! AD-10).
//!
//! Own numbered sequence starting at `001`, hand-rolled `include_str!`
//! runner — mirrors `ego-persistence`'s pattern
//! (`crates/persistence/src/postgres/migrations.rs`) rather than sqlx's
//! `migrate!`/`_sqlx_migrations` ledger. No collision with `ego-persistence`'s
//! `001`-`006`: different crate, different tables (`effect_`-prefixed), no
//! shared version ledger.

use sqlx::PgPool;

const MIGRATION_001_EFFECT_STATE: &str = include_str!("migrations/001_effect_state.sql");
const MIGRATION_002_EFFECT_DEDUP: &str = include_str!("migrations/002_effect_dedup.sql");

/// Runs every migration, in order, against `pool`. Idempotent (every
/// statement is `CREATE TABLE IF NOT EXISTS`/`CREATE INDEX IF NOT EXISTS`),
/// so calling this again against an already-migrated schema is a no-op.
///
/// Uses [`sqlx::raw_sql`] rather than `sqlx::query` — each migration file
/// here holds more than one `;`-terminated statement (a `CREATE TABLE`
/// followed by its indexes), and `sqlx::query`'s prepared-statement
/// protocol rejects that (`cannot insert multiple commands into a prepared
/// statement`, confirmed against a real PostgreSQL instance while authoring
/// this). `raw_sql` runs unprepared over the simple query protocol, which
/// PostgreSQL's wire protocol allows to carry a `;`-separated batch.
pub async fn run(pool: &PgPool) -> Result<(), sqlx::Error> {
    for sql in migrations() {
        sqlx::raw_sql(sql).execute(pool).await?;
    }
    Ok(())
}

fn migrations() -> Vec<&'static str> {
    vec![MIGRATION_001_EFFECT_STATE, MIGRATION_002_EFFECT_DEDUP]
}
