//! Stoolap-backed implementation of `ego_persistence_api::persistence::EventStore<E>`.
//!
//! `#[async_trait]`, unlike `Repository<A>`/`Snapshot` (S1), so every method
//! bridges Stoolap's synchronous `Database` API to async via
//! `tokio::task::spawn_blocking` — copied in shape from
//! `StoolapEffectStore::run_blocking` (`effect-store/src/stoolap/mod.rs`),
//! never `block_in_place` (design.md AD-2: `block_in_place` panics outside a
//! multi-threaded runtime, and this module's own tests run under plain
//! `#[tokio::test]`, Tokio's current-thread flavor).
//!
//! Reused from S1 (`stoolap_common`, design.md AD-2): `dsn_for`,
//! `SYSTEMWIDE_SCOPE`/`encode_tenant`, `internal_err`, `is_write_conflict`.
//! Not reused: S1's synchronous read-check-write `save()` body — this store's
//! shape (owned rows built before crossing into `spawn_blocking`, a fresh
//! `Database` clone re-read for a lost race) instead follows
//! `PostgreSQLEventStore` (`persistence/src/postgres/event_store.rs`), the
//! `EventStore<E>` reference implementation.

use std::fmt;
use std::marker::PhantomData;
use std::path::Path;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use stoolap::Database;

use ego_persistence_api::context::TenantId;
use ego_persistence_api::event::DomainEvent;
use ego_persistence_api::operation::key::OperationFingerprint;
use ego_persistence_api::operation::receipt::AggregateOutcome;
use ego_persistence_api::operation::{OperationKey, OperationReceipt};
use ego_persistence_api::persistence::{
    resolve_tenant, EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent,
};

use crate::persistence::stoolap_common::{dsn_for, encode_tenant, internal_err, is_write_conflict};

const CREATE_EVENTS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS events (
    tenant_id      TEXT    NOT NULL,
    aggregate_type TEXT    NOT NULL,
    aggregate_id   TEXT    NOT NULL,
    version        INTEGER NOT NULL,
    event_type     TEXT    NOT NULL,
    payload        TEXT    NOT NULL,
    occurred_at    TIMESTAMP NOT NULL,
    operation_key  TEXT,
    UNIQUE (tenant_id, aggregate_type, aggregate_id, version)
)";

const CREATE_RECEIPTS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS operation_receipts (
    tenant_id      TEXT    NOT NULL,
    aggregate_type TEXT    NOT NULL,
    aggregate_id   TEXT    NOT NULL,
    operation_key  TEXT    NOT NULL,
    fingerprint    TEXT    NOT NULL,
    outcome_kind   TEXT    NOT NULL,
    version_from   INTEGER,
    version_to     INTEGER,
    UNIQUE (tenant_id, aggregate_type, aggregate_id, operation_key)
)";

const SELECT_MAX_VERSION: &str = "SELECT COALESCE(MAX(version), 0) FROM events \
     WHERE tenant_id = $1 AND aggregate_type = $2 AND aggregate_id = $3";
const INSERT_EVENT: &str = "INSERT INTO events \
     (tenant_id, aggregate_type, aggregate_id, version, event_type, payload, occurred_at, operation_key) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";
const SELECT_EVENTS: &str = "SELECT version, event_type, payload, occurred_at, operation_key \
     FROM events WHERE tenant_id = $1 AND aggregate_type = $2 AND aggregate_id = $3 \
     ORDER BY version ASC";
const SELECT_AGGREGATE_IDS: &str = "SELECT DISTINCT aggregate_type, aggregate_id FROM events \
     WHERE tenant_id = $1 ORDER BY aggregate_type, aggregate_id";
const SELECT_RECEIPT: &str = "SELECT fingerprint, outcome_kind, version_from, version_to \
     FROM operation_receipts \
     WHERE tenant_id = $1 AND aggregate_type = $2 AND aggregate_id = $3 AND operation_key = $4";
