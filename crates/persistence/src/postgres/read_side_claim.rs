//! PostgreSQL durable [`ReadSideClaimStore`] (PROD-014C).
//!
//! # One statement, no check-then-act window
//!
//! `try_claim` is a single `INSERT … ON CONFLICT … DO UPDATE … WHERE
//! lease_until <= $now RETURNING fencing_token`. There is no read between
//! deciding whether the incumbent lease has lapsed and taking the claim over
//! — the same shape `reservation.rs`'s takeover branch uses, one level
//! earlier: here the identity may not exist yet, so the takeover branch and
//! the first-claim branch are the same statement rather than two.
//!
//! Taking over mints the next token as `fencing_token + 1` in SQL, not in
//! Rust — there is no prior row to read a token out of when the identity is
//! being claimed for the first time, so there is nothing for
//! `FencingToken::next()` to advance. A token that would overflow the
//! `BIGINT` column raises `22003` (`numeric_value_out_of_range`), mapped
//! below to [`ClaimError::FencingExhausted`] before the general
//! `Transient`/`Fatal` split ever sees it.
//!
//! # Release is an expiry, not a delete
//!
//! `release` sets `lease_until = now`, exactly like the migration's own doc
//! comment requires — never `DELETE` — so the fencing token stays strictly
//! monotone across the release boundary and a taken-over row is never
//! resurrected as fresh.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use ego_domain::operation::ReservationError;
use ego_domain::Clock;
use ego_persistence_api::read_side::claim::{ClaimError, ClaimFence, ClaimId, ReadSideClaimStore};
use ego_persistence_api::operation::reservation::OwnerId;

use crate::postgres::is_fatal;
use crate::postgres::reservation::{token_for_storage, token_from_storage};

/// Converts the reservation-store's storage-conversion error into this
/// port's error type.
///
/// [`token_from_storage`]/[`token_for_storage`] are reused verbatim from
/// `reservation.rs` (AD-3) and only ever fail with
/// [`ReservationError::Backend`] (a malformed stored value) or
/// [`ReservationError::FencingExhausted`] — never `StaleOwner`, which they
/// have no way to observe; the wildcard arm exists only so this stays
/// exhaustive against a future variant.
fn to_claim_error(err: ReservationError) -> ClaimError {
    match err {
        ReservationError::FencingExhausted => ClaimError::FencingExhausted,
        ReservationError::Backend(msg) => ClaimError::Fatal(msg),
        other => ClaimError::Fatal(format!("unexpected reservation error: {other}")),
    }
}

/// Maps a storage failure into the port's `Transient`/`Fatal`/`FencingExhausted`
/// split.
///
/// `22003` (`numeric_value_out_of_range`) is checked first, ahead of the
/// general [`is_fatal`] classification: it is what PostgreSQL raises when
/// `fencing_token + 1` would overflow the column, and that is this port's
/// own [`ClaimError::FencingExhausted`], not an ordinary fatal storage
/// failure.
fn claim_error(err: sqlx::Error) -> ClaimError {
    if let sqlx::Error::Database(db) = &err {
        if db.code().as_deref() == Some("22003") {
            return ClaimError::FencingExhausted;
        }
    }
    let text = err.to_string();
    if is_fatal(&err) {
        ClaimError::Fatal(text)
    } else {
        ClaimError::Transient(text)
    }
}

/// A durable [`ReadSideClaimStore`] backed by one PostgreSQL table
/// (`projection_claims`).
pub struct PostgreSQLReadSideClaimStore {
    pool: PgPool,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for PostgreSQLReadSideClaimStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgreSQLReadSideClaimStore")
            .field("pool", &self.pool)
            .finish_non_exhaustive()
    }
}

impl PostgreSQLReadSideClaimStore {
    /// Builds a store over `pool`, reading "now" from `clock` — never from
    /// the database's own `now()` (AD-8, mirrored from `reservation.rs`):
    /// deterministic under a test clock, and the honest statement of what
    /// clock skew across nodes can and cannot cost, stays in one place.
    ///
    /// `pool` must already have had `crate::postgres::migrations::run`
    /// applied — migration `016` creates the table this store reads and
    /// writes.
    pub fn new(pool: PgPool, clock: Arc<dyn Clock>) -> Self {
        Self { pool, clock }
    }

