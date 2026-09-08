//! Stoolap-backed [`ReadSideClaimStore`].
//!
//! `try_claim` ports `StoolapOperationReservationStore::reserve`'s two-statement
//! CAS (design.md AD-5): `INSERT ... ON CONFLICT DO NOTHING`, then a
//! conditional `UPDATE` that re-verifies the live row's `fencing_token` and
//! `lease_until` before granting a takeover. This is the only Stoolap-0.4
//! CAS shape proven in this crate — never a single-statement
//! `INSERT ... ON CONFLICT ... DO UPDATE` (unproven against Stoolap 0.4).
//!
//! `renew` and `release` share one private [`StoolapReadSideClaimStore::set_lease`]
//! (design.md AD-6): one statement, `UPDATE ... WHERE claim_id AND owner_id AND
//! fencing_token AND lease_until > $now`. `release` is never a `DELETE` — it
//! sets an already-expired `lease_until`, so the row persists, the fencing
//! token stays strictly monotone, and the claim is immediately reclaimable.
//!
//! # Time comes from the injected `Clock`, never `now()`
//!
//! Every expiry comparison reads `clock.now()`; this store never calls
//! `Utc::now()`, `SystemTime::now()`, or a SQL `now()`. The *lease bound*
//! (`lease_until`) is always the caller's — it arrives as a parameter on
//! `try_claim`/`renew` and is stored verbatim (design.md AD-7). This is the
//! satisfiable reading of "lease expiry is caller-computed": the lease bound
//! is caller-computed, and the store's own "now" comes only from the
//! injected `Clock`, never from ambient system time.
//!
//! # Blocking I/O
//!
//! Every `Database::execute`/`query` call is synchronous, disk-touching
//! I/O. All trait methods therefore run their body via
//! [`StoolapReadSideClaimStore::run_blocking`], which hands the closure to
//! [`tokio::task::spawn_blocking`] — never `block_in_place`, which panics
//! outside a multi-threaded Tokio runtime.
//!
//! # Concurrency scope
//!
//! Safe for concurrent use by multiple async tasks within one process
//! holding this store's underlying file. Not supported, not tested, and not
//! claimed to be safe across multiple OS processes or multiple nodes (see
//! `read_side` module doc).

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use stoolap::{Database, Value};

use ego_domain::operation::{FencingToken, OwnerId, ReservationError};
use ego_domain::Clock;
use ego_persistence_api::read_side::claim::{ClaimError, ClaimFence, ClaimId, ReadSideClaimStore};

use crate::persistence::stoolap_common::{
    dsn_declares_sync_full, dsn_for, is_write_conflict, token_for_storage, token_from_storage,
};

const CREATE_CLAIMS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS projection_claims (
    projection_id TEXT      NOT NULL,
    tag           TEXT      NOT NULL,
    tenant        TEXT      NOT NULL,
    owner_id      TEXT      NOT NULL,
    fencing_token INTEGER   NOT NULL,
    lease_until   TIMESTAMP NOT NULL,
    UNIQUE (projection_id, tag, tenant)
)";

/// Converts the reservation-store's storage-conversion error into this
/// port's error type — mirrors
/// `crates/persistence/src/postgres/read_side_claim.rs`'s `to_claim_error`
/// (design.md AD-11). [`token_from_storage`]/[`token_for_storage`] only ever
/// fail with [`ReservationError::Backend`] (a malformed stored value) or
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

/// Maps a raw Stoolap error to the port's `Transient`/`Fatal` split
/// (design.md AD-8): a write conflict is retry-safe, everything else is
/// fatal by default (fail-loud). `affected == 0` on a fence-verified
/// mutation is classified separately, as `StaleOwner` — never through this
/// function.
fn classify_error(e: stoolap::Error) -> ClaimError {
    if is_write_conflict(&e) {
        ClaimError::Transient(e.to_string())
    } else {
        ClaimError::Fatal(e.to_string())
    }
}

