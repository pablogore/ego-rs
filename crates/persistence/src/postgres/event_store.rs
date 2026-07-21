//! PostgreSQL event store.

use std::fmt;
use std::marker::PhantomData;

use sqlx::FromRow;
use sqlx::PgPool;
use tokio::runtime::Handle;

use ego_domain::event::DomainEvent;
use ego_domain::persistence::{EventStore, PersistenceError, StoredEvent};

use crate::postgres::resolve_tenant;

/// Row returned from the events table.
#[derive(FromRow)]
#[expect(dead_code)]
struct EventRow {
    aggregate_id: String,
    tenant_id: Option<String>,
    version: i64,
    event_type: String,
    payload: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// PostgreSQL event store backed by a PgPool.
///
/// Requires a deserializer function to reconstruct events from stored JSON.
pub struct PostgreSQLEventStore<E, F> {
    pool: PgPool,
    deserialize: F,
    _marker: PhantomData<E>,
}

impl<E, F> fmt::Debug for PostgreSQLEventStore<E, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgreSQLEventStore")
            .field("pool", &self.pool)
            .finish()
    }
}

impl<E, F> PostgreSQLEventStore<E, F> {
    /// Create a new PostgreSQL event store with the given connection pool and deserializer.
    pub fn new(pool: PgPool, deserialize: F) -> Self {
        Self {
            pool,
            deserialize,
            _marker: PhantomData,
        }
    }

    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        tokio::task::block_in_place(|| Handle::current().block_on(future))
    }
}

impl<E, F> EventStore<E> for PostgreSQLEventStore<E, F>
where
    E: DomainEvent + Clone + Send + 'static,
    F: Fn(&str, serde_json::Value) -> Result<E, PersistenceError> + Send + Sync,
{
    fn append(
        &mut self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let pool = self.pool.clone();
        let aggregate_id = aggregate_id.to_string();

        self.block_on(async move {
            let mut tx = pool.begin().await.map_err(|e| {
                PersistenceError::Internal(format!("failed to begin transaction: {}", e))
            })?;

            let current: i64 = sqlx::query_scalar(
                r#"SELECT COALESCE(MAX(version), 0) FROM events WHERE aggregate_id = $1 AND tenant_id = $2"#,
            )
            .bind(&aggregate_id)
            .bind(&tenant)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| PersistenceError::Internal(format!("failed to query current version: {}", e)))?;

            if current != expected_version {
                return Err(PersistenceError::Conflict {
                    aggregate_id: aggregate_id.clone(),
                    expected: expected_version,
                    actual: current,
                });
            }

            let new_version = current + events.len() as i64;

            for (i, stored) in events.iter().enumerate() {
                let event_version = current + (i as i64) + 1;
                let event = &stored.event;
                sqlx::query(
                    r#"INSERT INTO events (aggregate_id, tenant_id, version, event_type, payload, created_at)
                       VALUES ($1, $2, $3, $4, $5, $6)"#,
                )
                .bind(&aggregate_id)
                .bind(&tenant)
                .bind(event_version)
                .bind(event.event_type())
                .bind(event.payload().clone())
                .bind(*event.occurred_at())
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    if let sqlx::Error::Database(db_err) = &e {
                        if db_err.code().as_deref() == Some("23505") {
                            return PersistenceError::Conflict {
                                aggregate_id: aggregate_id.clone(),
                                expected: expected_version,
                                actual: current,
                            };
                        }
                    }
                    PersistenceError::Internal(format!("failed to insert event: {}", e))
                })?;
            }

            tx.commit().await.map_err(|e| {
                PersistenceError::Internal(format!("failed to commit transaction: {}", e))
            })?;

            Ok(new_version)
        })
    }

    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;

        let rows: Vec<EventRow> = self
            .block_on(async {
                sqlx::query_as(
                    r#"SELECT aggregate_id, tenant_id, version, event_type, payload, created_at
                   FROM events WHERE aggregate_id = $1 AND tenant_id = $2
                   ORDER BY version ASC"#,
                )
                .bind(aggregate_id)
                .bind(&tenant)
                .fetch_all(&self.pool)
                .await
            })
            .map_err(|e| PersistenceError::Internal(format!("failed to query events: {}", e)))?;

        if rows.is_empty() {
            return Err(PersistenceError::NotFound {
                aggregate_id: aggregate_id.to_string(),
            });
        }

        let events: Result<Vec<StoredEvent<E>>, PersistenceError> = rows
            .into_iter()
            .map(|row| {
                let event = (self.deserialize)(&row.event_type, row.payload)?;
                Ok(StoredEvent::without_correlation(event))
            })
            .collect();

        events
    }

    fn list_aggregate_ids(&self, tenant_id: Option<&str>) -> Result<Vec<String>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;

        let rows: Vec<(String,)> = self
            .block_on(async {
                sqlx::query_as(
                    r#"SELECT DISTINCT aggregate_id FROM events
                   WHERE tenant_id = $1
                   ORDER BY aggregate_id"#,
                )
                .bind(tenant)
                .fetch_all(&self.pool)
                .await
            })
            .map_err(|e| {
                PersistenceError::Internal(format!("failed to query aggregate ids: {}", e))
            })?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}
