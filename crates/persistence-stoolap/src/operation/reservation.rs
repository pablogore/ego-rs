//! Stoolap-backed [`OperationReservationStore`].
//!
//! Every transition is one conditional SQL statement whose `WHERE` clause
//! carries the full verification (design.md AD-4) — never a
//! `db.begin()`/`tx.commit()` unit of work. A separate read-then-write
//! transaction spanning a check and its dependent mutation would leave a
//! window in which the lease lapses or the row is taken over between the two
//! statements; the reference this store ports (`PostgresOperationReservationStore`,
//! `crates/persistence/src/postgres/reservation.rs`) states the same
//! rationale for its own CAS updates.
//!
//! # Blocking I/O
//!
//! Every `Database::execute`/`query` call is synchronous, disk-touching I/O.
//! All trait methods therefore run their body via
//! [`StoolapOperationReservationStore::run_blocking`], which hands the
//! closure to [`tokio::task::spawn_blocking`] — never `block_in_place`, which
//! panics outside a multi-threaded Tokio runtime and would break this
//! module's own `#[tokio::test]`s (current-thread flavor).
//!
//! # Time comes from the injected `Clock`, never `now()`
//!
//! Every expiry decision reads `clock.now()`, matching the Postgres
//! reference implementation (AD-8) and required by the shared conformance
//! harness's `TestClock`.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use stoolap::{Database, Value};

use ego_domain::operation::{
    FencingToken, Lease, OldestCompleted, OperationId, OperationReservationStore, OwnerFence,
    ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse,
};
use ego_domain::Clock;

use crate::persistence::stoolap_common::{dsn_declares_sync_full, dsn_for, encode_tenant};

const BASE64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

// Deliberately no `STATE_IN_PROGRESS`/`STATE_COMPLETED` constant for the
// in-progress literal: the queries below spell it as a SQL literal because a
// bound parameter cannot stand in for one, matching
// `PostgresOperationReservationStore`'s own choice.
const STATE_COMPLETED: &str = "completed";

const CREATE_RESERVATIONS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS operation_reservations (
    tenant_id     TEXT      NOT NULL,
    operation_key TEXT      NOT NULL,
    fingerprint   TEXT      NOT NULL,
    owner_id      TEXT      NOT NULL,
    fencing_token INTEGER   NOT NULL,
    lease_until   TIMESTAMP NOT NULL,
    state         TEXT      NOT NULL,
    completed_at  TIMESTAMP,
    response      TEXT,
    UNIQUE (tenant_id, operation_key)
)";

/// Maps a raw Stoolap error to the port's opaque backend variant.
fn backend_err(e: impl std::fmt::Display) -> ReservationError {
    ReservationError::Backend(e.to_string())
}

/// Converts a token into the column's type, refusing rather than wrapping —
/// mirrors `PostgresOperationReservationStore::token_for_storage` (AD-10; not
/// shared cross-crate because that function is `pub(crate)` in
/// `ego-persistence`).
fn token_for_storage(token: FencingToken) -> Result<i64, ReservationError> {
    i64::try_from(token.value()).map_err(|_| ReservationError::FencingExhausted)
}

/// Rebuilds a token from the column, refusing a value no writer of ours could
/// produce — mirrors `PostgresOperationReservationStore::token_from_storage`
/// (AD-10).
fn token_from_storage(raw: i64) -> Result<FencingToken, ReservationError> {
    if raw <= 0 {
        return Err(ReservationError::Backend(format!(
            "stored fencing_token {raw} is not positive; the sequence starts at 1"
        )));
    }
    let value = u64::try_from(raw).map_err(|_| {
        ReservationError::Backend(format!("stored fencing_token {raw} is not representable"))
    })?;
    Ok(FencingToken::from_value(value))
}

/// Clamped rather than refused: `batch` is an upper bound, so removing fewer
/// rows than asked never violates "at most `batch`" (mirrors
/// `PostgresOperationReservationStore::batch_for_storage`).
fn batch_for_storage(batch: usize) -> i64 {
    i64::try_from(batch).unwrap_or(i64::MAX)
}

/// A reservation row, as the queries below read it back.
struct ReservationRow {
    fingerprint: String,
    owner_id: String,
    fencing_token: i64,
    lease_until: DateTime<Utc>,
    state: String,
    response: Option<String>,
}

