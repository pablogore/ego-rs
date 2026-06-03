//! PostgreSQL repository.

use std::fmt;

use sqlx::FromRow;
use sqlx::PgPool;
use tokio::runtime::Handle;

use ego_domain::persistence::{PersistenceError, Repository};

/// Row returned from the aggregates table.
#[derive(FromRow)]
struct AggregateRow {
    aggregate_id: String,
    tenant_id: Option<String>,
    version: i64,
    payload: serde_json::Value,
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// PostgreSQL repository backed by a PgPool.
///
/// Requires a deserializer function to reconstruct aggregates from stored JSON.
pub struct PostgreSQLRepository<A, F> {
    pool: PgPool,
    deserialize: F,
    _marker: std::marker::PhantomData<A>,
}

impl<A, F> fmt::Debug for PostgreSQLRepository<A, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgreSQLRepository").field("pool", &self.pool).finish()
    }
}

impl<A, F> PostgreSQLRepository<A, F> {
    /// Constructor and helper methods for `PostgreSQLRepository`.

    /// Create a new PostgreSQL repository with the given connection pool and deserializer.
    pub fn new(pool: PgPool, deserialize: F) -> Self {
        Self {
            pool,
            deserialize,
            _marker: std::marker::PhantomData,
        }
    }

    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        Handle::current().block_on(future)
    }
}

impl<A, F> Repository<A> for PostgreSQLRepository<A, F>
where
    A: Clone + serde::Serialize,
    F: Fn(serde_json::Value) -> Result<A, PersistenceError> + Send + Sync,
{
    fn save(
        &mut self,
        aggregate_id: &str,
        aggregate: A,
        tenant_id: Option<&str>,
        expected_version: i64,
    ) -> Result<i64, PersistenceError> {
        let tenant = match tenant_id {
            Some("") => None,
            Some(t) => Some(t.to_string()),
            None => None,
        };

        let payload = serde_json::to_value(&aggregate)
            .map_err(|e| PersistenceError::Internal(format!("failed to serialize aggregate: {}", e)))?;

        let is_new: bool = self.block_on(async {
            sqlx::query_scalar(
                r#"SELECT NOT EXISTS(SELECT 1 FROM aggregates WHERE aggregate_id = $1 AND tenant_id = $2)"#,
            )
            .bind(aggregate_id)
            .bind(&tenant)
            .fetch_one(&self.pool)
            .await
        })
        .map_err(|e| PersistenceError::Internal(format!("failed to check existence: {}", e)))?;

        if !is_new {
            let current_version: Option<i64> = self.block_on(async {
                sqlx::query_scalar(
                    r#"SELECT version FROM aggregates WHERE aggregate_id = $1 AND tenant_id = $2"#,
                )
                .bind(aggregate_id)
                .bind(&tenant)
                .fetch_optional(&self.pool)
                .await
            })
            .map_err(|e| PersistenceError::Internal(format!("failed to query current version: {}", e)))?;

            let current = current_version.unwrap_or(0);
            if current != expected_version {
                return Err(PersistenceError::Conflict {
                    aggregate_id: aggregate_id.to_string(),
                    expected: expected_version,
                    actual: current,
                });
            }
        }

        let new_version = if is_new { 1 } else { expected_version + 1 };

        self.block_on(async {
            sqlx::query(
                r#"INSERT INTO aggregates (aggregate_id, tenant_id, version, payload, updated_at)
                   VALUES ($1, $2, $3, $4, NOW())
                   ON CONFLICT (aggregate_id, tenant_id) DO UPDATE
                   SET version = $3, payload = $4, updated_at = NOW()
                   WHERE aggregates.aggregate_id = $1 AND aggregates.tenant_id = $2"#,
            )
            .bind(aggregate_id)
            .bind(&tenant)
            .bind(new_version)
            .bind(payload)
            .execute(&self.pool)
            .await
        })
        .map_err(|e| PersistenceError::Internal(format!("failed to save aggregate: {}", e)))?;

        Ok(new_version)
    }

    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<A, PersistenceError> {
        let tenant = match tenant_id {
            Some("") => None,
            Some(t) => Some(t.to_string()),
            None => None,
        };

        let row: AggregateRow = self.block_on(async {
            sqlx::query_as(
                r#"SELECT aggregate_id, tenant_id, version, payload, updated_at
                   FROM aggregates WHERE aggregate_id = $1 AND tenant_id = $2"#,
            )
            .bind(aggregate_id)
            .bind(&tenant)
            .fetch_one(&self.pool)
            .await
        })
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => PersistenceError::NotFound {
                aggregate_id: aggregate_id.to_string(),
            },
            _ => PersistenceError::Internal(format!("failed to query aggregate: {}", e)),
        })?;

        (self.deserialize)(row.payload)
    }

    fn delete(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<(), PersistenceError> {
        let tenant = match tenant_id {
            Some("") => None,
            Some(t) => Some(t.to_string()),
            None => None,
        };

        let deleted = self.block_on(async {
            sqlx::query(
                r#"DELETE FROM aggregates WHERE aggregate_id = $1 AND tenant_id = $2"#,
            )
            .bind(aggregate_id)
            .bind(&tenant)
            .execute(&self.pool)
            .await
        })
        .map_err(|e| PersistenceError::Internal(format!("failed to delete aggregate: {}", e)))?;

        if deleted.rows_affected() == 0 {
            return Err(PersistenceError::NotFound {
                aggregate_id: aggregate_id.to_string(),
            });
        }

        Ok(())
    }
}