/// A claim row, as the queries below read it back.
///
/// No `owner_id` field: unlike `reserve`, `try_claim`'s takeover branch
/// needs no re-read to distinguish "the caller already owns it" from
/// "someone else does" — the port has no `OwnedInProgress`/`OtherInProgress`
/// split, so `Ok(None)` is the whole answer either way (design.md AD-5).
struct ClaimRow {
    fencing_token: i64,
    lease_until: DateTime<Utc>,
}

/// Reads a `TIMESTAMP` column back. No `FromValue` impl exists for
/// `DateTime<Utc>` in Stoolap 0.4.0, so every timestamp column is read via
/// `row.get_value` and matched manually — the same pattern
/// `StoolapOperationReservationStore::read_timestamp` already uses.
fn read_timestamp(
    row: &stoolap::ResultRow,
    idx: usize,
    column: &str,
) -> Result<DateTime<Utc>, ClaimError> {
    match row.get_value(idx) {
        Some(Value::Timestamp(dt)) => Ok(*dt),
        other => Err(ClaimError::Fatal(format!(
            "{column} column did not hold a Timestamp value: {other:?}"
        ))),
    }
}

/// A durable, single-process [`ReadSideClaimStore`] backed by an embedded
/// Stoolap database.
pub struct StoolapReadSideClaimStore {
    db: Database,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for StoolapReadSideClaimStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoolapReadSideClaimStore")
            .field("dsn", &self.db.dsn())
            .finish()
    }
}

impl StoolapReadSideClaimStore {
    /// Opens (creating the `projection_claims` table if absent) a
    /// Stoolap-backed claim store at `path`, reading time from `clock`.
    ///
    /// Fails closed (design.md AD-9, the same pattern
    /// `StoolapOffsetStore::open` already follows): only ever returns a
    /// store whose live engine reports `sync=full`, so `is_durable()` never
    /// outlives-lies about how the store was opened.
    pub async fn open(path: &Path, clock: Arc<dyn Clock>) -> Result<Self, ClaimError> {
        let dsn = dsn_for(path);
        let db = Database::open(&dsn).map_err(|e| ClaimError::Fatal(e.to_string()))?;

        if !dsn_declares_sync_full(db.dsn()) {
            return Err(ClaimError::Fatal(format!(
                "stoolap engine at {:?} is not configured for durable sync (sync=full); \
                 refusing to open a StoolapReadSideClaimStore that would misreport is_durable()",
                db.dsn()
            )));
        }

        // Dialect note (AD-3): a composite `PRIMARY KEY` is parsed but
        // silently NOT enforced by Stoolap 0.4.0 (no constraint, no index) —
        // `UNIQUE` is fully enforced and is what `ON CONFLICT` matches
        // against, so it is used here instead.
        db.execute(CREATE_CLAIMS_TABLE, ())
            .map_err(|e| ClaimError::Fatal(e.to_string()))?;

        Ok(Self { db, clock })
    }

    /// Runs `f` against a cloned `Database` handle on Tokio's
    /// blocking-thread pool — the same shape as
    /// `StoolapOffsetStore::run_blocking`.
    async fn run_blocking<F, R>(&self, f: F) -> Result<R, ClaimError>
    where
        F: FnOnce(&Database) -> Result<R, ClaimError> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || f(&db))
            .await
            .map_err(|e| ClaimError::Fatal(format!("blocking task panicked: {e}")))?
    }

    /// Reads the current row for a claim identity, if any.
    fn current(
        db: &Database,
        projection_id: &str,
        tag: &str,
        tenant: &str,
    ) -> Result<Option<ClaimRow>, ClaimError> {
        let mut rows = db
            .query(
                "SELECT fencing_token, lease_until FROM projection_claims
                 WHERE projection_id = $1 AND tag = $2 AND tenant = $3",
                (
                    projection_id.to_string(),
                    tag.to_string(),
                    tenant.to_string(),
                ),
            )
            .map_err(classify_error)?;
        let row = match rows.next() {
            Some(row) => row.map_err(classify_error)?,
            None => return Ok(None),
        };

        let fencing_token: i64 = row.get(0).map_err(classify_error)?;
        let lease_until = read_timestamp(&row, 1, "lease_until")?;

        Ok(Some(ClaimRow {
            fencing_token,
            lease_until,
        }))
    }

    #[cfg(test)]
    fn dsn(&self) -> &str {
        self.db.dsn()
    }
}