const INSERT_RECEIPT: &str = "INSERT INTO operation_receipts \
     (tenant_id, aggregate_type, aggregate_id, operation_key, fingerprint, outcome_kind, version_from, version_to) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";

/// One event, reduced to owned, `Send + 'static` fields before crossing into
/// `spawn_blocking` — `event.event_type()`/`payload()`/`occurred_at()` are
/// plain, non-blocking trait calls, so they run on the calling task rather
/// than inside the blocking closure, and the closure itself never needs `E`
/// to be `Send`/`'static` beyond what `DomainEvent` already requires.
struct EventRow {
    event_type: String,
    payload: String,
    occurred_at: DateTime<Utc>,
    operation_key: Option<String>,
}

fn to_event_row<E: DomainEvent>(stored: &StoredEvent<E>) -> Result<EventRow, PersistenceError> {
    Ok(EventRow {
        event_type: stored.event.event_type().to_string(),
        payload: serde_json::to_string(stored.event.payload()).map_err(internal_err)?,
        occurred_at: *stored.event.occurred_at(),
        operation_key: stored
            .operation_key
            .as_ref()
            .map(|k| k.as_str().to_string()),
    })
}

fn conflict(
    aggregate_type: &str,
    aggregate_id: &str,
    expected: i64,
    actual: i64,
) -> PersistenceError {
    PersistenceError::Conflict {
        aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
        expected,
        actual,
    }
}

fn read_max_version(
    db: &Database,
    scope: &str,
    aggregate_type: &str,
    aggregate_id: &str,
) -> Result<i64, PersistenceError> {
    Ok(db
        .query_opt(SELECT_MAX_VERSION, (scope, aggregate_type, aggregate_id))
        .map_err(internal_err)?
        .unwrap_or(0))
}

/// Inserts `rows` as a contiguous run of versions starting at `expected_version + 1`,
/// inside one transaction ending in exactly one `tx.commit()` (design.md AD-3.4).
///
/// `tx` is a live, already-open transaction so this can be shared by both the
/// standalone `EventStore::append` (its own self-contained transaction) and
/// `EventStoreUnitOfWork::append` (a transaction spanning multiple calls) —
/// only the caller decides when to commit.
fn append_into_tx(
    tx: &mut stoolap::ApiTransaction,
    scope: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    expected_version: i64,
    rows: &[EventRow],
) -> Result<i64, PersistenceError> {
    let current: i64 = tx
        .query_opt(SELECT_MAX_VERSION, (scope, aggregate_type, aggregate_id))
        .map_err(internal_err)?
        .unwrap_or(0);

    if current != expected_version {
        return Err(conflict(
            aggregate_type,
            aggregate_id,
            expected_version,
            current,
        ));
    }

    for (i, row) in rows.iter().enumerate() {
        let version = current + i as i64 + 1;
        tx.execute(
            INSERT_EVENT,
            (
                scope,
                aggregate_type,
                aggregate_id,
                version,
                row.event_type.as_str(),
                row.payload.as_str(),
                row.occurred_at,
                row.operation_key.as_deref(),
            ),
        )
        .map_err(internal_err)?;
    }

    Ok(current + rows.len() as i64)
}

/// Stoolap-backed implementation of `EventStore<E>`.
///
/// `F` deserializes a stored `(event_type, payload, occurred_at)` triple back
/// into `E` — the same dialect `PostgreSQLEventStore` uses.
pub struct StoolapEventStore<E, F> {
    db: Database,
    deserialize: F,
    _marker: PhantomData<E>,
}

impl<E, F> fmt::Debug for StoolapEventStore<E, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoolapEventStore")
            .field("dsn", &self.db.dsn())
            .finish()
    }
}

