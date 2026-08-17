//! PostgreSQL event store.

use std::fmt;
use std::marker::PhantomData;

use async_trait::async_trait;
use sqlx::FromRow;
use sqlx::PgPool;

use ego_domain::context::TenantId;
use ego_domain::event::DomainEvent;
use ego_domain::operation::{
    AggregateOutcome, OperationFingerprint, OperationKey, OperationReceipt,
};
use ego_domain::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};

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
    operation_key: Option<String>,
}

/// PostgreSQL event store backed by a PgPool.
///
/// Requires a deserializer function to reconstruct events from what was stored.
///
/// # The deserializer is handed the envelope, not only the payload
///
/// It receives `(event_type, payload, occurred_at)`. The timestamp is the row's
/// `created_at`, which `append` writes from `event.occurred_at()` — so it is
/// when the event *happened*, never when it was read back, and never `now()`.
///
/// It is a parameter because `DomainEvent::payload()` is the event's **business
/// data** and `occurred_at` is **envelope metadata**: this store already
/// persists the latter in its own column, and an earlier signature simply threw
/// it away at read time, leaving every caller to synthesise a timestamp and
/// silently rewrite history on each replay.
///
/// Duplicating it into the payload instead would create two sources of truth for
/// one instant — the column and a JSON field, free to diverge — and would make
/// every domain compensate for an infrastructure limitation by changing its own
/// persisted format.
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
                "refusing to open the event store: at least one row has no aggregate type, \
                 which means the backfill has not completed. Reading or writing now would \
                 treat existing streams as absent and fork their history. Run the backfill \
                 to completion first, then start this process."
                    .to_string(),
            ));
        }

        Ok(Self {
            pool,
            deserialize,
            _marker: PhantomData,
        })
    }
}

