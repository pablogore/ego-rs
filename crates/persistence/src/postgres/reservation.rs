//! The durable [`OperationReservationStore`].
//!
//! # Where "now" comes from
//!
//! Every expiry decision reads the injected [`Clock`], never the database's
//! `now()`. That is the design's choice (AD-8), and it has two consequences worth
//! stating rather than rediscovering.
//!
//! It makes takeover deterministic under a test clock: a scenario positions the
//! clock and the store agrees, with no sleeping and no dependence on how fast the
//! machine runs.
//!
//! And it means two nodes with skewed clocks can disagree about whether a lease has
//! expired. What that costs, stated precisely rather than reassuringly:
//!
//! - A premature takeover becomes possible, and with it **concurrent work**: the
//!   displaced owner may still be executing when the new one starts.
//! - The fencing token guarantees the displaced owner cannot **mutate or finalise
//!   this reservation** — its `renew`, `complete` and `abandon` all fail with
//!   [`ReservationError::StaleOwner`], so the completion that is eventually recorded
//!   and replayed is the current owner's.
//! - It guarantees **nothing about an external effect already in flight**. A token
//!   is a predicate on a row; it neither cancels a request that has left the process
//!   nor prevents a second one. Avoiding a duplicated external effect requires the
//!   effect boundary itself to carry the fence or to be idempotent on its own —
//!   which is B6's subject, not this store's.
//!
//! So expiry decides when an attempt is *permitted*, and fencing decides whose
//! reservation outcome is *authoritative*. Neither makes two concurrent executions
//! impossible, and an earlier version of this note claimed skew could never cause a
//! double execution. It can, until the effect boundary is wired.
//!
//! Tightening the clocks is therefore worth doing for the wasted work it avoids, and
//! is not a substitute for that wiring.
//!
//! # Selection within a purge batch is not a promise
//!
//! `purge_completed_before` guarantees eligibility, the batch limit, the returned
//! count, and that an in-progress reservation is never removed. It does not
//! guarantee which eligible rows a call chooses (AD-11). This implementation is
//! single-worker: correct against the contract, and deliberately not yet hardened
//! for concurrent workers, which is B7's subject.
//!
//! # Every value is bound, never interpolated
//!
//! All SQL here uses `$N` placeholders. An operation key is client-supplied, so
//! interpolating one would put caller-controlled text into a statement.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use ego_domain::context::TenantId;
use ego_domain::operation::{
    FencingToken, Lease, OperationId, OperationReservationStore, OwnerFence, ReservationError,
    ReservationOutcome, ReserveRequest, StoredServiceResponse,
};
use ego_domain::Clock;
use std::sync::Arc;

/// The state a completed reservation carries, for the one place Rust compares it.
///
/// There is no matching constant for the in-progress state: the queries below spell
/// their states as SQL literals, because a bound parameter cannot stand in for one,
/// and a constant nothing reads would be a name pretending to be a single source of
/// truth.
const STATE_COMPLETED: &str = "completed";

/// A durable reservation store backed by one PostgreSQL table.
pub struct PostgresOperationReservationStore {
    pool: PgPool,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for PostgresOperationReservationStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresOperationReservationStore")
            .field("pool", &self.pool)
            .finish_non_exhaustive()
    }
}

impl PostgresOperationReservationStore {
    /// Builds a store over `pool`, reading time from `clock`.
    pub fn new(pool: PgPool, clock: Arc<dyn Clock>) -> Self {
        Self { pool, clock }
    }
}

/// Maps a storage failure into the port's opaque backend variant.
fn storage(err: sqlx::Error) -> ReservationError {
    ReservationError::Backend(err.to_string())
}

/// Converts a token into the column's type, refusing rather than wrapping.
///
/// The domain counts tokens in `u64`; this column is `BIGINT`, which is `i64`. The
/// two ranges differ, and the difference is not academic: at `i64::MAX` the domain's
/// `next()` still succeeds — `u64` has room — and an unchecked cast would land on
/// `i64::MIN`, a value PostgreSQL accepts and which is *less* than the token it
/// displaced. The type's whole promise is that a new token is strictly greater than
/// the one it replaces, so wrapping there would silently retire the guarantee at
/// exactly the boundary the domain thought it had covered.
///
/// The exhaustion the port names is therefore the *storage* limit, not `u64`'s. That
/// is the honest reading: a token that cannot be stored cannot fence anything.
fn token_for_storage(token: FencingToken) -> Result<i64, ReservationError> {
    i64::try_from(token.value()).map_err(|_| ReservationError::FencingExhausted)
}