impl<E, F> StoolapEventStore<E, F>
where
    F: Fn(&str, serde_json::Value, DateTime<Utc>) -> Result<E, PersistenceError>,
{
    /// Opens (creating the `events`/`operation_receipts` tables if absent) a
    /// Stoolap-backed event store at `path`.
    ///
    /// Fails closed (design.md AD-3 criterion 2), exactly like
    /// `StoolapSnapshotStore::open`: only ever returns a store whose live
    /// engine reports `sync=full`, so `is_durable()` never outlives-lies
    /// about how the store was opened.
    pub async fn open(path: &Path, deserialize: F) -> Result<Self, PersistenceError> {
        let dsn = dsn_for(path);
        let db = Database::open(&dsn).map_err(internal_err)?;

        if !db.dsn().contains("sync=full") {
            return Err(PersistenceError::Internal(format!(
                "stoolap engine at {:?} is not configured for durable sync (sync=full); \
                 refusing to open an EventStore that would misreport is_durable()",
                db.dsn()
            )));
        }

        db.execute(CREATE_EVENTS_TABLE, ()).map_err(internal_err)?;
        db.execute(CREATE_RECEIPTS_TABLE, ())
            .map_err(internal_err)?;

        Ok(Self {
            db,
            deserialize,
            _marker: PhantomData,
        })
    }

    /// Runs `f` against a cloned `Database` handle on Tokio's blocking-thread
    /// pool (design.md AD-2 — the `StoolapEffectStore::run_blocking` shape).
    async fn run_blocking<T, R>(&self, f: T) -> Result<R, PersistenceError>
    where
        T: FnOnce(&Database) -> Result<R, PersistenceError> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || f(&db))
            .await
            .map_err(|e| PersistenceError::Internal(format!("blocking task panicked: {e}")))?
    }

    #[cfg(test)]
    fn dsn(&self) -> &str {
        self.db.dsn()
    }
}