#[async_trait]
impl ReadSideClaimStore for StoolapReadSideClaimStore {
    /// Truthful by construction (design.md AD-9): `open()` only ever
    /// returns a store whose live engine reports `sync=full`, so this
    /// re-derives from the same invariant rather than a hardcoded value.
    fn is_durable(&self) -> bool {
        dsn_declares_sync_full(self.db.dsn())
    }

    async fn try_claim(
        &self,
        claim_id: &ClaimId,
        owner_id: &OwnerId,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<ClaimFence>, ClaimError> {
        let projection_id = claim_id.projection_id.clone();
        let tag = claim_id.tag.value().to_string();
        let tenant = claim_id.tenant.clone();
        let owner = owner_id.as_str().to_string();
        let claim_id = claim_id.clone();
        let owner_id = owner_id.clone();
        let clock = self.clock.clone();

        self.run_blocking(move |db| {
            // Step 1 (AD-5): a first attempt inserts. `ON CONFLICT DO
            // NOTHING` makes two racing first attempts resolve without
            // either seeing a unique violation: exactly one inserts, the
            // other falls through to the observation below and sees the
            // winner's row.
            let inserted = db
                .execute(
                    "INSERT INTO projection_claims
                        (projection_id, tag, tenant, owner_id, fencing_token, lease_until)
                     VALUES ($1, $2, $3, $4, $5, $6)
                     ON CONFLICT (projection_id, tag, tenant) DO NOTHING",
                    (
                        projection_id.clone(),
                        tag.clone(),
                        tenant.clone(),
                        owner.clone(),
                        token_for_storage(FencingToken::initial()).map_err(to_claim_error)?,
                        lease_until,
                    ),
                )
                .map_err(classify_error)?;

            if inserted == 1 {
                return Ok(Some(ClaimFence {
                    claim_id,
                    owner_id,
                    fencing_token: FencingToken::initial(),
                }));
            }

            let existing = match Self::current(db, &projection_id, &tag, &tenant)? {
                Some(row) => row,
                None => {
                    return Err(ClaimError::Transient(
                        "claim row vanished after the insert conflict; retry".to_string(),
                    ))
                }
            };

            let now = clock.now();
            if now < existing.lease_until {
                // A live lease holds the claim: a refusal, not a failure.
                return Ok(None);
            }

            // Lapsed: take it over with a strictly greater token. The
            // `fencing_token = $displaced AND lease_until <= $now` predicate
            // is re-verified here against the LIVE row, not trusted from the
            // read above — the read and this write are separate statements,
            // so between them a peer can take the claim over or its owner
            // can renew it (design.md AD-5).
            let displaced = token_from_storage(existing.fencing_token).map_err(to_claim_error)?;
            // Checked explicitly (design.md AD-5 step 4), not left to
            // `token_for_storage`'s own overflow guard below: this is the
            // exhaustion check the port's contract names, reported as
            // `FencingExhausted` rather than wrapped or truncated.
            let next = displaced.next().ok_or(ClaimError::FencingExhausted)?;

            let took_over = db
                .execute(
                    "UPDATE projection_claims
                     SET owner_id = $1, fencing_token = $2, lease_until = $3
                     WHERE projection_id = $4 AND tag = $5 AND tenant = $6
                       AND fencing_token = $7
                       AND lease_until <= $8",
                    (
                        owner.clone(),
                        token_for_storage(next).map_err(to_claim_error)?,
                        lease_until,
                        projection_id,
                        tag,
                        tenant,
                        existing.fencing_token,
                        now,
                    ),
                )
                .map_err(classify_error)?;

            if took_over == 1 {
                return Ok(Some(ClaimFence {
                    claim_id,
                    owner_id,
                    fencing_token: next,
                }));
            }

            // A peer took over or the owner renewed in the window: a
            // refusal, never a failure (design.md AD-5 step 5).
            Ok(None)
        })
        .await
    }

    async fn renew(
        &self,
        fence: &ClaimFence,
        lease_until: DateTime<Utc>,
    ) -> Result<(), ClaimError> {
        self.set_lease(fence, lease_until).await
    }

    async fn release(&self, fence: &ClaimFence) -> Result<(), ClaimError> {
        let now = self.clock.now();
        self.set_lease(fence, now).await
    }
}

impl StoolapReadSideClaimStore {
    /// The one statement `renew` and `release` share (design.md AD-6):
    /// `renew` calls this with the caller's `lease_until`, `release` calls
    /// it with `clock.now()` — never a `DELETE`. `affected == 0` means the
    /// presented fence no longer matches the live row (a stale owner) or
    /// the lease had already lapsed (a lapsed holder may not resurrect its
    /// claim); both classify as `StaleOwner`, and the statement's `WHERE`
    /// clause guarantees the row is unmodified in that case — there is
    /// nothing else to roll back.
    ///
    /// ponytail: `renew`/`release` are the *identical* statement with one
    /// different bound value — one concrete helper, not a generic
    /// cross-store `mutate_owned` combinator (design.md AD-6; the threshold
    /// `reservation.rs`'s own ponytail comment names — "a fourth mutator
    /// with the same ownership predicate" — is not met across modules with
    /// different error types, key arity, and predicates).
    async fn set_lease(
        &self,
        fence: &ClaimFence,
        new_lease_until: DateTime<Utc>,
    ) -> Result<(), ClaimError> {
        let projection_id = fence.claim_id.projection_id.clone();
        let tag = fence.claim_id.tag.value().to_string();
        let tenant = fence.claim_id.tenant.clone();
        let owner = fence.owner_id.as_str().to_string();
        let token = token_for_storage(fence.fencing_token).map_err(to_claim_error)?;
        let clock = self.clock.clone();

        self.run_blocking(move |db| {
            let now = clock.now();
            let affected = db
                .execute(
                    "UPDATE projection_claims SET lease_until = $1
                     WHERE projection_id = $2 AND tag = $3 AND tenant = $4
                       AND owner_id = $5 AND fencing_token = $6
                       AND lease_until > $7",
                    (
                        new_lease_until,
                        projection_id,
                        tag,
                        tenant,
                        owner,
                        token,
                        now,
                    ),
                )
                .map_err(classify_error)?;

            if affected == 0 {
                return Err(ClaimError::StaleOwner);
            }
            Ok(())
        })
        .await
    }
}

/// Colocated unit tests open a real embedded Stoolap database per test, each
/// against its own `tempfile` path — same documented exception
/// `persistence/repository.rs` relies on (`skills/testing/SKILL.md` Rule 1,
/// design.md AD-3 criterion 4, AD-4, AD-7, and the same embedded/file-backed
/// reasoning as AD-9 criterion 1).
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use ego_persistence_api::read_side::event_tag::EventTag;
    use ego_testkit::TestClock;

