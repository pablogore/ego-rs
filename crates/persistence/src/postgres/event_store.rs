//! PostgreSQL event store.

use std::fmt;
use std::marker::PhantomData;

use sqlx::FromRow;
use sqlx::PgPool;
use tokio::runtime::Handle;

use ego_domain::event::DomainEvent;
use ego_domain::persistence::{EventStore, PersistenceError};

/// Row returned from the events table.
#[derive(FromRow)]
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
        f.debug_struct("PostgreSQLEventStore").field("pool", &self.pool).finish()
    }
}

impl<E, F> PostgreSQLEventStore<E, F> {
    /// Constructor and helper methods for `PostgreSQLEventStore`.

    /// Create a new PostgreSQL event store with the given connection pool and deserializer.
    pub fn new(pool: PgPool, deserialize: F) -> Self {
        Self {
            pool,
            deserialize,
            _marker: PhantomData,
        }
    }

    fn block_on<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        Handle::current().block_on(future)
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
        events: Vec<E>,
    ) -> Result<i64, PersistenceError> {
        let tenant = match tenant_id {
            Some("") => None,
            Some(t) => Some(t.to_string()),
            None => None,
        };

        let current_version: Option<i64> = self.block_on(async {
            sqlx::query_scalar(
                r#"SELECT COALESCE(MAX(version), 0) FROM events WHERE aggregate_id = $1 AND tenant_id = $2"#,
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

        let new_version = current + events.len() as i64;

        for (i, event) in events.iter().enumerate() {
            let event_version = current + (i as i64) + 1;
            self.block_on(async {
                sqlx::query(
                    r#"INSERT INTO events (aggregate_id, tenant_id, version, event_type, payload, created_at)
                       VALUES ($1, $2, $3, $4, $5, $6)"#,
                )
                .bind(aggregate_id)
                .bind(&tenant)
                .bind(event_version)
                .bind(event.event_type())
                .bind(event.payload().clone())
                .bind(*event.occurred_at())
                .execute(&self.pool)
                .await
            })
            .map_err(|e| PersistenceError::Internal(format!("failed to insert event: {}", e)))?;
        }

        Ok(new_version)
    }

    fn load(
        &self,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<E>, PersistenceError> {
        let tenant = match tenant_id {
            Some("") => None,
            Some(t) => Some(t.to_string()),
            None => None,
        };

        let rows: Vec<EventRow> = self.block_on(async {
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

        let events: Result<Vec<E>, PersistenceError> = rows
            .into_iter()
            .map(|row| (self.deserialize)(&row.event_type, row.payload))
            .collect();

        events
    }

    fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<String>, PersistenceError> {
        let tenant = match tenant_id {
            Some("") => None,
            Some(t) => Some(t.to_string()),
            None => None,
        };

        let rows: Vec<(String,)> = self.block_on(async {
            sqlx::query_as(
                r#"SELECT DISTINCT aggregate_id FROM events
                   WHERE tenant_id = $1
                   ORDER BY aggregate_id"#,
            )
            .bind(tenant)
            .fetch_all(&self.pool)
            .await
        })
        .map_err(|e| PersistenceError::Internal(format!("failed to query aggregate ids: {}", e)))?;

        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}