#[async_trait]
impl<E, F> EventStore<E> for StoolapEventStore<E, F>
where
    // `'static`: `EventStore::begin`'s `Box<dyn EventStoreUnitOfWork<E>>` return
    // type elides to `Box<dyn EventStoreUnitOfWork<E> + 'static>`, so every
    // implementor needs it — not a bound this adapter invents.
    E: DomainEvent + 'static,
    F: Fn(&str, serde_json::Value, DateTime<Utc>) -> Result<E, PersistenceError> + Send + Sync,
{
    /// Truthful by construction (design.md AD-3 criterion 3), like
    /// `StoolapSnapshotStore::is_durable`.
    fn is_durable(&self) -> bool {
        self.db.dsn().contains("sync=full")
    }

    async fn append(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let resolved = resolve_tenant(tenant_id)?;
        let scope = encode_tenant(resolved.as_deref()).to_string();
        let aggregate_type = aggregate_type.to_string();
        let aggregate_id = aggregate_id.to_string();
        let rows = events
            .iter()
            .map(to_event_row)
            .collect::<Result<Vec<_>, _>>()?;

        self.run_blocking(move |db| {
            let mut tx = db.begin().map_err(internal_err)?;
            let new_version = match append_into_tx(
                &mut tx,
                &scope,
                &aggregate_type,
                &aggregate_id,
                expected_version,
                &rows,
            ) {
                Ok(v) => v,
                // The version check itself already found the conflict (or a
                // genuine internal error) — abandon `tx`, nothing to commit.
                Err(e) => {
                    drop(tx);
                    return Err(e);
                }
            };
            match tx.commit() {
                Ok(()) => Ok(new_version),
                // A peer committed between our version check and our commit
                // (matching `PostgreSQLEventStore`'s re-read-on-commit-conflict
                // shape): re-read the now-current version through the fresh
                // `db` handle rather than the abandoned `tx`.
                Err(e) if is_write_conflict(&e) => {
                    let actual = read_max_version(db, &scope, &aggregate_type, &aggregate_id)?;
                    Err(conflict(
                        &aggregate_type,
                        &aggregate_id,
                        expected_version,
                        actual,
                    ))
                }
                Err(e) => Err(internal_err(e)),
            }
        })
        .await
    }

    async fn load(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        let resolved = resolve_tenant(tenant_id)?;
        let scope = encode_tenant(resolved.as_deref()).to_string();
        let aggregate_type_owned = aggregate_type.to_string();
        let aggregate_id_owned = aggregate_id.to_string();

        let rows: Vec<(String, String, DateTime<Utc>, Option<String>)> = self
            .run_blocking(move |db| {
                let rows = db
                    .query(
                        SELECT_EVENTS,
                        (
                            scope.as_str(),
                            aggregate_type_owned.as_str(),
                            aggregate_id_owned.as_str(),
                        ),
                    )
                    .map_err(internal_err)?;

                let mut out = Vec::new();
                for row in rows {
                    let row = row.map_err(internal_err)?;
                    let event_type: String = row.get(1).map_err(internal_err)?;
                    let payload: String = row.get(2).map_err(internal_err)?;
                    let occurred_at = match row.get_value(3) {
                        Some(stoolap::Value::Timestamp(dt)) => *dt,
                        other => {
                            return Err(PersistenceError::Internal(format!(
                                "occurred_at column did not hold a Timestamp value: {other:?}"
                            )))
                        }
                    };
                    let operation_key: Option<String> = row.get(4).map_err(internal_err)?;
                    out.push((event_type, payload, occurred_at, operation_key));
                }
                Ok(out)
            })
            .await?;

        if rows.is_empty() {
            return Err(PersistenceError::NotFound {
                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
            });
        }

        rows.into_iter()
            .map(|(event_type, payload, occurred_at, operation_key)| {
                let value: serde_json::Value =
                    serde_json::from_str(&payload).map_err(internal_err)?;
                let event = (self.deserialize)(&event_type, value, occurred_at)?;
                let stored = StoredEvent::new(event);
                match operation_key {
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
            .collect()
    }

    async fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        let resolved = resolve_tenant(tenant_id)?;
        let scope = encode_tenant(resolved.as_deref()).to_string();

        self.run_blocking(move |db| {
            let rows = db
                .query(SELECT_AGGREGATE_IDS, (scope.as_str(),))
                .map_err(internal_err)?;
            let mut out = Vec::new();
            for row in rows {
                let row = row.map_err(internal_err)?;
                let aggregate_type: String = row.get(0).map_err(internal_err)?;
                let aggregate_id: String = row.get(1).map_err(internal_err)?;
                out.push((aggregate_type, aggregate_id));
            }
            Ok(out)
        })
        .await
    }

    async fn find_receipt(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        operation_key: &str,
    ) -> Result<Option<OperationReceipt>, PersistenceError> {
        let resolved = resolve_tenant(tenant_id)?;
        let scope = encode_tenant(resolved.as_deref()).to_string();
        let aggregate_type = aggregate_type.to_string();
        let aggregate_id = aggregate_id.to_string();
        let operation_key = operation_key.to_string();

        let query_aggregate_type = aggregate_type.clone();
        let query_aggregate_id = aggregate_id.clone();
        let query_operation_key = operation_key.clone();
        let row: Option<(String, String, Option<i64>, Option<i64>)> = self
            .run_blocking(move |db| {
                let mut rows = db
                    .query(
                        SELECT_RECEIPT,
                        (
                            scope.as_str(),
                            query_aggregate_type.as_str(),
                            query_aggregate_id.as_str(),
                            query_operation_key.as_str(),
                        ),
                    )
                    .map_err(internal_err)?;
                match rows.next() {
                    None => Ok(None),
                    Some(row) => {
                        let row = row.map_err(internal_err)?;
                        let fingerprint: String = row.get(0).map_err(internal_err)?;
                        let outcome_kind: String = row.get(1).map_err(internal_err)?;
                        let version_from: Option<i64> = row.get(2).map_err(internal_err)?;
                        let version_to: Option<i64> = row.get(3).map_err(internal_err)?;
                        Ok(Some((fingerprint, outcome_kind, version_from, version_to)))
                    }
                }
            })
            .await?;

        let Some((fingerprint, outcome_kind, version_from, version_to)) = row else {
            return Ok(None);
        };

        let outcome = match (outcome_kind.as_str(), version_from, version_to) {
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
                     kind {outcome_kind:?}, range {version_from:?}..={version_to:?}"
                )))
            }
        };

        let tenant = match resolved {
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
        let db = self.db.clone();
        let tx = tokio::task::spawn_blocking(move || db.begin().map_err(internal_err))
            .await
            .map_err(|e| PersistenceError::Internal(format!("blocking task panicked: {e}")))??;
        Ok(Box::new(StoolapEventStoreUnitOfWork {
            tx: Some(tx),
            _marker: PhantomData,
        }))
    }
}