#[async_trait]
impl<E, F> EventStore<E> for PostgreSQLEventStore<E, F>
where
    E: DomainEvent + Clone + Send + Sync + 'static,
    F: Fn(&str, serde_json::Value, chrono::DateTime<chrono::Utc>) -> Result<E, PersistenceError>
        + Send
        + Sync,
{
    async fn append(
        &self,
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
        .map_err(|e| {
            PersistenceError::Internal(format!("failed to query current version: {}", e))
        })?;

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
            let inserted = sqlx::query(
                    r#"INSERT INTO events (aggregate_type, aggregate_id, tenant_id, version, event_type, payload, created_at, operation_key)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
                )
                .bind(&aggregate_type)
                .bind(&aggregate_id)
                .bind(&tenant)
                .bind(event_version)
                .bind(event.event_type())
                .bind(event.payload().clone())
                .bind(*event.occurred_at())
            .bind(stored.operation_key.as_ref().map(|k| k.as_str()))
                .execute(&mut *tx)
                .await;

            if let Err(e) = inserted {
                let is_identity_collision = matches!(
                    &e,
                    sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505")
                );
                if !is_identity_collision {
                    return Err(PersistenceError::Internal(format!(
                        "failed to insert event: {e}"
                    )));
                }

                // The database refused a row at a version this transaction had
                // just read as free, which means another writer committed one
                // in between. That is a concurrency conflict, not an internal
                // error, and the caller's retry is the correct response.
                //
                // The version cannot be reported from `current`: the check
                // above already established that `current == expected_version`,
                // so reusing it would produce a conflict claiming the expected
                // and actual versions are the same — self-contradictory, and
                // useless to whoever has to act on it. This transaction is
                // aborted and can no longer be queried, so the stream is
                // re-read on another connection. That value is a reading taken
                // after the failure rather than at the instant of it, which is
                // the only thing "actual" can mean once a competing writer
                // exists.
                drop(tx);
                let actual: i64 = sqlx::query_scalar(
                    r#"SELECT COALESCE(MAX(version), 0) FROM events
                           WHERE aggregate_type = $1 AND aggregate_id = $2
                             AND tenant_id IS NOT DISTINCT FROM $3"#,
                )
                .bind(&aggregate_type)
                .bind(&aggregate_id)
                .bind(&tenant)
                .fetch_one(&pool)
                .await
                .map_err(|read_back| {
                    PersistenceError::Internal(format!(
                        "the stream identity was already taken ({e}), and re-reading the \
                             stream to report its current version also failed: {read_back}"
                    ))
                })?;

                return Err(PersistenceError::Conflict {
                    aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
                    expected: expected_version,
                    actual,
                });
            }
        }

        tx.commit().await.map_err(|e| {
            PersistenceError::Internal(format!("failed to commit transaction: {}", e))
        })?;

        Ok(new_version)
    }

    async fn load(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;

        let rows: Vec<EventRow> = sqlx::query_as(
                    r#"SELECT aggregate_type, aggregate_id, tenant_id, version, event_type, payload, created_at, operation_key
                   FROM events WHERE aggregate_type = $1 AND aggregate_id = $2
                     AND tenant_id IS NOT DISTINCT FROM $3
                   ORDER BY version ASC"#,
                )
        .bind(aggregate_type)
        .bind(aggregate_id)
        .bind(&tenant)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| PersistenceError::Internal(format!("failed to query events: {}", e)))?;

        if rows.is_empty() {
            return Err(PersistenceError::NotFound {
                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
            });
        }

        let events: Result<Vec<StoredEvent<E>>, PersistenceError> = rows
            .into_iter()
            .map(|row| {
                let event = (self.deserialize)(&row.event_type, row.payload, row.created_at)?;
                let stored = StoredEvent::without_correlation(event);
                // A stored key was validated on the way in, so failing to parse it
                // on the way out means the row was written by something that did
                // not go through `OperationKey`. Surfacing that rather than
                // dropping the key keeps the store from quietly returning an event
                // as though it had no operation behind it.
                match row.operation_key {
                    None => Ok(stored),
                    Some(raw) => {
                        let key = OperationKey::parse(raw.clone()).map_err(|e| {
                            PersistenceError::Internal(format!(
                                "stored operation_key {raw:?} is not a valid operation key: {e}"
                            ))
                        })?;
                        Ok(stored.with_operation_key(key))
                    }
                }
            })
            .collect();

        events
    }

    async fn list_aggregate_ids(
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
        let rows: Vec<(String, String)> = sqlx::query_as(
            r#"SELECT DISTINCT aggregate_type, aggregate_id FROM events
                   WHERE tenant_id IS NOT DISTINCT FROM $1
                   ORDER BY aggregate_type, aggregate_id"#,
        )
        .bind(tenant)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| PersistenceError::Internal(format!("failed to query aggregate ids: {}", e)))?;

        Ok(rows)
    }

    async fn find_receipt(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        operation_key: &str,
    ) -> Result<Option<OperationReceipt>, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;

        // `IS NOT DISTINCT FROM` rather than `=`, for the same reason every other
        // tenant-partitioned query here uses it: plain equality never matches SQL
        // NULL, so the systemwide partition would silently report every lookup as
        // a miss and re-run operations that already completed.
        let row: Option<(Option<String>, String, String, Option<i64>, Option<i64>)> =
            sqlx::query_as(
                r#"SELECT tenant_id, fingerprint, outcome_kind, version_from, version_to
               FROM operation_receipts
               WHERE aggregate_type = $1 AND aggregate_id = $2
                 AND tenant_id IS NOT DISTINCT FROM $3
                 AND operation_key = $4"#,
            )
            .bind(aggregate_type)
            .bind(aggregate_id)
            .bind(&tenant)
            .bind(operation_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| PersistenceError::Internal(format!("failed to read receipt: {e}")))?;

        let Some((stored_tenant, fingerprint, kind, version_from, version_to)) = row else {
            return Ok(None);
        };

        // The CHECK constraint makes these shapes unreachable from a conforming
        // writer. They are still mapped to an error rather than unwrapped: a
        // receipt this adapter cannot read must never be reported as absent,
        // because absence means "run the command", and running a command whose
        // record is merely unreadable duplicates exactly what the receipt exists
        // to prevent.
        let outcome = match (kind.as_str(), version_from, version_to) {
            ("no_events", None, None) => AggregateOutcome::NoEvents,
            ("events", Some(from), Some(to)) => {
                AggregateOutcome::events(from, to).map_err(|e| {
                    PersistenceError::Internal(format!(
                        "a stored receipt carries an invalid event range: {e:?}"
                    ))
                })?
            }
            _ => {
                return Err(PersistenceError::Internal(format!(
                    "a stored receipt carries an outcome this adapter cannot read: \
                     kind {kind:?}, range {version_from:?}..={version_to:?}"
                )))
            }
        };

        let tenant = match stored_tenant {
            Some(raw) => Some(TenantId::new(raw).map_err(|_| {
                PersistenceError::Internal(
                    "a stored receipt carries a tenant_id the domain rejects".to_string(),
                )
            })?),
            None => None,
        };
        let key = OperationKey::parse(operation_key).map_err(|e| {
            PersistenceError::Internal(format!("a stored receipt carries an invalid key: {e}"))
        })?;

        Ok(Some(OperationReceipt::new(
            aggregate_type,
            aggregate_id,
            tenant,
            key,
            OperationFingerprint::new(fingerprint),
            outcome,
        )))
    }

    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<E>>, PersistenceError> {
        let tx =
            self.pool.begin().await.map_err(|e| {
                PersistenceError::Internal(format!("failed to begin transaction: {e}"))
            })?;
        Ok(Box::new(PostgresEventStoreUnitOfWork {
            tx,
            pool: self.pool.clone(),
            _marker: PhantomData,
        }))
    }
}

