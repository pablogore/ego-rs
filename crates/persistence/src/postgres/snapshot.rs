//! PostgreSQL snapshot store.

use std::fmt;

use sqlx::FromRow;
use sqlx::PgPool;
use tokio::runtime::Handle;

use ego_domain::persistence::{PersistenceError, Snapshot};
use serde_json::Value;

use crate::postgres::resolve_tenant;

/// Row returned from the snapshots table.
#[derive(FromRow)]
#[expect(dead_code)]
struct SnapshotRow {
    id: i64,
    aggregate_id: String,
    tenant_id: Option<String>,
    version: i64,
    payload: Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// PostgreSQL snapshot store backed by a PgPool.
pub struct PostgreSQLSnapshotStore {
    pool: PgPool,
}

impl fmt::Debug for PostgreSQLSnapshotStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgreSQLSnapshotStore")
            .field("pool", &self.pool)
            .finish()
    }
}

impl PostgreSQLSnapshotStore {
    /// Constructor and helper methods for `PostgreSQLSnapshotStore`.
    /// Create a new PostgreSQL snapshot store with the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        tokio::task::block_in_place(|| Handle::current().block_on(future))
    }
}

impl Snapshot for PostgreSQLSnapshotStore {
    fn is_durable(&self) -> bool {
        true
    }

    fn save_snapshot(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        version: i64,
        payload: Value,
    ) -> Result<(), PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;

        // Check if a snapshot exists for this aggregate.
        //
        // `tenant_id IS NOT DISTINCT FROM $2`, never `= $2`: a systemwide
        // scope binds SQL NULL here, and `tenant_id = NULL` is never true
        // even against a row whose own `tenant_id` is also NULL. `IS NOT
        // DISTINCT FROM` treats two NULLs as equal while still comparing
        // non-NULL values with ordinary equality — the same pattern already
        // used throughout `event_store.rs` for the identical reason.
        let existing_version: Option<i64> = self
            .block_on(async {
                sqlx::query_scalar(
                    r#"SELECT version FROM snapshots \
                       WHERE aggregate_id = $1 AND tenant_id IS NOT DISTINCT FROM $2"#,
                )
                .bind(aggregate_id)
                .bind(tenant.clone())
                .fetch_optional(&self.pool)
                .await
            })
            .map_err(|e| {
                PersistenceError::Internal(format!("failed to query existing snapshot: {}", e))
            })?;

        // Only update if the new version is higher
        if let Some(existing) = existing_version {
            if version <= existing {
                return Ok(());
            }
        }

        // `ON CONFLICT` infers the partial index matching this row's own
        // tenant scope (migration 012: one partial unique index per side of
        // the `tenant_id IS NULL` split) — branching the statement is what
        // makes conflict inference exact for each side; a single
        // `ON CONFLICT (aggregate_id, tenant_id)` cannot target a partial
        // index at all, and a plain `WHERE tenant_id = $2` in the `DO
        // UPDATE` guard would go back to being always-false for the
        // systemwide case.
        let insert_result = match &tenant {
            Some(_) => {
                self.block_on(async {
                    sqlx::query(
                        r#"INSERT INTO snapshots (aggregate_id, tenant_id, version, payload, created_at)
                           VALUES ($1, $2, $3, $4, NOW())
                           ON CONFLICT (tenant_id, aggregate_id) WHERE tenant_id IS NOT NULL DO UPDATE
                           SET version = $3, payload = $4, created_at = NOW()"#,
                    )
                    .bind(aggregate_id)
                    .bind(tenant.clone())
                    .bind(version)
                    .bind(payload.clone())
                    .execute(&self.pool)
                    .await
                })
            }
            None => {
                self.block_on(async {
                    sqlx::query(
                        r#"INSERT INTO snapshots (aggregate_id, tenant_id, version, payload, created_at)
                           VALUES ($1, NULL, $2, $3, NOW())
                           ON CONFLICT (aggregate_id) WHERE tenant_id IS NULL DO UPDATE
                           SET version = $2, payload = $3, created_at = NOW()"#,
                    )
                    .bind(aggregate_id)
                    .bind(version)
                    .bind(payload)
                    .execute(&self.pool)
                    .await
                })
            }
        };
        insert_result
            .map_err(|e| PersistenceError::Internal(format!("failed to save snapshot: {}", e)))?;

        Ok(())
    }

    fn load_snapshot(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<(i64, Value)>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;

        let row: Option<SnapshotRow> = self
            .block_on(async {
                sqlx::query_as(
                    r#"SELECT id, aggregate_id, tenant_id, version, payload, created_at
                   FROM snapshots WHERE aggregate_id = $1 AND tenant_id IS NOT DISTINCT FROM $2
                   ORDER BY version DESC LIMIT 1"#,
                )
                .bind(aggregate_id)
                .bind(tenant)
                .fetch_optional(&self.pool)
                .await
            })
            .map_err(|e| PersistenceError::Internal(format!("failed to query snapshot: {}", e)))?;

        Ok(row.map(|r| (r.version, r.payload)))
    }
}