/// A unit of work backed by one real, live Stoolap transaction, moved in and
/// out of `spawn_blocking` on each call (design.md AD-2: `spawn_blocking`,
/// never `block_in_place`).
///
/// `stoolap::ApiTransaction` is `Send` but not `Sync` (confirmed by direct
/// experiment against stoolap 0.4.0's `Transaction` struct: it holds `Option<Box<dyn
/// Transaction>>`, whose `Transaction` trait supertrait is `Send`, not `Sync`)
/// — exactly what `spawn_blocking`'s closure bound needs, since only one
/// thread ever touches this transaction at a time and it is never shared
/// behind an `Arc`. `tx` is `Option` only so it can be moved out into a
/// closure and moved back in afterward; every method restores it before
/// returning, except `commit`, which consumes it.
pub struct StoolapEventStoreUnitOfWork<E> {
    tx: Option<stoolap::ApiTransaction>,
    _marker: PhantomData<E>,
}

#[async_trait]
impl<E> EventStoreUnitOfWork<E> for StoolapEventStoreUnitOfWork<E>
where
    E: DomainEvent,
{
    async fn append(
        &mut self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        let resolved = resolve_tenant(tenant_id)?;
        let scope = encode_tenant(resolved.as_deref()).to_string();
        let aggregate_type = aggregate_type.to_string();
        let aggregate_id = aggregate_id.to_string();
        let rows = events
            .iter()
            .map(to_event_row)
            .collect::<Result<Vec<_>, _>>()?;

        let mut tx = self.tx.take().ok_or_else(|| {
            PersistenceError::Internal("unit of work already committed".to_string())
        })?;

        let (tx, result) = tokio::task::spawn_blocking(move || {
            let result = append_into_tx(
                &mut tx,
                &scope,
                &aggregate_type,
                &aggregate_id,
                expected_version,
                &rows,
            );
            (tx, result)
        })
        .await
        .map_err(|e| PersistenceError::Internal(format!("blocking task panicked: {e}")))?;

        self.tx = Some(tx);
        result
    }

    async fn confirm_receipt(
        &mut self,
        receipt: &OperationReceipt,
    ) -> Result<(), PersistenceError> {
        let scope = encode_tenant(receipt.tenant().map(|t| t.as_str())).to_string();
        let aggregate_type = receipt.aggregate_type().to_string();
        let aggregate_id = receipt.aggregate_id().to_string();
        let operation_key = receipt.operation_key().as_str().to_string();
        let fingerprint = receipt.fingerprint().as_str().to_string();
        let (outcome_kind, version_from, version_to) = match receipt.outcome() {
            AggregateOutcome::NoEvents => ("no_events", None, None),
            AggregateOutcome::Events {
                version_from,
                version_to,
            } => ("events", Some(*version_from), Some(*version_to)),
        };

        let mut tx = self.tx.take().ok_or_else(|| {
            PersistenceError::Internal("unit of work already committed".to_string())
        })?;

        let (tx, result) = tokio::task::spawn_blocking(move || {
            let result = confirm_receipt_in_tx(
                &mut tx,
                &scope,
                &aggregate_type,
                &aggregate_id,
                &operation_key,
                &fingerprint,
                outcome_kind,
                version_from,
                version_to,
            );
            (tx, result)
        })
        .await
        .map_err(|e| PersistenceError::Internal(format!("blocking task panicked: {e}")))?;

        self.tx = Some(tx);
        result
    }

    async fn commit(mut self: Box<Self>) -> Result<(), PersistenceError> {
        let mut tx = self.tx.take().ok_or_else(|| {
            PersistenceError::Internal("unit of work already committed".to_string())
        })?;

        // AD-3 criterion 4: exactly one commit, no deferred/batched path.
        tokio::task::spawn_blocking(move || tx.commit().map_err(internal_err))
            .await
            .map_err(|e| PersistenceError::Internal(format!("blocking task panicked: {e}")))?
    }
}