    /// Runs one fence-verified mutation and turns "no row matched" into
    /// [`ClaimError::StaleOwner`].
    ///
    /// Shares `reservation.rs`'s `mutate_owned` shape: `renew` and `release`
    /// share the identical obligation of verifying the full
    /// `claim_id + owner_id + fencing_token` triple, plus an unexpired
    /// lease, inside the same statement that mutates. Zero rows affected
    /// means one of "not yours", "not that token", or "already lapsed", and
    /// the port makes no distinction among them: all three are
    /// `StaleOwner`, and all three leave the claim unmodified.
    async fn mutate_claimed<F>(&self, fence: &ClaimFence, build: F) -> Result<(), ClaimError>
    where
        F: FnOnce(
            String,
            String,
            String,
            String,
            i64,
            DateTime<Utc>,
        )
            -> sqlx::query::Query<'static, sqlx::Postgres, sqlx::postgres::PgArguments>,
    {
        let projection_id = fence.claim_id.projection_id.clone();
        let tag = fence.claim_id.tag.value().to_string();
        let tenant = fence.claim_id.tenant.clone();
        let owner_id = fence.owner_id.as_str().to_string();
        let token = token_for_storage(fence.fencing_token).map_err(to_claim_error)?;
        let now = self.clock.now();

        let affected = build(projection_id, tag, tenant, owner_id, token, now)
            .execute(&self.pool)
            .await
            .map_err(claim_error)?
            .rows_affected();

        if affected == 0 {
            return Err(ClaimError::StaleOwner);
        }
        Ok(())
    }
}

#[async_trait]
impl ReadSideClaimStore for PostgreSQLReadSideClaimStore {
    fn is_durable(&self) -> bool {
        true
    }

    async fn try_claim(
        &self,
        claim_id: &ClaimId,
        owner_id: &OwnerId,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<ClaimFence>, ClaimError> {
        let now = self.clock.now();

        // One statement handles both branches: a first claim on an identity
        // that has no row yet, and a takeover of a row whose lease has
        // lapsed. `fencing_token + 1` reads the *existing* row's token
        // (`projection_claims.fencing_token`, the table, not `EXCLUDED`) so a
        // takeover strictly advances it; a fresh identity starts at the
        // column's `1` default via the inserted value.
        //
        // The `WHERE` guards the `DO UPDATE` only — an outright conflict on
        // an unexpired lease leaves the existing row untouched and the
        // statement affects zero rows, which is exactly `Ok(None)`: a live
        // claim already holds it.
        let row = sqlx::query(
            r#"INSERT INTO projection_claims
                   (projection_id, tag, tenant, owner_id, fencing_token, lease_until)
               VALUES ($1, $2, $3, $4, 1, $5)
               ON CONFLICT (projection_id, tag, tenant) DO UPDATE
                   SET owner_id = EXCLUDED.owner_id,
                       fencing_token = projection_claims.fencing_token + 1,
                       lease_until = EXCLUDED.lease_until
                   WHERE projection_claims.lease_until <= $6
               RETURNING fencing_token"#,
        )
        .bind(&claim_id.projection_id)
        .bind(claim_id.tag.value())
        .bind(&claim_id.tenant)
        .bind(owner_id.as_str())
        .bind(lease_until)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(claim_error)?;

        let Some(row) = row else {
            return Ok(None);
        };
        let raw: i64 = row.get("fencing_token");
        let fencing_token = token_from_storage(raw).map_err(to_claim_error)?;

        Ok(Some(ClaimFence {
            claim_id: claim_id.clone(),
            owner_id: owner_id.clone(),
            fencing_token,
        }))
    }

    async fn renew(&self, fence: &ClaimFence, lease_until: DateTime<Utc>) -> Result<(), ClaimError> {
        self.mutate_claimed(fence, move |projection_id, tag, tenant, owner_id, token, now| {
            sqlx::query(
                r#"UPDATE projection_claims
                   SET lease_until = $1
                   WHERE projection_id = $2
                     AND tag = $3
                     AND tenant = $4
                     AND owner_id = $5
                     AND fencing_token = $6
                     AND lease_until > $7"#,
            )
            .bind(lease_until)
            .bind(projection_id)
            .bind(tag)
            .bind(tenant)
            .bind(owner_id)
            .bind(token)
            .bind(now)
        })
        .await
    }

    async fn release(&self, fence: &ClaimFence) -> Result<(), ClaimError> {
        self.mutate_claimed(fence, |projection_id, tag, tenant, owner_id, token, now| {
            sqlx::query(
                r#"UPDATE projection_claims
                   SET lease_until = $1
                   WHERE projection_id = $2
                     AND tag = $3
                     AND tenant = $4
                     AND owner_id = $5
                     AND fencing_token = $6
                     AND lease_until > $7"#,
            )
            .bind(now)
            .bind(projection_id)
            .bind(tag)
            .bind(tenant)
            .bind(owner_id)
            .bind(token)
            .bind(now)
        })
        .await
    }
}