    /// Every test that touches the database serializes on stoolap's own
    /// failpoint lock, matching `reservation.rs`'s guard: `WAL_WRITE_FAIL`
    /// is a process-wide `AtomicBool`, so an unguarded test here can
    /// observe another module's failpoint test armed mid-run.
    fn db_test_guard() -> stoolap::test_failpoints::FailpointGuard {
        stoolap::test_failpoints::FailpointGuard::new()
    }

    fn epoch() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
    }

    fn claim_id(tenant: &str) -> ClaimId {
        ClaimId {
            projection_id: "proj".to_string(),
            tag: EventTag::new("users-by-tenant"),
            tenant: tenant.to_string(),
        }
    }

    async fn fresh_store() -> (StoolapReadSideClaimStore, Arc<TestClock>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let clock = Arc::new(TestClock::new(epoch()));
        let store = StoolapReadSideClaimStore::open(dir.path(), clock.clone())
            .await
            .expect("open StoolapReadSideClaimStore");
        (store, clock, dir)
    }

    #[tokio::test]
    async fn an_opened_store_requested_full_sync() {
        let _fp = db_test_guard();
        let (store, _clock, dir) = fresh_store().await;
        assert_eq!(store.dsn(), dsn_for(dir.path()));
        assert!(store.is_durable());
    }

    /// AD-9 regression guard, mirroring `StoolapOffsetStore`'s identical
    /// test: a live, weakly-configured engine already holding `path` when
    /// `open()` is called must be refused, not silently accepted.
    #[tokio::test]
    async fn open_refuses_a_path_already_locked_by_a_non_durable_engine() {
        let _fp = db_test_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        let weak_dsn = format!("file://{}", dir.path().display());
        let _weak_db = Database::open(&weak_dsn).unwrap();

        let err = StoolapReadSideClaimStore::open(dir.path(), Arc::new(TestClock::new(epoch())))
            .await
            .expect_err("expected open() to refuse a path locked by a non-durable engine");
        assert!(matches!(err, ClaimError::Fatal(_)));
    }

    #[tokio::test]
    async fn try_claim_grants_a_fresh_claim_with_no_live_lease() {
        let _fp = db_test_guard();
        let (store, _clock, _dir) = fresh_store().await;
        let id = claim_id("tenant-a");
        let owner = OwnerId::new("owner-1");

        let fence = store
            .try_claim(&id, &owner, epoch() + Duration::seconds(30))
            .await
            .unwrap()
            .expect("no live lease exists; the claim must be granted");
        assert_eq!(fence.fencing_token, FencingToken::initial());
    }

    #[tokio::test]
    async fn a_live_claim_refuses_a_second_claimant() {
        let _fp = db_test_guard();
        let (store, _clock, _dir) = fresh_store().await;
        let id = claim_id("tenant-b");
        let owner_a = OwnerId::new("owner-a");
        let owner_b = OwnerId::new("owner-b");
        let lease_until = epoch() + Duration::seconds(30);

        store.try_claim(&id, &owner_a, lease_until).await.unwrap();
        let refusal = store.try_claim(&id, &owner_b, lease_until).await.unwrap();
        assert!(
            refusal.is_none(),
            "a live lease must refuse a second claimant"
        );
    }

    #[tokio::test]
    async fn takeover_of_a_lapsed_lease_mints_a_strictly_greater_token() {
        let _fp = db_test_guard();
        let (store, clock, _dir) = fresh_store().await;
        let id = claim_id("tenant-c");
        let owner_a = OwnerId::new("owner-a");
        let owner_b = OwnerId::new("owner-b");

        let original = store
            .try_claim(&id, &owner_a, epoch() + Duration::seconds(30))
            .await
            .unwrap()
            .expect("fresh grant");

        clock.advance(Duration::seconds(31));
        let taken_over = store
            .try_claim(&id, &owner_b, clock.now() + Duration::seconds(30))
            .await
            .unwrap()
            .expect("a lapsed lease must be taken over");

        assert!(
            taken_over.fencing_token > original.fencing_token,
            "the new token must be strictly greater than the lapsed one"
        );
        // The lapsed holder's fence no longer verifies.
        assert_eq!(
            store
                .renew(&original, clock.now() + Duration::seconds(60))
                .await,
            Err(ClaimError::StaleOwner)
        );
    }

    #[tokio::test]
    async fn fencing_exhaustion_is_reported_not_wrapped() {
        let _fp = db_test_guard();
        let (store, clock, _dir) = fresh_store().await;
        let id = claim_id("tenant-d");
        let owner_a = OwnerId::new("owner-a");
        let owner_b = OwnerId::new("owner-b");

        store
            .try_claim(&id, &owner_a, epoch() + Duration::seconds(30))
            .await
            .unwrap()
            .expect("fresh grant");

        // Force the stored token to the maximum representable value so the
        // next takeover's `FencingToken::next()` returns `None`.
        store
            .db
            .execute(
                "UPDATE projection_claims SET fencing_token = $1
                 WHERE projection_id = 'proj' AND tag = 'users-by-tenant' AND tenant = 'tenant-d'",
                (i64::MAX,),
            )
            .unwrap();

        clock.advance(Duration::seconds(31));
        let outcome = store
            .try_claim(&id, &owner_b, clock.now() + Duration::seconds(30))
            .await;
        assert_eq!(
            outcome,
            Err(ClaimError::FencingExhausted),
            "exhaustion must be reported, never wrapped or truncated into a token"
        );
    }

    #[tokio::test]
    async fn renew_and_release_reject_a_stale_or_lapsed_fence_without_mutating_state() {
        let _fp = db_test_guard();
        let (store, clock, _dir) = fresh_store().await;
        let id = claim_id("tenant-e");
        let owner_a = OwnerId::new("owner-a");
        let owner_b = OwnerId::new("owner-b");
        let lease_until = epoch() + Duration::seconds(30);

        let original = store
            .try_claim(&id, &owner_a, lease_until)
            .await
            .unwrap()
            .expect("fresh grant");

        // A fence that no longer matches the live claim (wrong token) fails
        // StaleOwner on both mutators, without mutating stored state.
        let mismatched = ClaimFence {
            claim_id: id.clone(),
            owner_id: owner_a.clone(),
            fencing_token: FencingToken::from_value(original.fencing_token.value() + 1),
        };
        assert_eq!(
            store
                .renew(&mismatched, clock.now() + Duration::seconds(60))
                .await,
            Err(ClaimError::StaleOwner)
        );
        assert_eq!(
            store.release(&mismatched).await,
            Err(ClaimError::StaleOwner)
        );

        // A fence whose lease has already lapsed also fails StaleOwner — a
        // lapsed holder may not resurrect its own claim.
        clock.advance(Duration::seconds(31));
        assert_eq!(
            store
                .renew(&original, clock.now() + Duration::seconds(60))
                .await,
            Err(ClaimError::StaleOwner)
        );
        assert_eq!(store.release(&original).await, Err(ClaimError::StaleOwner));

        // Neither rejected attempt modified the claim: a fresh takeover from
        // a different owner still mints exactly the next token after the
        // original, proving the stored fencing_token was never touched by
        // the rejected calls above.
        let taken_over = store
            .try_claim(&id, &owner_b, clock.now() + Duration::seconds(30))
            .await
            .unwrap()
            .expect("the lapsed lease must still be takeover-eligible");
        assert_eq!(
            taken_over.fencing_token,
            FencingToken::from_value(original.fencing_token.value() + 1)
        );
    }

    #[tokio::test]
    async fn release_marks_the_claim_expired_not_deleted() {
        let _fp = db_test_guard();
        let (store, clock, _dir) = fresh_store().await;
        let id = claim_id("tenant-f");
        let owner_a = OwnerId::new("owner-a");
        let owner_b = OwnerId::new("owner-b");

        let held = store
            .try_claim(&id, &owner_a, clock.now() + Duration::seconds(30))
            .await
            .unwrap()
            .expect("fresh grant");

        store.release(&held).await.unwrap();

        // The row still exists with an expired lease: a subsequent
        // try_claim for the identical claim_id succeeds immediately,
        // without waiting for the original lease_until, and the fencing
        // token is unchanged by release itself — the takeover branch mints
        // the next token, proving `release` did not already advance it.
        let after_release = store
            .try_claim(&id, &owner_b, clock.now() + Duration::seconds(30))
            .await
            .unwrap()
            .expect("release must make the claim immediately reclaimable");
        assert_eq!(
            after_release.fencing_token,
            FencingToken::from_value(held.fencing_token.value() + 1)
        );
    }
}