/// Rebuilds a token from the column, refusing a value no writer of ours could
/// produce.
///
/// The sequence starts at one, so a stored token is always positive and anything else
/// means the row was written by something that did not go through this adapter. The
/// table's own CHECK forbids it, and this is the second line rather than the only
/// one: a store that trusted the constraint would be trusting a schema it does not
/// re-verify on every deployment.
///
/// The predicate is `raw <= 0`, not a `u64` conversion. `u64::try_from` rejects
/// negatives and **accepts zero**, which is exactly the value this function's own
/// promise excludes — a `FencingToken(0)` is a token nobody minted, and the earlier
/// version of this code built one while its documentation said it would not.
fn token_from_storage(raw: i64) -> Result<FencingToken, ReservationError> {
    if raw <= 0 {
        return Err(ReservationError::Backend(format!(
            "stored fencing_token {raw} is not positive; the sequence starts at 1 and the \
             table's own CHECK forbids anything else"
        )));
    }
    // Positive by the check above, so the conversion cannot fail — but expressed as a
    // conversion rather than a cast, so a later change to the guard cannot silently
    // reintroduce a wrap.
    let value = u64::try_from(raw).map_err(|_| {
        ReservationError::Backend(format!("stored fencing_token {raw} is not representable"))
    })?;
    Ok(FencingToken::from_value(value))
}

/// Converts a batch limit into the column's type.
///
/// Clamped rather than refused, and safe to clamp because `batch` is an *upper*
/// bound: removing fewer rows than asked never violates "at most `batch`". A caller
/// requesting more rows than `i64::MAX` is asking for more than can exist, so the
/// clamp changes nothing observable — unlike an unchecked cast, which would wrap to a
/// negative limit and delete nothing at all.
fn batch_for_storage(batch: usize) -> i64 {
    i64::try_from(batch).unwrap_or(i64::MAX)
}

/// The `tenant_id` a row is filed under, as the database sees it.
fn tenant_column(tenant: Option<&TenantId>) -> Option<String> {
    tenant.map(|t| t.as_str().to_string())
}

/// A reservation row, as the queries below read it back.
struct Row_ {
    fingerprint: String,
    owner_id: String,
    fencing_token: i64,
    lease_until: DateTime<Utc>,
    state: String,
    response: Option<Vec<u8>>,
}

impl PostgresOperationReservationStore {
    /// Reads the current row for an identity, if any.
    ///
    /// `tenant_id IS NOT DISTINCT FROM $1`, never `= $1`: the systemwide scope binds
    /// SQL NULL, and `tenant_id = NULL` is unknown rather than true for every row —
    /// including the rows whose tenant genuinely is NULL. With plain equality a
    /// systemwide reservation would be invisible to its own lookup and every attempt
    /// would look like a first one.
    async fn current(
        &self,
        tenant: &Option<String>,
        operation_key: &str,
    ) -> Result<Option<Row_>, ReservationError> {
        let row = sqlx::query(
            r#"SELECT fingerprint, owner_id, fencing_token, lease_until, state, response
               FROM operation_reservations
               WHERE tenant_id IS NOT DISTINCT FROM $1 AND operation_key = $2"#,
        )
        .bind(tenant)
        .bind(operation_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;

        Ok(row.map(|r| Row_ {
            fingerprint: r.get("fingerprint"),
            owner_id: r.get("owner_id"),
            fencing_token: r.get("fencing_token"),
            lease_until: r.get("lease_until"),
            state: r.get("state"),
            response: r.get("response"),
        }))
    }
}

#[async_trait]
impl OperationReservationStore for PostgresOperationReservationStore {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        let tenant = tenant_column(req.tenant.as_ref());
        let key = req.operation_key.as_str().to_string();
        let operation_id = OperationId::new(req.tenant.clone(), req.operation_key.clone());

