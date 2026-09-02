//! PostgreSQL durable [`OffsetStore`].
//!
//! # The delivered guarantee
//!
//! `write_offset` is a plain upsert — no compare-and-swap, no
//! expected-previous-offset check, and no detection of a concurrent
//! overwrite. This is a faithful implementation of [`OffsetStore`]'s own
//! write contract (the trait never expresses a conditional write), not an
//! adapter-level shortcoming. Safe operation depends on an external,
//! unenforced adoption constraint: exactly one writer per
//! `(projection_id, tag, tenant)`. Two replicas of the same projection are
//! outside the guarantee this store provides, and nothing here detects or
//! refuses that configuration — see **PROD-014C — Atomic Read-Side Event
//! Claiming** for the named, distinct follow-up that would close this gap.

use std::fmt;

use async_trait::async_trait;
use sqlx::PgPool;

use ego_domain::read_side::event_tag::EventTag;
use ego_domain::read_side::offset::{Offset, OffsetStore, OffsetStoreError};

use crate::postgres::is_fatal;

/// Maps a storage failure into the port's `Transient`/`Fatal` split.
fn offset_error(err: sqlx::Error) -> OffsetStoreError {
    let text = err.to_string();
    if is_fatal(&err) {
        OffsetStoreError::Fatal(text)
    } else {
        OffsetStoreError::Transient(text)
    }
}

/// A durable [`OffsetStore`] backed by one PostgreSQL table
/// (`projection_offsets`).
pub struct PostgreSQLOffsetStore {
    pool: PgPool,
}

impl fmt::Debug for PostgreSQLOffsetStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgreSQLOffsetStore")
            .field("pool", &self.pool)
            .finish()
    }
}

impl PostgreSQLOffsetStore {
    /// Creates a new durable offset store over `pool`.
    ///
    /// `pool` must already have had `crate::postgres::migrations::run`
    /// applied — migration `013` creates the table this store reads and
    /// writes.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OffsetStore for PostgreSQLOffsetStore {
    fn is_durable(&self) -> bool {
        true
    }

    async fn read_offset(
        &self,
        projection_id: &str,
        tag: &EventTag,
        tenant: &str,
    ) -> Result<Option<Offset>, OffsetStoreError> {
        let stored: Option<i64> = sqlx::query_scalar(
            r#"SELECT offset_value FROM projection_offsets
               WHERE projection_id = $1 AND tag = $2 AND tenant = $3"#,
        )
        .bind(projection_id)
        .bind(tag.value())
        .bind(tenant)
        .fetch_optional(&self.pool)
        .await
        .map_err(offset_error)?;

        Ok(stored.map(Offset::Sequence))
    }

    async fn write_offset(
        &self,
        projection_id: &str,
        tag: &EventTag,
        tenant: &str,
        offset: &Offset,
    ) -> Result<(), OffsetStoreError> {
        sqlx::query(
            r#"INSERT INTO projection_offsets (projection_id, tag, tenant, offset_value)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (projection_id, tag, tenant)
               DO UPDATE SET offset_value = EXCLUDED.offset_value, updated_at = NOW()"#,
        )
        .bind(projection_id)
        .bind(tag.value())
        .bind(tenant)
        .bind(offset.as_sequence().expect("Offset has exactly one variant"))
        .execute(&self.pool)
        .await
        .map_err(offset_error)?;

        Ok(())
    }
}