/// A unit of work backed by one real PostgreSQL transaction.
///
/// Rollback-on-drop is not implemented here and is not a gap: `sqlx`'s
/// `Transaction` already rolls back when dropped without being committed, and
/// re-implementing that would mean tracking commit state a second time in order
/// to disagree with it.
///
/// This rests on `sqlx`'s own documented guarantee and is currently **not**
/// asserted here: the tests that proved it against a real transaction were moved
/// out of this workspace, and no in-process double can stand in for one. See
/// `docs/integration-test-backlog.md` for the properties awaiting reconstruction.
pub struct PostgresEventStoreUnitOfWork<E> {
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
    /// A separate handle, used only to re-read a stream's version after the
    /// database refuses a duplicate identity. That refusal aborts `tx`, which can
    /// no longer answer questions, and reporting a conflict without the real
    /// version is what the direct append path was corrected for.
    pool: PgPool,
    _marker: PhantomData<E>,
}

#[async_trait]
impl<E> EventStoreUnitOfWork<E> for PostgresEventStoreUnitOfWork<E>
where
    E: DomainEvent + Clone + Send + Sync + 'static,
{
    async fn append(
        &mut self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let tenant = resolve_tenant(tenant_id)?;

        // Reads its own uncommitted writes, which is what makes two appends to
        // one stream inside a single unit of work advance rather than collide.
        let current: i64 = sqlx::query_scalar(
            r#"SELECT COALESCE(MAX(version), 0) FROM events
               WHERE aggregate_type = $1 AND aggregate_id = $2
                 AND tenant_id IS NOT DISTINCT FROM $3"#,
        )
        .bind(aggregate_type)
        .bind(aggregate_id)
        .bind(&tenant)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(|e| PersistenceError::Internal(format!("failed to query current version: {e}")))?;

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
            let inserted = sqlx::query(
                r#"INSERT INTO events (aggregate_type, aggregate_id, tenant_id, version, event_type, payload, created_at, operation_key)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
            )
            .bind(aggregate_type)
            .bind(aggregate_id)
            .bind(&tenant)
            .bind(event_version)
            .bind(event.event_type())
            .bind(event.payload().clone())
            .bind(*event.occurred_at())
            .bind(stored.operation_key.as_ref().map(|k| k.as_str()))
            .execute(&mut *self.tx)
            .await;

            if let Err(e) = inserted {
                let is_identity_collision = matches!(
                    &e,
                    sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505")
                );
                if !is_identity_collision {
                    return Err(PersistenceError::Internal(format!(
                        "failed to insert event: {e}"
                    )));
                }

                // Same reasoning as the direct append path: `current` was just
                // proven equal to `expected_version`, so reporting it would
                // produce a conflict claiming the two are the same number. This
                // transaction is aborted and cannot be queried, so the stream is
                // re-read on another connection — a reading taken after the
                // failure, which is the only thing "actual" can mean once a
                // competing writer exists.
                let actual: i64 = sqlx::query_scalar(
                    r#"SELECT COALESCE(MAX(version), 0) FROM events
                       WHERE aggregate_type = $1 AND aggregate_id = $2
                         AND tenant_id IS NOT DISTINCT FROM $3"#,
                )
                .bind(aggregate_type)
                .bind(aggregate_id)
                .bind(&tenant)
                .fetch_one(&self.pool)
                .await
                .map_err(|read_back| {
                    PersistenceError::Internal(format!(
                        "the stream identity was already taken ({e}), and re-reading the stream \
                         to report its current version also failed: {read_back}"
                    ))
                })?;

                return Err(PersistenceError::Conflict {
                    aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
                    expected: expected_version,
                    actual,
                });
            }
        }

        Ok(new_version)
    }

    async fn confirm_receipt(
        &mut self,
        receipt: &OperationReceipt,
    ) -> Result<(), PersistenceError> {
        let tenant = receipt.tenant().map(|t| t.as_str().to_string());
        let key = receipt.operation_key().as_str();

        // `ON CONFLICT DO NOTHING` rather than letting the unique violation
        // surface, and that choice is load-bearing rather than stylistic.
        //
        // A raw 23505 **aborts this transaction**, after which nothing further
        // can be read from it — including the fingerprint of the row that won.
        // An implementation that treated every violation as a conflict would
        // therefore refuse an ordinary concurrent retry of the *same* request,
        // which is precisely the case idempotency exists to serve. Swallowing
        // the conflict keeps the transaction alive so the winning row can be
        // read and compared.
        //
        // No conflict target is named: the identity is enforced by the AD-1
        // complementary partial pair, and a bare `DO NOTHING` covers whichever
        // of the two a given row falls under. Naming one would miss the other
        // partition — the systemwide one, where a duplicate is least visible.
        //
        // A competing writer that has inserted but not committed blocks this
        // statement until it resolves. That wait is correct: the answer to
        // "does this identity exist?" is not knowable until the other
        // transaction commits or rolls back.
        let (kind, version_from, version_to) = match receipt.outcome() {
            AggregateOutcome::NoEvents => ("no_events", None, None),
            AggregateOutcome::Events {
                version_from,
                version_to,
            } => ("events", Some(*version_from), Some(*version_to)),
        };

        let inserted = sqlx::query(
            r#"INSERT INTO operation_receipts
                   (tenant_id, aggregate_type, aggregate_id, operation_key, fingerprint,
                    outcome_kind, version_from, version_to)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&tenant)
        .bind(receipt.aggregate_type())
        .bind(receipt.aggregate_id())
        .bind(key)
        .bind(receipt.fingerprint().as_str())
        .bind(kind)
        .bind(version_from)
        .bind(version_to)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| PersistenceError::Internal(format!("failed to write receipt: {e}")))?;

        // No commit anywhere in this method, deliberately: it stages. Committing
        // would make the receipt durable ahead of the events it describes, which
        // is the exact split the unit of work exists to prevent.
        if inserted.rows_affected() > 0 {
            return Ok(());
        }

        // Nothing was written, so a row for this identity already exists —
        // either committed before this transaction opened, or staged earlier
        // within it. Both are readable here, and only the fingerprint decides
        // which of the two answers is right.
        let existing: Option<(String,)> = sqlx::query_as(
            r#"SELECT fingerprint FROM operation_receipts
               WHERE aggregate_type = $1 AND aggregate_id = $2
                 AND tenant_id IS NOT DISTINCT FROM $3
                 AND operation_key = $4"#,
        )
        .bind(receipt.aggregate_type())
        .bind(receipt.aggregate_id())
        .bind(&tenant)
        .bind(key)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(|e| {
            PersistenceError::Internal(format!("failed to read the winning receipt: {e}"))
        })?;

        match existing {
            // The same request, arriving twice. Idempotent success: the stored
            // row already says what this call was going to say.
            Some((stored,)) if stored == receipt.fingerprint().as_str() => Ok(()),
            // A different request reusing an operation key. Refused, never
            // overwritten — replacing it would hand one caller another caller's
            // stored result.
            Some(_) => Err(PersistenceError::Conflict {
                aggregate_id: format!("{}-{}", receipt.aggregate_type(), receipt.aggregate_id()),
                expected: 0,
                actual: 0,
            }),
            // The insert was suppressed by a conflict, so a row exists; a read
            // that then finds none means the identity this query builds does not
            // match the one the indexes enforce. Reporting success would confirm
            // a receipt nobody can look up.
            None => Err(PersistenceError::Internal(
                "a receipt insert conflicted with a row the equivalent lookup cannot find; \
                 the write and read identities disagree"
                    .to_string(),
            )),
        }
    }

    async fn commit(self: Box<Self>) -> Result<(), PersistenceError> {
        self.tx
            .commit()
            .await
            .map_err(|e| PersistenceError::Internal(format!("failed to commit transaction: {e}")))
    }
}