        // A first attempt inserts. `ON CONFLICT DO NOTHING` makes two racing first
        // attempts resolve without either seeing a unique violation: exactly one
        // inserts, the other falls through to the observation below and sees the
        // winner's row. Doing this as a check-then-insert would leave a window
        // between them.
        let inserted = sqlx::query(
            r#"INSERT INTO operation_reservations
                   (tenant_id, operation_key, fingerprint, owner_id, fencing_token,
                    lease_until, state)
               VALUES ($1, $2, $3, $4, $5, $6, 'in_progress')
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&tenant)
        .bind(&key)
        .bind(req.fingerprint.as_str())
        .bind(req.owner_id.as_str())
        .bind(token_for_storage(FencingToken::initial())?)
        .bind(req.lease_until)
        .execute(&self.pool)
        .await
        .map_err(storage)?;

        if inserted.rows_affected() == 1 {
            return Ok(ReservationOutcome::Fresh(Lease {
                operation_id,
                owner_id: req.owner_id,
                fencing_token: FencingToken::initial(),
                lease_until: req.lease_until,
            }));
        }

        let existing = match self.current(&tenant, &key).await? {
            Some(row) => row,
            // The row vanished between the conflicting insert and this read — a
            // concurrent purge or abandon. Reporting a storage error rather than
            // guessing: the caller retries and gets a clean answer, whereas
            // inventing an outcome here would answer a question nobody asked.
            None => {
                return Err(ReservationError::Backend(
                    "the reservation disappeared between the insert conflict and the read; \
                     retry the reserve"
                        .to_string(),
                ))
            }
        };

        // Fingerprint first, before any ownership or lease consideration. A
        // different fingerprint under the same key is a permanent conflict whatever
        // the lease says — checking ownership first would let a same-owner retry
        // with changed content through as OwnedInProgress.
        if existing.fingerprint != req.fingerprint.as_str() {
            return Ok(ReservationOutcome::Conflict);
        }

        if existing.state == STATE_COMPLETED {
            let response = existing.response.ok_or_else(|| {
                ReservationError::Backend(
                    "a completed reservation has no stored response, which the table's own \
                     CHECK constraint forbids"
                        .to_string(),
                )
            })?;
            return Ok(ReservationOutcome::Succeeded(StoredServiceResponse::new(
                response,
            )));
        }