/// Reads a `TIMESTAMP` column back. No `FromValue` impl exists for
/// `DateTime<Utc>` in Stoolap 0.4.0 (only i64/i32/f64/String/bool/Value and
/// `Option<T: FromValue>`), so every timestamp column is read via
/// `row.get_value` and matched manually — the same pattern
/// `StoolapEventStore` and `StoolapEffectStore::claim_due` already use.
fn read_timestamp(
    row: &stoolap::ResultRow,
    idx: usize,
    column: &str,
) -> Result<DateTime<Utc>, ReservationError> {
    match row.get_value(idx) {
        Some(Value::Timestamp(dt)) => Ok(*dt),
        other => Err(backend_err(format!(
            "{column} column did not hold a Timestamp value: {other:?}"
        ))),
    }
}

/// A durable, single-process [`OperationReservationStore`] backed by an
/// embedded Stoolap database.
pub struct StoolapOperationReservationStore {
    db: Database,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for StoolapOperationReservationStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoolapOperationReservationStore")
            .field("dsn", &self.db.dsn())
            .finish()
    }
}

impl StoolapOperationReservationStore {
    /// Opens (creating the `operation_reservations` table if absent) a
    /// Stoolap-backed reservation store at `path`, reading time from `clock`.
    ///
    /// Fails closed (design.md AD-3/AD-9, exactly like `StoolapSnapshotStore::open`
    /// and `StoolapEventStore::open`): only ever returns a store whose live
    /// engine reports `sync=full`, so `is_durable()` never outlives-lies
    /// about how the store was opened.
    pub async fn open(path: &Path, clock: Arc<dyn Clock>) -> Result<Self, ReservationError> {
        let dsn = dsn_for(path);
        let db = Database::open(&dsn).map_err(backend_err)?;

        if !dsn_declares_sync_full(db.dsn()) {
            return Err(ReservationError::Backend(format!(
                "stoolap engine at {:?} is not configured for durable sync (sync=full); \
                 refusing to open an OperationReservationStore that would misreport is_durable()",
                db.dsn()
            )));
        }

        // Dialect note (AD-9/AD-4): a composite `PRIMARY KEY` is parsed but
        // silently NOT enforced by Stoolap 0.4.0 (no constraint, no index) —
        // `UNIQUE` is fully enforced and is what `ON CONFLICT` matches
        // against, so it is used here instead.
        db.execute(CREATE_RESERVATIONS_TABLE, ())
            .map_err(backend_err)?;

        Ok(Self { db, clock })
    }

    /// Runs `f` against a cloned `Database` handle on Tokio's blocking-thread
    /// pool — the same shape as `StoolapEffectStore::run_blocking`.
    async fn run_blocking<F, R>(&self, f: F) -> Result<R, ReservationError>
    where
        F: FnOnce(&Database) -> Result<R, ReservationError> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || f(&db))
            .await
            .map_err(|e| ReservationError::Backend(format!("blocking task panicked: {e}")))?
    }

    /// Reads the current row for an identity, if any.
    fn current(
        db: &Database,
        tenant: &str,
        key: &str,
    ) -> Result<Option<ReservationRow>, ReservationError> {
        let mut rows = db
            .query(
                "SELECT fingerprint, owner_id, fencing_token, lease_until, state, response
                 FROM operation_reservations
                 WHERE tenant_id = $1 AND operation_key = $2",
                (tenant.to_string(), key.to_string()),
            )
            .map_err(backend_err)?;
        let row = match rows.next() {
            Some(row) => row.map_err(backend_err)?,
            None => return Ok(None),
        };

        let fingerprint: String = row.get(0).map_err(backend_err)?;
        let owner_id: String = row.get(1).map_err(backend_err)?;
        let fencing_token: i64 = row.get(2).map_err(backend_err)?;
        let lease_until = read_timestamp(&row, 3, "lease_until")?;
        let state: String = row.get(4).map_err(backend_err)?;
        let response: Option<String> = row.get(5).map_err(backend_err)?;

        Ok(Some(ReservationRow {
            fingerprint,
            owner_id,
            fencing_token,
            lease_until,
            state,
            response,
        }))
    }

    #[cfg(test)]
    fn dsn(&self) -> &str {
        self.db.dsn()
    }
}

