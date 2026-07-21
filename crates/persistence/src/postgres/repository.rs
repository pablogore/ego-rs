//! PostgreSQL repository.

use std::fmt;

use sqlx::FromRow;
use sqlx::PgPool;
use tokio::runtime::Handle;

use ego_domain::persistence::{PersistenceError, Repository};

use crate::postgres::resolve_tenant;

/// Row returned from the aggregates table.
#[derive(FromRow)]
#[expect(dead_code)]
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
        f.debug_struct("PostgreSQLRepository")
            .field("pool", &self.pool)
            .finish()
    }
}

impl<A, F> PostgreSQLRepository<A, F> {
    /// Create a new PostgreSQL repository with the given connection pool and deserializer.
    pub fn new(pool: PgPool, deserialize: F) -> Self {
        Self {
            pool,
            deserialize,
            _marker: std::marker::PhantomData,
        }
    }

    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        tokio::task::block_in_place(|| Handle::current().block_on(future))
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
        let tenant = resolve_tenant(tenant_id)?;
        let payload = serde_json::to_value(&aggregate).map_err(|e| {
            PersistenceError::Internal(format!("failed to serialize aggregate: {}", e))
        })?;
        let pool = self.pool.clone();
        let aggregate_id = aggregate_id.to_string();

        self.block_on(async move {
            let mut tx = pool.begin().await.map_err(|e| {
                PersistenceError::Internal(format!("failed to begin transaction: {}", e))
            })?;

            // Lock the row for update to prevent concurrent version bypasses.
            let current_version: Option<i64> = sqlx::query_scalar(
                r#"SELECT version FROM aggregates WHERE aggregate_id = $1 AND tenant_id = $2 FOR UPDATE"#,
            )
            .bind(&aggregate_id)
            .bind(&tenant)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| {
                PersistenceError::Internal(format!("failed to query current version: {}", e))
            })?;

            let new_version = match current_version {
                None => 1,
                Some(current) => {
                    if current != expected_version {
                        return Err(PersistenceError::Conflict {
                            aggregate_id: aggregate_id.clone(),
                            expected: expected_version,
                            actual: current,
                        });
                    }
                    expected_version + 1
                }
            };

            sqlx::query(
                r#"INSERT INTO aggregates (aggregate_id, tenant_id, version, payload, updated_at)
                   VALUES ($1, $2, $3, $4, NOW())
                   ON CONFLICT (aggregate_id, tenant_id) DO UPDATE
                   SET version = $3, payload = $4, updated_at = NOW()"#,
            )
            .bind(&aggregate_id)
            .bind(&tenant)
            .bind(new_version)
            .bind(&payload)
            .execute(&mut *tx)
            .await
            .map_err(|e| PersistenceError::Internal(format!("failed to save aggregate: {}", e)))?;

            tx.commit().await.map_err(|e| {
                PersistenceError::Internal(format!("failed to commit transaction: {}", e))
            })?;

            Ok(new_version)
        })
    }

    fn load(&self, aggregate_id: &str, tenant_id: Option<&str>) -> Result<A, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;

        let row: AggregateRow = self
            .block_on(async {
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
        let tenant = resolve_tenant(tenant_id)?;

        let deleted = self
            .block_on(async {
                sqlx::query(r#"DELETE FROM aggregates WHERE aggregate_id = $1 AND tenant_id = $2"#)
                    .bind(aggregate_id)
                    .bind(&tenant)
                    .execute(&self.pool)
                    .await
            })
            .map_err(|e| {
                PersistenceError::Internal(format!("failed to delete aggregate: {}", e))
            })?;

        if deleted.rows_affected() == 0 {
            return Err(PersistenceError::NotFound {
                aggregate_id: aggregate_id.to_string(),
            });
        }

        Ok(())
    }
}