        let now = self.clock.now();
        if now >= existing.lease_until {
            // Expired: take it over with a strictly greater token.
            //
            // The `lease_until <= $N` predicate is what makes this safe, and it is
            // load-bearing rather than defensive: the read above and this write are
            // separate statements, so between them another caller can take the
            // reservation over or its owner can renew it. Re-checking the lease
            // inside the update means a caller that waited on the row lock is judged
            // against the row that exists, not the row it remembers.
            //
            // This is guarded, and by exactly one test:
            // `integration-tests/tests/fencing_window_postgres.rs` forces the window
            // open with `SELECT … FOR UPDATE`, renews the lease while the takeover
            // blocks, and requires the refusal. Neutralising this predicate makes
            // that test report a takeover of the renewed lease — measured, and with
            // the rest of that suite staying green, so it is the only check.
            //
            // An earlier version of this comment said the predicate was unguarded
            // because the original test had been moved out of the workspace. That was
            // true then and is not now.
            //
            // `fencing_token = $N` is a compare-and-swap on the row version, and it
            // is **redundant given the predicate above**: every path that could
            // change the token also pushes `lease_until` into the future, which the
            // lease predicate already rejects. No test distinguishes the two, and
            // that was checked rather than assumed — removing the token predicate
            // leaves the whole suite green. It stays because a conditional update
            // that names the version it read is the correct shape for one, and
            // because a later change to the lease predicate would otherwise remove
            // the only guard silently. It is not, however, the thing carrying the
            // guarantee, and an earlier version of this comment claimed it was.
            let displaced = token_from_storage(existing.fencing_token)?;
            let next = displaced.next().ok_or(ReservationError::FencingExhausted)?;

            let took_over = sqlx::query(
                r#"UPDATE operation_reservations
                   SET owner_id = $1, fencing_token = $2, lease_until = $3
                   WHERE tenant_id IS NOT DISTINCT FROM $4
                     AND operation_key = $5
                     AND state = 'in_progress'
                     AND fencing_token = $6
                     AND lease_until <= $7"#,
            )
            .bind(req.owner_id.as_str())
            .bind(token_for_storage(next)?)
            .bind(req.lease_until)
            .bind(&tenant)
            .bind(&key)
            .bind(existing.fencing_token)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(storage)?;

            if took_over.rows_affected() == 1 {
                return Ok(ReservationOutcome::TakenOver(Lease {
                    operation_id,
                    owner_id: req.owner_id,
                    fencing_token: next,
                    lease_until: req.lease_until,
                }));
            }

            // Someone else took it over first. Re-read rather than assume: the
            // winner may be this same owner recovering, in which case the honest
            // answer is OwnedInProgress and not OtherInProgress.
            let after = self.current(&tenant, &key).await?.ok_or_else(|| {
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
    }

    async fn renew(
        &self,
        fence: &OwnerFence,
        until: DateTime<Utc>,
    ) -> Result<(), ReservationError> {
        self.mutate_owned(fence, |tenant, key, token, now| {
            sqlx::query(
                r#"UPDATE operation_reservations
                   SET lease_until = $1
                   WHERE tenant_id IS NOT DISTINCT FROM $2
                     AND operation_key = $3
                     AND owner_id = $4
                     AND fencing_token = $5
                     AND state = 'in_progress'
                     AND lease_until > $6"#,
            )
            .bind(until)
            .bind(tenant)
            .bind(key)
            .bind(fence.owner_id.as_str().to_string())
            .bind(token)
            .bind(now)
        })
        .await
    }

    async fn complete(
        &self,
        fence: &OwnerFence,
        response: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        let bytes = response.as_bytes().to_vec();
        self.mutate_owned(fence, move |tenant, key, token, now| {
            sqlx::query(
                r#"UPDATE operation_reservations
                   SET state = 'completed', completed_at = $1, response = $2
                   WHERE tenant_id IS NOT DISTINCT FROM $3
                     AND operation_key = $4
                     AND owner_id = $5
                     AND fencing_token = $6
                     AND state = 'in_progress'
                     AND lease_until > $7"#,
            )
            .bind(now)
            .bind(bytes.clone())
            .bind(tenant)
            .bind(key)
            .bind(fence.owner_id.as_str().to_string())
            .bind(token)
            .bind(now)
        })
        .await
    }

    async fn abandon(&self, fence: &OwnerFence) -> Result<(), ReservationError> {
        self.mutate_owned(fence, |tenant, key, token, now| {
            sqlx::query(
                r#"DELETE FROM operation_reservations
                   WHERE tenant_id IS NOT DISTINCT FROM $1
                     AND operation_key = $2
                     AND owner_id = $3
                     AND fencing_token = $4
                     AND state = 'in_progress'
                     AND lease_until > $5"#,
            )
            .bind(tenant)
            .bind(key)
            .bind(fence.owner_id.as_str().to_string())
            .bind(token)
            .bind(now)
        })
        .await
    }

    async fn purge_completed_before(
        &self,
        cutoff: DateTime<Utc>,
        batch: usize,
    ) -> Result<u64, ReservationError> {
        // `completed_at < $1`, strictly: a reservation completed at exactly the
        // cutoff survives.
        //
        // `state = 'completed'` is what keeps an in-progress reservation out
        // regardless of age. The predicate is on state rather than on
        // `completed_at IS NOT NULL` because state is the thing the guarantee is
        // stated in terms of — the table's CHECK already ties the two together, and
        // filtering on the derived column would make the query's intent depend on
        // that constraint rather than saying it.
        //
        // `DELETE` takes no `LIMIT`, so the batch bound goes through a subquery over
        // `ctid`. The subquery has no `ORDER BY`: selection within a batch is
        // deliberately outside the contract (AD-11), and adding one here would make
        // an ordering observable that a caller must not depend on.
        let deleted = sqlx::query(
            r#"DELETE FROM operation_reservations
               WHERE ctid IN (
                   SELECT ctid FROM operation_reservations
                   WHERE state = 'completed' AND completed_at < $1
                   LIMIT $2
               )"#,
        )
        .bind(cutoff)
        .bind(batch_for_storage(batch))
        .execute(&self.pool)
        .await
        .map_err(storage)?;

        Ok(deleted.rows_affected())
    }

    async fn probe(&self) -> Result<(), ReservationError> {
        // Against the reservation table, not `SELECT 1`. Both prove a
        // connection can be acquired and a statement round-tripped; only this
        // one also proves the schema this store writes to is actually there.
        // A pool that connects to a database whose migration has not run yet
        // answers `SELECT 1` happily, and the first real `reserve` is then the
        // thing that discovers the missing table — after the process has
        // already been reported ready and handed traffic.
        //
        // `LIMIT 1` with no predicate and a discarded row: the planner stops
        // at the first tuple (or at the first empty page), so the cost does
        // not grow with the table. Reading a row rather than counting them
        // keeps it that way — `COUNT(*)` would scan the whole table on every
        // readiness probe.
        //
        // `fetch_optional`, because an empty table is a perfectly reachable
        // store. Zero rows is not an error here; it is the normal state of a
        // freshly migrated deployment.
        sqlx::query("SELECT 1 FROM operation_reservations LIMIT 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?;

        Ok(())
    }
}

impl PostgresOperationReservationStore {
    /// Runs one fence-verified mutation and turns "no row matched" into
    /// [`ReservationError::StaleOwner`].
    ///
    /// The three mutators share this shape because they share the whole obligation:
    /// verify the full triple — identity, owner, fencing token — and additionally
    /// reject an already-expired lease even when the triple matches. Both live in
    /// the `WHERE` clause, so verification and mutation are one statement: a
    /// separate read-then-write would leave a window in which the lease lapses or
    /// the row is taken over between the check and the change.
    ///
    /// Zero rows affected therefore means one of "not yours", "not that token", or
    /// "no longer valid", and the port makes no distinction among them: all three
    /// are `StaleOwner`, and all three leave the reservation unmodified.
    async fn mutate_owned<F>(&self, fence: &OwnerFence, build: F) -> Result<(), ReservationError>
    where
        F: FnOnce(
            Option<String>,
            String,
            i64,
            DateTime<Utc>,
        )
            -> sqlx::query::Query<'static, sqlx::Postgres, sqlx::postgres::PgArguments>,
    {
        let tenant = tenant_column(fence.operation_id.tenant());
        let key = fence.operation_id.operation_key().as_str().to_string();
        // Converted here rather than in each caller: one place to be right about the
        // range difference between the domain's counter and the column.
        let token = token_for_storage(fence.fencing_token)?;
        let now = self.clock.now();

        let affected = build(tenant, key, token, now)
            .execute(&self.pool)
            .await
            .map_err(storage)?
            .rows_affected();

        if affected == 0 {
            return Err(ReservationError::StaleOwner);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stored-token guard admits exactly the positive values.
    ///
    /// Zero is the case worth naming: `u64::try_from` accepts it, so a guard expressed
    /// only as that conversion would build a `FencingToken(0)` — a token no sequence
    /// mints, since it starts at one — while claiming to reject anything not positive.
    /// That is what this test exists to pin, and one is included so the guard is shown
    /// to admit the first real token rather than reject everything.
    #[test]
    fn a_stored_token_must_be_positive() {
        // Zero first, deliberately. It is the value a `u64` conversion accepts, so it
        // is the one that regresses if the guard is ever expressed as that conversion
        // alone — and a failure here should name that case rather than surface as a
        // message quibble about `i64::MIN`, which any conversion rejects anyway.
        for rejected in [0, -1, i64::MIN] {
            let err = match token_from_storage(rejected) {
                Err(err) => err,
                Ok(token) => panic!(
                    "a non-positive stored token must be refused, but {rejected} was accepted \
                     as {token:?}"
                ),
            };
            match err {
                ReservationError::Backend(message) => assert!(
                    message.contains("is not positive"),
                    "the error must say what was wrong with the stored value: {message}"
                ),
                other => panic!("expected Backend for {rejected}, got {other:?}"),
            }
        }

        let accepted =
            token_from_storage(1).expect("the first token the sequence mints must be accepted");
        assert_eq!(accepted, FencingToken::initial());

        let large = token_from_storage(i64::MAX).expect("the largest storable token is valid");
        assert_eq!(large.value(), i64::MAX as u64);
    }

    /// A token beyond the column's range is refused rather than wrapped.
    ///
    /// The other half of the boundary the durable test covers end-to-end: this pins the
    /// conversion itself, so a regression is named here before it has to be inferred
    /// from a reservation behaving oddly.
    #[test]
    fn a_token_the_column_cannot_hold_is_refused() {
        let at_limit = FencingToken::from_value(i64::MAX as u64);
        assert_eq!(
            token_for_storage(at_limit).expect("the limit itself is storable"),
            i64::MAX
        );

        let past_limit = at_limit.next().expect("u64 still has room past i64::MAX");
        assert_eq!(
            token_for_storage(past_limit),
            Err(ReservationError::FencingExhausted),
            "a token u64 can hold but the column cannot must report exhaustion, not wrap"
        );
    }
}