#[async_trait]
impl OperationReservationStore for StoolapOperationReservationStore {
    /// Truthful by construction (design.md AD-3/AD-9 criterion 3): `open()`
    /// only ever returns a store whose live engine reports `sync=full`, so
    /// this re-derives from the same invariant rather than a hardcoded value.
    fn is_durable(&self) -> bool {
        dsn_declares_sync_full(self.db.dsn())
    }

    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        let tenant = encode_tenant(req.tenant.as_ref().map(|t| t.as_str())).to_string();
        let key = req.operation_key.as_str().to_string();
        let operation_id = OperationId::new(req.tenant.clone(), req.operation_key.clone());
        let clock = self.clock.clone();

        self.run_blocking(move |db| {
            // A first attempt inserts. `ON CONFLICT DO NOTHING` makes two
            // racing first attempts resolve without either seeing a unique
            // violation: exactly one inserts, the other falls through to the
            // observation below and sees the winner's row.
            let inserted = db
                .execute(
                    "INSERT INTO operation_reservations
                        (tenant_id, operation_key, fingerprint, owner_id, fencing_token,
                         lease_until, state)
                     VALUES ($1, $2, $3, $4, $5, $6, 'in_progress')
                     ON CONFLICT (tenant_id, operation_key) DO NOTHING",
                    (
                        tenant.clone(),
                        key.clone(),
                        req.fingerprint.as_str().to_string(),
                        req.owner_id.as_str().to_string(),
                        token_for_storage(FencingToken::initial())?,
                        req.lease_until,
                    ),
                )
                .map_err(backend_err)?;

            if inserted == 1 {
                return Ok(ReservationOutcome::Fresh(Lease {
                    operation_id,
                    owner_id: req.owner_id,
                    fencing_token: FencingToken::initial(),
                    lease_until: req.lease_until,
                }));
            }

            let existing =
                match Self::current(db, &tenant, &key)? {
                    Some(row) => row,
                    None => return Err(ReservationError::Backend(
                        "the reservation disappeared between the insert conflict and the read; \
                         retry the reserve"
                            .to_string(),
                    )),
                };

            // Fingerprint first, before any ownership or lease consideration:
            // a different fingerprint under the same key is a permanent
            // conflict whatever the lease says.
            if existing.fingerprint != req.fingerprint.as_str() {
                return Ok(ReservationOutcome::Conflict);
            }

            if existing.state == STATE_COMPLETED {
                let response = existing.response.ok_or_else(|| {
                    ReservationError::Backend(
                        "a completed reservation has no stored response".to_string(),
                    )
                })?;
                let bytes = BASE64
                    .decode(response.as_bytes())
                    .map_err(|e| backend_err(format!("response decode: {e}")))?;
                return Ok(ReservationOutcome::Succeeded(StoredServiceResponse::new(
                    bytes,
                )));
            }

            let now = clock.now();
            if now >= existing.lease_until {
                // Expired: take it over with a strictly greater token. The
                // `lease_until <= $now` predicate — re-checked here, not
                // trusted from the read above — is what makes this safe: the
                // read and this write are separate statements, so between
                // them another caller can take the reservation over or its
                // owner can renew it (AD-4).
                let displaced = token_from_storage(existing.fencing_token)?;
                let next = displaced.next().ok_or(ReservationError::FencingExhausted)?;

                let took_over = db
                    .execute(
                        "UPDATE operation_reservations
                         SET owner_id = $1, fencing_token = $2, lease_until = $3
                         WHERE tenant_id = $4 AND operation_key = $5
                           AND state = 'in_progress'
                           AND fencing_token = $6
                           AND lease_until <= $7",
                        (
                            req.owner_id.as_str().to_string(),
                            token_for_storage(next)?,
                            req.lease_until,
                            tenant.clone(),
                            key.clone(),
                            existing.fencing_token,
                            now,
                        ),
                    )
                    .map_err(backend_err)?;

                if took_over == 1 {
                    return Ok(ReservationOutcome::TakenOver(Lease {
                        operation_id,
                        owner_id: req.owner_id,
                        fencing_token: next,
                        lease_until: req.lease_until,
                    }));
                }

                // Someone else took it over (or renewed) first — re-read
                // rather than assume, so a recovering owner is reported as
                // OwnedInProgress rather than OtherInProgress.
                let after = Self::current(db, &tenant, &key)?.ok_or_else(|| {
                    ReservationError::Backend(
                        "the reservation disappeared during a takeover race; retry the reserve"
                            .to_string(),
                    )
                })?;
                if after.owner_id == req.owner_id.as_str() {
                    return Ok(ReservationOutcome::OwnedInProgress(Lease {
                        operation_id,
                        owner_id: req.owner_id,
                        fencing_token: token_from_storage(after.fencing_token)?,
                        lease_until: after.lease_until,
                    }));
                }
                return Ok(ReservationOutcome::OtherInProgress);
            }

            if existing.owner_id == req.owner_id.as_str() {
                return Ok(ReservationOutcome::OwnedInProgress(Lease {
                    operation_id,
                    owner_id: req.owner_id,
                    fencing_token: token_from_storage(existing.fencing_token)?,
                    lease_until: existing.lease_until,
                }));
            }

            Ok(ReservationOutcome::OtherInProgress)
        })
        .await
    }

    // ponytail: Postgres's reference factors renew/complete/abandon through a
    // shared `mutate_owned<F>` combinator (same WHERE clause, different SET
    // fragment). Each mutator here inlines its own CAS statement instead —
    // three call sites with a one-line WHERE clause each don't earn a generic
    // combinator; promote to `mutate_owned` if a fourth mutator with the same
    // ownership predicate shows up.
    async fn renew(
        &self,
        fence: &OwnerFence,
        until: DateTime<Utc>,
    ) -> Result<(), ReservationError> {
        let tenant = encode_tenant(fence.operation_id.tenant().map(|t| t.as_str())).to_string();
        let key = fence.operation_id.operation_key().as_str().to_string();
        let owner = fence.owner_id.as_str().to_string();
        let token = token_for_storage(fence.fencing_token)?;
        let clock = self.clock.clone();

        self.run_blocking(move |db| {
            let now = clock.now();
            let affected = db
                .execute(
                    "UPDATE operation_reservations
                     SET lease_until = $1
                     WHERE tenant_id = $2 AND operation_key = $3
                       AND owner_id = $4 AND fencing_token = $5
                       AND state = 'in_progress' AND lease_until > $6",
                    (until, tenant, key, owner, token, now),
                )
                .map_err(backend_err)?;

            if affected == 0 {
                return Err(ReservationError::StaleOwner);
            }
            Ok(())
        })
        .await
    }

    async fn complete(
        &self,
        fence: &OwnerFence,
        response: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        let tenant = encode_tenant(fence.operation_id.tenant().map(|t| t.as_str())).to_string();
        let key = fence.operation_id.operation_key().as_str().to_string();
        let owner = fence.owner_id.as_str().to_string();
        let token = token_for_storage(fence.fencing_token)?;
        let encoded = BASE64.encode(response.as_bytes());
        let clock = self.clock.clone();

        self.run_blocking(move |db| {
            let now = clock.now();
            let affected = db
                .execute(
                    "UPDATE operation_reservations
                     SET state = 'completed', completed_at = $1, response = $2
                     WHERE tenant_id = $3 AND operation_key = $4
                       AND owner_id = $5 AND fencing_token = $6
                       AND state = 'in_progress' AND lease_until > $7",
                    (now, encoded, tenant, key, owner, token, now),
                )
                .map_err(backend_err)?;

            if affected == 0 {
                return Err(ReservationError::StaleOwner);
            }
            Ok(())
        })
        .await
    }

    async fn abandon(&self, fence: &OwnerFence) -> Result<(), ReservationError> {
        let tenant = encode_tenant(fence.operation_id.tenant().map(|t| t.as_str())).to_string();
        let key = fence.operation_id.operation_key().as_str().to_string();
        let owner = fence.owner_id.as_str().to_string();
        let token = token_for_storage(fence.fencing_token)?;
        let clock = self.clock.clone();

        self.run_blocking(move |db| {
            let now = clock.now();
            let affected = db
                .execute(
                    "DELETE FROM operation_reservations
                     WHERE tenant_id = $1 AND operation_key = $2
                       AND owner_id = $3 AND fencing_token = $4
                       AND state = 'in_progress' AND lease_until > $5",
                    (tenant, key, owner, token, now),
                )
                .map_err(backend_err)?;

            if affected == 0 {
                return Err(ReservationError::StaleOwner);
            }
            Ok(())
        })
        .await
    }

    async fn purge_completed_before(
        &self,
        cutoff: DateTime<Utc>,
        batch: usize,
    ) -> Result<u64, ReservationError> {
        let batch = batch_for_storage(batch);

        self.run_blocking(move |db| {
            // Dialect note (AD-9/AD-4): `DELETE ... WHERE col IN (SELECT ...
            // LIMIT n)` silently deletes ZERO rows against Stoolap 0.4.0.
            // Purge is therefore two steps: select the bounded batch of
            // eligible identities, then delete each by its own equality
            // predicate that RE-ASSERTS eligibility (state='completed' AND
            // completed_at < cutoff) rather than matching on the key alone —
            // a reservation key is re-creatable after `abandon`, so a bare
            // key match could delete a *new*, unrelated reservation that
            // happens to reuse the key.
            let candidates: Vec<(String, String)> = {
                let rows = db
                    .query(
                        "SELECT tenant_id, operation_key FROM operation_reservations
                         WHERE state = 'completed' AND completed_at < $1
                         LIMIT $2",
                        (cutoff, batch),
                    )
                    .map_err(backend_err)?;
                let mut out = Vec::new();
                for row in rows {
                    let row = row.map_err(backend_err)?;
                    let tenant: String = row.get(0).map_err(backend_err)?;
                    let key: String = row.get(1).map_err(backend_err)?;
                    out.push((tenant, key));
                }
                out
            };

            let mut deleted = 0u64;
            for (tenant, key) in candidates {
                let affected = db
                    .execute(
                        "DELETE FROM operation_reservations
                         WHERE tenant_id = $1 AND operation_key = $2
                           AND state = 'completed' AND completed_at < $3",
                        (tenant, key, cutoff),
                    )
                    .map_err(backend_err)?;
                deleted += affected.max(0) as u64;
            }
            Ok(deleted)
        })
        .await
    }

    /// `MIN(completed_at)` over the completed rows that remain — supported
    /// by Stoolap 0.4.0's aggregate executor. A bare aggregate with no
    /// `GROUP BY` always returns exactly one row even over an empty table
    /// (its value is SQL `NULL`), so `query_one` never sees "zero rows" here
    /// — the `NULL` case is read explicitly and mapped to
    /// [`OldestCompleted::Empty`], a real answer distinct from `Unsupported`.
    async fn oldest_completed(&self) -> Result<OldestCompleted, ReservationError> {
        self.run_blocking(move |db| {
            let mut rows = db
                .query(
                    "SELECT MIN(completed_at) FROM operation_reservations WHERE state = 'completed'",
                    (),
                )
                .map_err(backend_err)?;
            let row = rows
                .next()
                .ok_or_else(|| backend_err("MIN(completed_at) returned no row"))?
                .map_err(backend_err)?;

            match row.get_value(0) {
                Some(Value::Timestamp(dt)) => Ok(OldestCompleted::At(*dt)),
                Some(Value::Null(_)) | None => Ok(OldestCompleted::Empty),
                other => Err(backend_err(format!(
                    "MIN(completed_at) did not hold a Timestamp or NULL value: {other:?}"
                ))),
            }
        })
        .await
    }

    async fn probe(&self) -> Result<(), ReservationError> {
        // Against the reservation table, not a bare `SELECT 1`: proves the
        // schema this store writes to actually exists, not merely that the
        // engine is reachable.
        self.run_blocking(move |db| {
            db.query("SELECT 1 FROM operation_reservations LIMIT 1", ())
                .map_err(backend_err)?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use ego_domain::operation::{OperationFingerprint, OperationKey, OwnerId};
    use ego_testkit::TestClock;

    /// Every test that touches the database serializes on stoolap's own
    /// failpoint lock, matching `snapshot.rs`'s and `repository.rs`'s guard:
    /// `WAL_WRITE_FAIL` is a process-wide `AtomicBool`, so an unguarded test
    /// here can observe another module's failpoint test armed mid-run and
    /// see its own unrelated `open()`/`reserve()` fail with a spurious
    /// "failpoint: WAL write" error.
    fn db_test_guard() -> stoolap::test_failpoints::FailpointGuard {
        stoolap::test_failpoints::FailpointGuard::new()
    }

    fn epoch() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
    }

    fn request(owner: &str, key: &str, lease_until: DateTime<Utc>) -> ReserveRequest {
        ReserveRequest {
            tenant: None,
            operation_key: OperationKey::parse(key).unwrap(),
            fingerprint: OperationFingerprint::new("fp-1"),
            owner_id: OwnerId::new(owner),
            lease_until,
        }
    }

    fn fence_of(lease: &Lease) -> OwnerFence {
        OwnerFence {
            operation_id: lease.operation_id.clone(),
            owner_id: lease.owner_id.clone(),
            fencing_token: lease.fencing_token,
        }
    }

    async fn fresh_store() -> (
        StoolapOperationReservationStore,
        Arc<TestClock>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let clock = Arc::new(TestClock::new(epoch()));
        let store = StoolapOperationReservationStore::open(dir.path(), clock.clone())
            .await
            .expect("open StoolapOperationReservationStore");
        (store, clock, dir)
    }

    #[tokio::test]
    async fn an_opened_store_requested_full_sync() {
        let _fp = db_test_guard();
        let (store, _clock, dir) = fresh_store().await;
        assert_eq!(store.dsn(), dsn_for(dir.path()));
        assert!(store.is_durable());
    }

    /// AD-9 regression guard, mirroring `StoolapSnapshotStore`'s and
    /// `StoolapEventStore`'s identical test: a live, weakly-configured
    /// engine already holds `path` when `open()` is called must be refused,
    /// not silently accepted.
    #[tokio::test]
    async fn open_refuses_a_path_already_locked_by_a_non_durable_engine() {
        let _fp = db_test_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        let weak_dsn = format!("file://{}", dir.path().display());
        let _weak_db = Database::open(&weak_dsn).unwrap();

        let err =
            StoolapOperationReservationStore::open(dir.path(), Arc::new(TestClock::new(epoch())))
                .await
                .expect_err("expected open() to refuse a path locked by a non-durable engine");
        assert!(matches!(err, ReservationError::Backend(_)));
    }

    #[tokio::test]
    async fn a_key_nobody_holds_is_a_fresh_reservation() {
        let _fp = db_test_guard();
        let (store, _clock, _dir) = fresh_store().await;
        let outcome = store
            .reserve(request("owner-a", "op-1", epoch() + Duration::seconds(30)))
            .await
            .unwrap();
        assert!(matches!(outcome, ReservationOutcome::Fresh(_)));
    }

    #[tokio::test]
    async fn the_same_owner_mid_lease_is_owned_in_progress() {
        let _fp = db_test_guard();
        let (store, _clock, _dir) = fresh_store().await;
        let lease_until = epoch() + Duration::seconds(30);
        store
            .reserve(request("owner-a", "op-2", lease_until))
            .await
            .unwrap();
        let outcome = store
            .reserve(request("owner-a", "op-2", lease_until))
            .await
            .unwrap();
        assert!(matches!(outcome, ReservationOutcome::OwnedInProgress(_)));
    }

    #[tokio::test]
    async fn a_different_owner_mid_lease_is_other_in_progress() {
        let _fp = db_test_guard();
        let (store, _clock, _dir) = fresh_store().await;
        let lease_until = epoch() + Duration::seconds(30);
        store
            .reserve(request("owner-a", "op-3", lease_until))
            .await
            .unwrap();
        let outcome = store
            .reserve(request("owner-b", "op-3", lease_until))
            .await
            .unwrap();
        assert_eq!(outcome, ReservationOutcome::OtherInProgress);
    }

    #[tokio::test]
    async fn a_different_fingerprint_under_the_same_key_is_a_conflict() {
        let _fp = db_test_guard();
        let (store, _clock, _dir) = fresh_store().await;
        let lease_until = epoch() + Duration::seconds(30);
        store
            .reserve(request("owner-a", "op-4", lease_until))
            .await
            .unwrap();
        let mut other = request("owner-b", "op-4", lease_until);
        other.fingerprint = OperationFingerprint::new("fp-different");
        let outcome = store.reserve(other).await.unwrap();
        assert_eq!(outcome, ReservationOutcome::Conflict);
    }

    #[tokio::test]
    async fn an_expired_lease_is_taken_over_with_a_strictly_greater_token() {
        let _fp = db_test_guard();
        let (store, clock, _dir) = fresh_store().await;
        let fresh = store
            .reserve(request(
                "owner-a",
                "op-takeover",
                epoch() + Duration::seconds(30),
            ))
            .await
            .unwrap();
        let original_fence = match fresh {
            ReservationOutcome::Fresh(lease) => fence_of(&lease),
            other => panic!("expected Fresh, got {other:?}"),
        };

        clock.advance(Duration::seconds(31));
        let outcome = store
            .reserve(request(
                "owner-b",
                "op-takeover",
                clock.now() + Duration::seconds(30),
            ))
            .await
            .unwrap();
        match outcome {
            ReservationOutcome::TakenOver(lease) => {
                assert!(lease.fencing_token > original_fence.fencing_token);
            }
            other => panic!("expected TakenOver, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_stale_fence_is_rejected_by_every_mutator_and_leaves_state_unmodified() {
        let _fp = db_test_guard();
        let (store, clock, _dir) = fresh_store().await;
        let fresh = store
            .reserve(request("owner-a", "op-5", epoch() + Duration::seconds(30)))
            .await
            .unwrap();
        let stale_fence = match fresh {
            ReservationOutcome::Fresh(lease) => fence_of(&lease),
            other => panic!("expected Fresh, got {other:?}"),
        };

        // The lease lapses and a second owner takes it over — this is what
        // makes `stale_fence` genuinely stale, not merely unused.
        clock.advance(Duration::seconds(31));
        let taken_over = store
            .reserve(request(
                "owner-b",
                "op-5",
                clock.now() + Duration::seconds(30),
            ))
            .await
            .unwrap();
        let current_fence = match taken_over {
            ReservationOutcome::TakenOver(lease) => fence_of(&lease),
            other => panic!("expected TakenOver, got {other:?}"),
        };

        // Every mutator rejects the displaced owner's fence...
        assert_eq!(
            store
                .renew(&stale_fence, clock.now() + Duration::seconds(60))
                .await,
            Err(ReservationError::StaleOwner)
        );
        assert_eq!(
            store
                .complete(&stale_fence, StoredServiceResponse::new(b"stale".to_vec()))
                .await,
            Err(ReservationError::StaleOwner)
        );
        assert_eq!(
            store.abandon(&stale_fence).await,
            Err(ReservationError::StaleOwner)
        );

        // ...and none of those rejected attempts modified the reservation:
        // the current owner's fence is still valid.
        store
            .complete(&current_fence, StoredServiceResponse::new(b"real".to_vec()))
            .await
            .expect("the current owner's fence must still be valid after every stale attempt");
    }

    #[tokio::test]
    async fn purge_removes_only_rows_completed_strictly_before_the_cutoff() {
        let _fp = db_test_guard();
        let (store, _clock, _dir) = fresh_store().await;
        let cutoff = epoch() + Duration::seconds(100);

        let outcome = store
            .reserve(request(
                "owner-a",
                "op-old",
                epoch() + Duration::seconds(300),
            ))
            .await
            .unwrap();
        let fence = match outcome {
            ReservationOutcome::Fresh(lease) => fence_of(&lease),
            other => panic!("expected Fresh, got {other:?}"),
        };
        store
            .complete(&fence, StoredServiceResponse::new(b"old".to_vec()))
            .await
            .unwrap();

        let purged = store.purge_completed_before(cutoff, 10).await.unwrap();
        assert_eq!(purged, 1);

        let after = store
            .reserve(request("probe", "op-old", cutoff + Duration::seconds(300)))
            .await
            .unwrap();
        assert!(matches!(after, ReservationOutcome::Fresh(_)));
    }

    #[tokio::test]
    async fn an_in_progress_reservation_is_never_purged() {
        let _fp = db_test_guard();
        let (store, _clock, _dir) = fresh_store().await;
        store
            .reserve(request(
                "owner-a",
                "op-live",
                epoch() + Duration::seconds(30),
            ))
            .await
            .unwrap();

        let purged = store
            .purge_completed_before(epoch() + Duration::seconds(10_000), 10)
            .await
            .unwrap();
        assert_eq!(purged, 0);
    }

    #[tokio::test]
    async fn oldest_completed_is_empty_when_the_backlog_is_clear() {
        let _fp = db_test_guard();
        let (store, _clock, _dir) = fresh_store().await;
        assert_eq!(
            store.oldest_completed().await.unwrap(),
            OldestCompleted::Empty
        );
    }

    #[tokio::test]
    async fn probe_succeeds_against_an_empty_table() {
        let _fp = db_test_guard();
        let (store, _clock, _dir) = fresh_store().await;
        store.probe().await.unwrap();
    }
}
