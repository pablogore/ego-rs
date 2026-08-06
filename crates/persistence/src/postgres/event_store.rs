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
    aggregate_type: Option<String>,
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
    /// Opens a store against `pool`, refusing to open at all while any row is
    /// still missing its aggregate type.
    ///
    /// This is the only constructor, deliberately. Every read and the version
    /// check filter on the type column, so against a row that predates the
    /// split — type null, identifier still joined — neither filter matches: the
    /// comparison against null is never true and the joined text is not the bare
    /// identifier. A stream in that state reads as absent, the version check
    /// returns zero, and an append writes a **second, forked stream** while the
    /// original rows sit orphaned. There is no clean recovery from that once
    /// traffic has passed through.
    ///
    /// So the check runs here rather than living in a runbook step somebody has
    /// to remember. Wiring the store up in the wrong order — new binary before
    /// the backfill — produces a visible, recoverable startup failure instead of
    /// silent history divergence. It runs on **every** open, with no cached
    /// flag: a cached answer would go stale exactly when an old writer inserts
    /// one more untyped row mid-transition, which is the case worth catching.
    ///
    /// The cost is one existence query per open. That buys refusing to operate
    /// in a state where correctness is not achievable.
    pub async fn open(pool: PgPool, deserialize: F) -> Result<Self, PersistenceError> {
        let unmigrated: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE aggregate_type IS NULL)")
                .fetch_one(&pool)
                .await
                .map_err(|e| {
                    PersistenceError::Internal(format!(
                "could not determine whether the aggregate-type backfill has completed: {e}"
            ))
                })?;

        if unmigrated {
            return Err(PersistenceError::Internal(
                "refusing to open the event store: at least one row has no aggregate type,                  which means the backfill has not completed. Reading or writing now would                  treat existing streams as absent and fork their history. Run the backfill                  to completion first, then start this process."
                    .to_string(),
            ));
        }

        Ok(Self {
            pool,
            deserialize,
            _marker: PhantomData,
        })
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
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;
        let pool = self.pool.clone();
        let aggregate_type = aggregate_type.to_string();
        let aggregate_id = aggregate_id.to_string();

        self.block_on(async move {
            let mut tx = pool.begin().await.map_err(|e| {
                PersistenceError::Internal(format!("failed to begin transaction: {}", e))
            })?;

            let current: i64 = sqlx::query_scalar(
                // `tenant_id IS NOT DISTINCT FROM $3`, never `= $3`: the
                // systemwide mode binds SQL NULL here, and `tenant_id = NULL`
                // is unknown rather than true for every row — including the
                // rows whose tenant genuinely is NULL. With plain equality a
                // systemwide stream is invisible to its own version check, so
                // every append reads an empty history and writes version 1
                // again. IS NOT DISTINCT FROM treats two NULLs as equal while
                // still keeping NULL distinct from any concrete tenant, which
                // is what separates the systemwide partition from a tenant's.
                r#"SELECT COALESCE(MAX(version), 0) FROM events
                   WHERE aggregate_type = $1 AND aggregate_id = $2
                     AND tenant_id IS NOT DISTINCT FROM $3"#,
            )
            .bind(&aggregate_type)
            .bind(&aggregate_id)
            .bind(&tenant)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| PersistenceError::Internal(format!("failed to query current version: {}", e)))?;

            if current != expected_version {
                return Err(PersistenceError::Conflict {
                    aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
                    expected: expected_version,
                    actual: current,
                });
            }

            let new_version = current + events.len() as i64;

            for (i, stored) in events.iter().enumerate() {
                let event_version = current + (i as i64) + 1;
                let event = &stored.event;
                sqlx::query(
                    r#"INSERT INTO events (aggregate_type, aggregate_id, tenant_id, version, event_type, payload, created_at)
                       VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
                )
                .bind(&aggregate_type)
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
                                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
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
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;

        let rows: Vec<EventRow> = self
            .block_on(async {
                sqlx::query_as(
                    r#"SELECT aggregate_type, aggregate_id, tenant_id, version, event_type, payload, created_at
                   FROM events WHERE aggregate_type = $1 AND aggregate_id = $2
                     AND tenant_id IS NOT DISTINCT FROM $3
                   ORDER BY version ASC"#,
                )
                .bind(aggregate_type)
                .bind(aggregate_id)
                .bind(&tenant)
                .fetch_all(&self.pool)
                .await
            })
            .map_err(|e| PersistenceError::Internal(format!("failed to query events: {}", e)))?;

        if rows.is_empty() {
            return Err(PersistenceError::NotFound {
                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
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

    fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;

        // No filter excluding a NULL `aggregate_type`: there is nothing left to
        // exclude. `open` refuses to return a store while any row lacks its
        // type, and the backfill makes the column mandatory in the database, so
        // by the time this method can be called the column is non-null for every
        // row. A filter guarding against that would imply the store still admits
        // rows it cannot admit.
        let rows: Vec<(String, String)> = self
            .block_on(async {
                sqlx::query_as(
                    r#"SELECT DISTINCT aggregate_type, aggregate_id FROM events
                   WHERE tenant_id IS NOT DISTINCT FROM $1
                   ORDER BY aggregate_type, aggregate_id"#,
                )
                .bind(tenant)
                .fetch_all(&self.pool)
                .await
            })
            .map_err(|e| {
                PersistenceError::Internal(format!("failed to query aggregate ids: {}", e))
            })?;

        Ok(rows)
    }
}
