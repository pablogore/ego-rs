//! PostgreSQL durable [`DedupStore`].
//!
//! # Storage convergence, not execution exclusion
//!
//! Two concurrent `mark_seen` calls for the same `(projection_id, tag,
//! event_id)` converge to **one row**, with no error surfacing to either
//! caller — that is what the `ON CONFLICT (…) DO NOTHING` below guarantees.
//! It does **not** prevent a handler from having already run twice: `seen()`
//! and `mark_seen()` are separate calls with the handler running between
//! them, so two writers can both observe "not yet seen" and both execute
//! before either records it. This capability delivers at-least-once handler
//! execution with best-effort dedup bookkeeping, never exactly-once handling.
//!
//! Safe operation depends on an external, unenforced adoption constraint:
//! exactly one writer per `(projection_id, tag, tenant)`. No leader
//! election, lock, lease, or fencing token exists here to enforce that
//! constraint across replicas — see **PROD-014C — Atomic Read-Side Event
//! Claiming**, the named, distinct follow-up that would close this gap.
//!
//! # Storage growth is unbounded
//!
//! `projection_dedup` grows monotonically with the number of unique events
//! processed, with no purge, TTL, or eviction mechanism shipped by this
//! store. Row count is a signal to observe; the mechanism and its horizon
//! belong to a separately owned retention follow-up.

use std::fmt;

use async_trait::async_trait;
use sqlx::PgPool;

use ego_domain::read_side::dedup::{DedupStore, DedupStoreError};
use ego_domain::read_side::event_tag::EventTag;

use crate::postgres::is_fatal;

/// Maps a storage failure into the port's `Transient`/`Fatal` split.
fn dedup_error(err: sqlx::Error) -> DedupStoreError {
    let text = err.to_string();
    if is_fatal(&err) {
        DedupStoreError::Fatal(text)
    } else {
        DedupStoreError::Transient(text)
    }
}

/// A durable [`DedupStore`] backed by one PostgreSQL table
/// (`projection_dedup`).
///
/// Deduplication identity is `(projection_id, tag, event_id)` — no
/// `tenant` column. This is not a tenant-isolation defect: `seen`/`mark_seen`
/// take no tenant on the port itself, and this table stores no tenant-owned
/// value, only the presence of an event identifier.
pub struct PostgreSQLDedupStore {
    pool: PgPool,
}

impl fmt::Debug for PostgreSQLDedupStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgreSQLDedupStore")
            .field("pool", &self.pool)
            .finish()
    }
}

impl PostgreSQLDedupStore {
    /// Creates a new durable dedup store over `pool`.
    ///
    /// `pool` must already have had `crate::postgres::migrations::run`
    /// applied — migration `014` creates the table this store reads and
    /// writes.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DedupStore for PostgreSQLDedupStore {
    fn is_durable(&self) -> bool {
        true
    }

    async fn seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<bool, DedupStoreError> {
        let hit: Option<i32> = sqlx::query_scalar(
            r#"SELECT 1 FROM projection_dedup
               WHERE projection_id = $1 AND tag = $2 AND event_id = $3"#,
        )
        .bind(projection_id)
        .bind(tag.value())
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dedup_error)?;

        Ok(hit.is_some())
    }

    async fn mark_seen(
        &self,
        projection_id: &str,
        tag: &EventTag,
        event_id: &str,
    ) -> Result<(), DedupStoreError> {
        sqlx::query(
            r#"INSERT INTO projection_dedup (projection_id, tag, event_id)
               VALUES ($1, $2, $3)
               ON CONFLICT (projection_id, tag, event_id) DO NOTHING"#,
        )
        .bind(projection_id)
        .bind(tag.value())
        .bind(event_id)
        .execute(&self.pool)
        .await
        .map_err(dedup_error)?;

        Ok(())
    }
}