/// Stages a receipt inside `tx`, never committing (the trait contract:
/// `EventStoreUnitOfWork::confirm_receipt` must not end the transaction).
///
/// A conflicting fingerprint at the same identity is refused, never
/// overwritten — see `EventStoreUnitOfWork::confirm_receipt`'s docs.
#[allow(clippy::too_many_arguments)]
fn confirm_receipt_in_tx(
    tx: &mut stoolap::ApiTransaction,
    scope: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    operation_key: &str,
    fingerprint: &str,
    outcome_kind: &str,
    version_from: Option<i64>,
    version_to: Option<i64>,
) -> Result<(), PersistenceError> {
    let existing: Option<String> = tx
        .query_opt::<String, _>(
            "SELECT fingerprint FROM operation_receipts \
             WHERE tenant_id = $1 AND aggregate_type = $2 AND aggregate_id = $3 AND operation_key = $4",
            (scope, aggregate_type, aggregate_id, operation_key),
        )
        .map_err(internal_err)?;

    if let Some(existing_fingerprint) = existing {
        return if existing_fingerprint == fingerprint {
            // The same request, staged or already committed earlier. Idempotent.
            Ok(())
        } else {
            Err(PersistenceError::Conflict {
                aggregate_id: format!("{aggregate_type}-{aggregate_id}"),
                expected: 0,
                actual: 0,
            })
        };
    }

    tx.execute(
        INSERT_RECEIPT,
        (
            scope,
            aggregate_type,
            aggregate_id,
            operation_key,
            fingerprint,
            outcome_kind,
            version_from,
            version_to,
        ),
    )
    .map_err(internal_err)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_persistence_api::operation::key::OperationFingerprint;
    use serde_json::json;
    use tempfile::TempDir;

    #[derive(Debug, Clone, PartialEq)]
    struct TestEvent {
        event_type: String,
        payload: serde_json::Value,
        occurred_at: DateTime<Utc>,
    }

    impl DomainEvent for TestEvent {
        fn aggregate_id(&self) -> &str {
            "unused"
        }
        fn event_type(&self) -> &str {
            &self.event_type
        }
        fn payload(&self) -> &serde_json::Value {
            &self.payload
        }
        fn occurred_at(&self) -> &DateTime<Utc> {
            &self.occurred_at
        }
    }

    fn test_event(event_type: &str, value: serde_json::Value) -> TestEvent {
        TestEvent {
            event_type: event_type.to_string(),
            payload: value,
            occurred_at: Utc::now(),
        }
    }

    fn deserialize_test_event(
        event_type: &str,
        payload: serde_json::Value,
        occurred_at: DateTime<Utc>,
    ) -> Result<TestEvent, PersistenceError> {
        Ok(TestEvent {
            event_type: event_type.to_string(),
            payload,
            occurred_at,
        })
    }

    type TestStore = StoolapEventStore<
        TestEvent,
        fn(&str, serde_json::Value, DateTime<Utc>) -> Result<TestEvent, PersistenceError>,
    >;

    async fn new_store(path: &Path) -> TestStore {
        let deserialize: fn(
            &str,
            serde_json::Value,
            DateTime<Utc>,
        ) -> Result<TestEvent, PersistenceError> = deserialize_test_event;
        StoolapEventStore::open(path, deserialize).await.unwrap()
    }

    /// Every test that touches the database serializes on stoolap's own
    /// failpoint lock, matching `repository.rs`/`snapshot.rs`'s guard.
    fn db_test_guard() -> stoolap::test_failpoints::FailpointGuard {
        stoolap::test_failpoints::FailpointGuard::new()
    }

    #[tokio::test]
    async fn an_opened_store_requested_full_sync() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;
        assert_eq!(store.dsn(), dsn_for(dir.path()));
        assert!(store.is_durable());
    }

    #[tokio::test]
    async fn appended_events_are_loaded_back_in_order() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;

        let events = vec![
            StoredEvent::new(test_event("Created", json!({"n": 1}))),
            StoredEvent::new(test_event("Renamed", json!({"n": 2}))),
        ];
        let version = store
            .append("widget", "agg-1", None, 0, events)
            .await
            .unwrap();
        assert_eq!(version, 2);

        let loaded = store.load("widget", "agg-1", None).await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].event.event_type, "Created");
        assert_eq!(loaded[1].event.event_type, "Renamed");
    }

    #[tokio::test]
    async fn loading_a_missing_aggregate_is_not_found() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;

        let err = store
            .load("widget", "never-appended", None)
            .await
            .unwrap_err();
        assert!(matches!(err, PersistenceError::NotFound { .. }));
    }

    #[tokio::test]
    async fn list_aggregate_ids_returns_every_stream_in_scope() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;

        store
            .append(
                "widget",
                "agg-1",
                None,
                0,
                vec![StoredEvent::new(test_event("Created", json!({})))],
            )
            .await
            .unwrap();
        store
            .append(
                "widget",
                "agg-2",
                None,
                0,
                vec![StoredEvent::new(test_event("Created", json!({})))],
            )
            .await
            .unwrap();

        let mut ids = store.list_aggregate_ids(None).await.unwrap();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                ("widget".to_string(), "agg-1".to_string()),
                ("widget".to_string(), "agg-2".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn a_stale_expected_version_is_a_conflict() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;

        store
            .append(
                "widget",
                "agg-3",
                None,
                0,
                vec![StoredEvent::new(test_event("Created", json!({})))],
            )
            .await
            .unwrap();

        let err = store
            .append(
                "widget",
                "agg-3",
                None,
                0,
                vec![StoredEvent::new(test_event("Renamed", json!({})))],
            )
            .await
            .unwrap_err();

        match err {
            PersistenceError::Conflict {
                expected, actual, ..
            } => {
                assert_eq!(expected, 0);
                assert_eq!(actual, 1);
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_tenants_events_are_isolated_from_another_tenant_sharing_the_same_aggregate_id() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;

        store
            .append(
                "widget",
                "shared-agg",
                Some("tenant-a"),
                0,
                vec![StoredEvent::new(test_event("OwnedByA", json!({})))],
            )
            .await
            .unwrap();
        store
            .append(
                "widget",
                "shared-agg",
                Some("tenant-b"),
                0,
                vec![StoredEvent::new(test_event("OwnedByB", json!({})))],
            )
            .await
            .unwrap();

        let a = store
            .load("widget", "shared-agg", Some("tenant-a"))
            .await
            .unwrap();
        let b = store
            .load("widget", "shared-agg", Some("tenant-b"))
            .await
            .unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].event.event_type, "OwnedByA");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].event.event_type, "OwnedByB");
    }

    #[tokio::test]
    async fn a_tenants_events_are_isolated_from_the_systemwide_scope() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;

        store
            .append(
                "widget",
                "shared-agg-2",
                Some("tenant-a"),
                0,
                vec![StoredEvent::new(test_event("OwnedByA", json!({})))],
            )
            .await
            .unwrap();

        let err = store
            .load("widget", "shared-agg-2", None)
            .await
            .unwrap_err();
        assert!(matches!(err, PersistenceError::NotFound { .. }));
    }

    #[tokio::test]
    async fn a_unit_of_work_can_append_across_two_calls_before_committing() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;

        let mut uow = store.begin().await.unwrap();
        let v1 = uow
            .append(
                "widget",
                "agg-4",
                None,
                0,
                vec![StoredEvent::new(test_event("Created", json!({})))],
            )
            .await
            .unwrap();
        assert_eq!(v1, 1);
        let v2 = uow
            .append(
                "widget",
                "agg-4",
                None,
                1,
                vec![StoredEvent::new(test_event("Renamed", json!({})))],
            )
            .await
            .unwrap();
        assert_eq!(v2, 2);

        // Nothing is visible to another handle before commit.
        assert!(store.load("widget", "agg-4", None).await.is_err());

        uow.commit().await.unwrap();

        let loaded = store.load("widget", "agg-4", None).await.unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[tokio::test]
    async fn a_dropped_unit_of_work_leaves_nothing_committed() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;

        {
            let mut uow = store.begin().await.unwrap();
            uow.append(
                "widget",
                "agg-5",
                None,
                0,
                vec![StoredEvent::new(test_event("Created", json!({})))],
            )
            .await
            .unwrap();
            // Dropped without commit.
        }

        let err = store.load("widget", "agg-5", None).await.unwrap_err();
        assert!(matches!(err, PersistenceError::NotFound { .. }));
    }

    #[tokio::test]
    async fn confirm_receipt_then_commit_makes_it_findable() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;

        let key = OperationKey::parse("op-1").unwrap();
        let receipt = OperationReceipt::new(
            "widget",
            "agg-6",
            None,
            key.clone(),
            OperationFingerprint::new("fp-1"),
            AggregateOutcome::events(1, 1).unwrap(),
        );

        let mut uow = store.begin().await.unwrap();
        uow.append(
            "widget",
            "agg-6",
            None,
            0,
            vec![StoredEvent::new(test_event("Created", json!({})))],
        )
        .await
        .unwrap();
        uow.confirm_receipt(&receipt).await.unwrap();

        // Not visible before commit — the read half lives on the store and
        // only ever sees committed rows.
        assert_eq!(
            store
                .find_receipt("widget", "agg-6", None, "op-1")
                .await
                .unwrap(),
            None
        );

        uow.commit().await.unwrap();

        let found = store
            .find_receipt("widget", "agg-6", None, "op-1")
            .await
            .unwrap();
        assert_eq!(found.unwrap().fingerprint().as_str(), "fp-1");
    }

    #[tokio::test]
    async fn confirm_receipt_with_a_different_fingerprint_at_the_same_identity_is_a_conflict() {
        let _fp = db_test_guard();
        let dir = TempDir::new().unwrap();
        let store = new_store(dir.path()).await;

        let key = OperationKey::parse("op-2").unwrap();
        let first = OperationReceipt::new(
            "widget",
            "agg-7",
            None,
            key.clone(),
            OperationFingerprint::new("fp-first"),
            AggregateOutcome::NoEvents,
        );
        let mut uow = store.begin().await.unwrap();
        uow.confirm_receipt(&first).await.unwrap();
        uow.commit().await.unwrap();

        let second = OperationReceipt::new(
            "widget",
            "agg-7",
            None,
            key,
            OperationFingerprint::new("fp-second"),
            AggregateOutcome::NoEvents,
        );
        let mut uow2 = store.begin().await.unwrap();
        let err = uow2.confirm_receipt(&second).await.unwrap_err();
        assert!(matches!(err, PersistenceError::Conflict { .. }));
    }
}
