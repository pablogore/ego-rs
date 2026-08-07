//! The shared conformance contract for [`OperationReservationStore`]
//! implementations.
//!
//! Every scenario here has exactly one definition. The aggregate function
//! composes the groups; it restates nothing.
//!
//! # Why the contract lives here rather than beside an implementation
//!
//! These scenarios were written against the in-memory double and lived inside its
//! own module, where they tested *that store* rather than *the port*. A durable
//! implementation written later would have had its own copy of them, and two
//! copies of a contract drift.
//!
//! That is not a hypothetical worry in this repository. Four divergences between
//! the two `EventStore` implementations were found exactly that way — the
//! systemwide tenant comparison, the unit-of-work version offsets, the
//! absent-stream report, and the tenant-partitioned key of the default store —
//! and in every case the hermetic suite exercised the implementation that happened
//! to be right. Lease ownership, expiry and fencing are the correctness core of
//! idempotency, so a divergence here is a hole in the guarantee rather than an
//! inconvenience.
//!
//! # Each scenario gets a fresh store
//!
//! The caller passes a factory rather than a store. The original scenarios each
//! constructed their own store and their own clock, and they depend on that
//! isolation: several advance the clock and then assert what a *second* owner
//! observes, which a shared store would let a neighbouring scenario disturb.
//! Parameterising the setup preserves that; sharing one store and assigning
//! distinct keys would not — it would be a different contract, quietly.
//!
//! The factory is asynchronous because a durable implementation needs to reach its
//! database to hand back a clean store, while an in-memory one simply allocates.
//!
//! # What the purge contract does and does not promise
//!
//! [`OperationReservationStore::purge_completed_before`] guarantees eligibility, the
//! `batch` limit, the returned count, and that an `InProgress` reservation is never
//! removed. It does **not** guarantee which eligible rows a call chooses when more
//! are eligible than `batch` admits, so the scenarios here assert count,
//! preservation of the ineligible, and eventual drainage through successive calls —
//! never identities. Against a fixed `cutoff` the eligible set strictly shrinks with
//! every call, so drainage holds however each batch chooses.
//!
//! An implementation may pick a deterministic order when its query needs one
//! operationally; the contract must not let a caller observe it.

use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use ego_domain::operation::{
    OperationFingerprint, OperationKey, OperationReservationStore, OwnerFence, OwnerId,
    ReservationError, ReservationOutcome, ReserveRequest, StoredResponse,
};
use ego_domain::Clock;

use crate::TestClock;

/// The instant every scenario starts from. Fixed, so a failure is reproducible and
/// nothing depends on when the suite ran.
fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn request_with_fingerprint(
    owner: &str,
    key: &str,
    fingerprint: &str,
    lease_until: DateTime<Utc>,
) -> ReserveRequest {
    ReserveRequest {
        tenant: None,
        operation_key: OperationKey::parse(key).unwrap(),
        fingerprint: OperationFingerprint::new(fingerprint),
        owner_id: OwnerId::new(owner),
        lease_until,
    }
}

fn request(owner: &str, key: &str, lease_until: DateTime<Utc>) -> ReserveRequest {
    request_with_fingerprint(owner, key, "fp-1", lease_until)
}

fn fence_of(lease: &ego_domain::operation::Lease) -> OwnerFence {
    OwnerFence {
        operation_id: lease.operation_id.clone(),
        owner_id: lease.owner_id.clone(),
        fencing_token: lease.fencing_token,
    }
}

/// Moves `clock` to exactly `at`, using the only mutator `TestClock` exposes.
///
/// A `set` method would be more direct and would widen the double's surface for the
/// benefit of this file alone. Every scenario starts from a fresh clock at
/// [`epoch`] and only ever moves forward, so an advance by the difference is
/// equivalent — and it fails loudly rather than silently rewinding if that ever
/// stops being true.
fn position(clock: &TestClock, at: DateTime<Utc>) {
    let delta = at - clock.now();
    assert!(
        delta >= Duration::zero(),
        "a scenario tried to move the clock backwards, from {} to {at}: TestClock only \
         advances, and a scenario that needs to rewind is asserting something other than \
         what it says",
        clock.now()
    );
    clock.advance(delta);
}

/// Reserves a key, advances the clock past its lease, and lets a second owner take
/// it over. Returns the stale (original) and current (post-takeover) fences.
async fn reserve_then_take_over<S: OperationReservationStore>(
    store: &S,
    clock: &TestClock,
    key: &str,
) -> (OwnerFence, OwnerFence) {
    let first_lease_until = clock.now() + Duration::seconds(30);
    let fresh = store
        .reserve(request("owner-a", key, first_lease_until))
        .await
        .unwrap();
    let stale_fence = match fresh {
        ReservationOutcome::Fresh(lease) => fence_of(&lease),
        other => panic!("expected Fresh, got {other:?}"),
    };

    clock.advance(Duration::seconds(31));
    let second_lease_until = clock.now() + Duration::seconds(30);
    let taken_over = store
        .reserve(request("owner-b", key, second_lease_until))
        .await
        .unwrap();
    let current_fence = match taken_over {
        ReservationOutcome::TakenOver(lease) => fence_of(&lease),
        other => panic!("expected TakenOver, got {other:?}"),
    };

    (stale_fence, current_fence)
}

/// The seven scenarios that pin [`OperationReservationStore::reserve`]: every
/// outcome the port admits, plus the atomic takeover of an expired lease.
///
/// # Panics
///
/// Panics on the first divergence, naming the scenario and what it expected.
pub async fn assert_reserve_conformance<S, F, Fut>(fresh: F)
where
    S: OperationReservationStore,
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = (S, Arc<TestClock>)>,
{
    // --- 1. A key nobody holds is a fresh reservation -----------------------
    {
        let (store, _clock) = fresh().await;
        let outcome = store
            .reserve(request("owner-a", "op-1", epoch() + Duration::seconds(30)))
            .await
            .unwrap();
        assert!(
            matches!(outcome, ReservationOutcome::Fresh(_)),
            "first reservation for a key must be Fresh, got {outcome:?}"
        );
    }

    // --- 2. The same owner mid-lease is recovering, not colliding -----------
    {
        let (store, _clock) = fresh().await;
        let lease_until = epoch() + Duration::seconds(30);
        store
            .reserve(request("owner-a", "op-2", lease_until))
            .await
            .unwrap();

        // Same owner retries before the lease expires (e.g. a client retry).
        let outcome = store
            .reserve(request("owner-a", "op-2", lease_until))
            .await
            .unwrap();
        assert!(
            matches!(outcome, ReservationOutcome::OwnedInProgress(_)),
            "the same owner mid-lease must observe OwnedInProgress, got {outcome:?}"
        );
    }

    // --- 3. A different owner mid-lease must not proceed --------------------
    {
        let (store, _clock) = fresh().await;
        let lease_until = epoch() + Duration::seconds(30);
        store
            .reserve(request("owner-a", "op-3", lease_until))
            .await
            .unwrap();

        let outcome = store
            .reserve(request("owner-b", "op-3", lease_until))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            ReservationOutcome::OtherInProgress,
            "a different owner mid-lease must observe OtherInProgress"
        );
    }

    // --- 4. An expired lease is taken over, with a strictly greater token ---
    {
        let (store, clock) = fresh().await;
        let first_lease_until = epoch() + Duration::seconds(30);
        let fresh_outcome = store
            .reserve(request("owner-a", "op-4", first_lease_until))
            .await
            .unwrap();
        let original_fence = match fresh_outcome {
            ReservationOutcome::Fresh(lease) => lease.fencing_token,
            other => panic!("expected Fresh, got {other:?}"),
        };

        // Advance the deterministic clock past the lease deadline — no real
        // sleep, no timing race.
        clock.advance(Duration::seconds(31));
        let second_lease_until = clock.now() + Duration::seconds(30);

        let outcome = store
            .reserve(request("owner-b", "op-4", second_lease_until))
            .await
            .unwrap();
        match outcome {
            ReservationOutcome::TakenOver(lease) => {
                assert_eq!(lease.owner_id, OwnerId::new("owner-b"));
                assert!(
                    lease.fencing_token > original_fence,
                    "takeover fencing token must be strictly greater than the original"
                );
            }
            other => panic!("expected TakenOver, got {other:?}"),
        }
    }

    // --- 5. The same key under a different fingerprint is a hard conflict ---
    {
        let (store, _clock) = fresh().await;
        let lease_until = epoch() + Duration::seconds(30);
        store
            .reserve(request_with_fingerprint(
                "owner-a",
                "op-10",
                "fp-original",
                lease_until,
            ))
            .await
            .unwrap();

        // A different owner retrying under the identical key but a different
        // fingerprint must never be dispatched as if it were the same
        // operation — a permanent conflict, checked independently of ownership
        // or lease state.
        let outcome = store
            .reserve(request_with_fingerprint(
                "owner-b",
                "op-10",
                "fp-different",
                lease_until,
            ))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            ReservationOutcome::Conflict,
            "same key with a different fingerprint must be a permanent conflict"
        );
    }

    // --- 6. A matching fingerprint is not a conflict ------------------------
    {
        // Triangulation: prove the fingerprint check does not turn every
        // matching-fingerprint retry into a false conflict — only a mismatch
        // conflicts.
        let (store, _clock) = fresh().await;
        let lease_until = epoch() + Duration::seconds(30);
        store
            .reserve(request_with_fingerprint(
                "owner-a",
                "op-11",
                "fp-same",
                lease_until,
            ))
            .await
            .unwrap();

        let outcome = store
            .reserve(request_with_fingerprint(
                "owner-a",
                "op-11",
                "fp-same",
                lease_until,
            ))
            .await
            .unwrap();
        assert!(
            matches!(outcome, ReservationOutcome::OwnedInProgress(_)),
            "an identical fingerprint must not be treated as a conflict, got {outcome:?}"
        );
    }

    // --- 7. A completed reservation replays its stored response -------------
    {
        // Triangulation: the flip side of "same key, different fingerprint is a
        // conflict" — same key, same fingerprint, already completed, returns the
        // stored outcome instead of re-executing.
        let (store, _clock) = fresh().await;
        let lease_until = epoch() + Duration::seconds(30);
        let fresh_outcome = store
            .reserve(request_with_fingerprint(
                "owner-a",
                "op-12",
                "fp-same",
                lease_until,
            ))
            .await
            .unwrap();
        let fence = match fresh_outcome {
            ReservationOutcome::Fresh(lease) => fence_of(&lease),
            other => panic!("expected Fresh, got {other:?}"),
        };
        store
            .complete(&fence, StoredResponse::new(b"welcome-email-sent".to_vec()))
            .await
            .unwrap();

        let outcome = store
            .reserve(request_with_fingerprint(
                "owner-b",
                "op-12",
                "fp-same",
                lease_until,
            ))
            .await
            .unwrap();
        match outcome {
            ReservationOutcome::Succeeded(response) => {
                assert_eq!(response.as_bytes(), b"welcome-email-sent");
            }
            other => panic!("expected Succeeded, got {other:?}"),
        }
    }
}

/// The eight scenarios that pin the fence-verifying mutators — `renew`,
/// `complete`, `abandon` — against a stale owner, against a lapsed one, and the
/// renewal cadence the port commits to.
///
/// Each case asserts the reservation was left **untouched**, not merely that the
/// call errored: "rejected but mutated" would still corrupt state, and only the
/// follow-up observation distinguishes them.
///
/// # Panics
///
/// Panics on the first divergence, naming the scenario and what it expected.
pub async fn assert_lease_mutation_conformance<S, F, Fut>(fresh: F)
where
    S: OperationReservationStore,
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = (S, Arc<TestClock>)>,
{
    // --- 1. A stale owner cannot complete ----------------------------------
    {
        let (store, clock) = fresh().await;
        let (stale_fence, current_fence) = reserve_then_take_over(&store, &clock, "op-5").await;

        let stale_result = store
            .complete(&stale_fence, StoredResponse::new(b"stale".to_vec()))
            .await;
        assert_eq!(
            stale_result,
            Err(ReservationError::StaleOwner),
            "a stale owner's complete must be rejected"
        );

        // Prove the stale attempt did not modify the reservation: the current
        // (post-takeover) owner can still legitimately complete it.
        let current_result = store
            .complete(&current_fence, StoredResponse::new(b"current".to_vec()))
            .await;
        assert!(
            current_result.is_ok(),
            "the current owner's complete must still succeed after the stale \
             attempt: {current_result:?}"
        );
    }

    // --- 2. A stale owner cannot renew -------------------------------------
    {
        let (store, clock) = fresh().await;
        let (stale_fence, current_fence) = reserve_then_take_over(&store, &clock, "op-6").await;

        let stale_result = store
            .renew(&stale_fence, clock.now() + Duration::seconds(60))
            .await;
        assert_eq!(
            stale_result,
            Err(ReservationError::StaleOwner),
            "a stale owner's renew must be rejected"
        );

        let current_result = store
            .renew(&current_fence, clock.now() + Duration::seconds(60))
            .await;
        assert!(
            current_result.is_ok(),
            "the current owner's renew must still succeed after the stale \
             attempt: {current_result:?}"
        );
    }

    // --- 3. A stale owner cannot abandon -----------------------------------
    {
        let (store, clock) = fresh().await;
        let (stale_fence, current_fence) = reserve_then_take_over(&store, &clock, "op-7").await;

        let stale_result = store.abandon(&stale_fence).await;
        assert_eq!(
            stale_result,
            Err(ReservationError::StaleOwner),
            "a stale owner's abandon must be rejected"
        );

        let current_result = store.abandon(&current_fence).await;
        assert!(
            current_result.is_ok(),
            "the current owner's abandon must still succeed after the stale \
             attempt: {current_result:?}"
        );
    }

    // --- 4. Only an explicit renew extends a lease -------------------------
    //
    // The port's chosen default: lease length is caller/deployment
    // configuration, and the store never renews on its own. A long-running
    // operation either finishes inside its configured lease or is legitimately
    // taken over. These two scenarios pin both halves.
    {
        let (store, clock) = fresh().await;
        let first_lease_until = clock.now() + Duration::seconds(30);
        let fresh_outcome = store
            .reserve(request("owner-a", "op-8", first_lease_until))
            .await
            .unwrap();
        let fence = match fresh_outcome {
            ReservationOutcome::Fresh(lease) => fence_of(&lease),
            other => panic!("expected Fresh, got {other:?}"),
        };

        // The owner explicitly renews before its original 30s lease elapses.
        clock.advance(Duration::seconds(20));
        let extended_until = clock.now() + Duration::seconds(30);
        store.renew(&fence, extended_until).await.unwrap();

        // Now past the ORIGINAL lease_until (20 + 15 = 35 > 30), but still
        // inside the renewed one (35 < 20 + 30 = 50).
        clock.advance(Duration::seconds(15));
        let outcome = store
            .reserve(request(
                "owner-b",
                "op-8",
                clock.now() + Duration::seconds(30),
            ))
            .await
            .unwrap();
        assert_eq!(
            outcome,
            ReservationOutcome::OtherInProgress,
            "an explicit renew must extend the lease past its original deadline"
        );
    }

    // --- 5. Without a renew, the lease expires at exactly its length -------
    {
        let (store, clock) = fresh().await;
        let lease_until = clock.now() + Duration::seconds(30);
        store
            .reserve(request("owner-a", "op-9", lease_until))
            .await
            .unwrap();

        // No renewal ever called — nothing in the store or the clock extends a
        // lease on its own. Advance exactly to the configured deadline.
        clock.advance(Duration::seconds(30));
        let outcome = store
            .reserve(request(
                "owner-b",
                "op-9",
                clock.now() + Duration::seconds(30),
            ))
            .await
            .unwrap();
        assert!(
            matches!(outcome, ReservationOutcome::TakenOver(_)),
            "no background renewal exists: an un-renewed lease is takeover-eligible \
             exactly at its configured length, got {outcome:?}"
        );
    }

    // --- 6, 7, 8. A lapsed holder is not an owner --------------------------
    //
    // Every mutation must reject an expired lease even when its fence triple
    // still matches. The clock is positioned at exactly `lease_until` — the
    // instant `reserve` already treats as expired and takeover-eligible — so
    // testing the boundary pins both decisions to one definition of expiry
    // rather than leaving a one-instant window where a lease is simultaneously
    // seizable and renewable.

    // 6. A lapsed holder must not resurrect a dead lease. If it could, a
    //    takeover that had already become legitimate would silently lose.
    {
        let (store, clock) = fresh().await;
        let lease_until = epoch() + Duration::seconds(30);
        let outcome = store
            .reserve(request("owner-a", "op-renew-expired", lease_until))
            .await
            .unwrap();
        let fence = match outcome {
            ReservationOutcome::Fresh(lease) => fence_of(&lease),
            other => panic!("expected Fresh, got {other:?}"),
        };

        clock.advance(Duration::seconds(30));
        assert_eq!(clock.now(), lease_until, "positioned exactly at expiry");

        let result = store
            .renew(&fence, lease_until + Duration::seconds(60))
            .await;
        assert_eq!(
            Err(ReservationError::StaleOwner),
            result,
            "an expired holder must not renew"
        );

        // Unmodified: a different owner still sees an expired lease it may
        // seize. Had the renewal landed, this would be OtherInProgress.
        let after = store
            .reserve(request(
                "owner-b",
                "op-renew-expired",
                lease_until + Duration::seconds(90),
            ))
            .await
            .unwrap();
        assert!(
            matches!(after, ReservationOutcome::TakenOver(_)),
            "the lease must still be seizable, got {after:?}"
        );
    }

    // 7. A lapsed holder must not record a completion. If it could, a later
    //    replay would serve, as authoritative, a result produced for an
    //    operation the caller no longer owned.
    {
        let (store, clock) = fresh().await;
        let lease_until = epoch() + Duration::seconds(30);
        let outcome = store
            .reserve(request("owner-a", "op-complete-expired", lease_until))
            .await
            .unwrap();
        let fence = match outcome {
            ReservationOutcome::Fresh(lease) => fence_of(&lease),
            other => panic!("expected Fresh, got {other:?}"),
        };

        clock.advance(Duration::seconds(30));

        let result = store
            .complete(&fence, StoredResponse::new(b"late".to_vec()))
            .await;
        assert_eq!(
            Err(ReservationError::StaleOwner),
            result,
            "an expired holder must not complete"
        );

        // Unmodified: no response was stored, so another owner takes over
        // rather than observing a success it never produced.
        let after = store
            .reserve(request(
                "owner-b",
                "op-complete-expired",
                lease_until + Duration::seconds(90),
            ))
            .await
            .unwrap();
        assert!(
            matches!(after, ReservationOutcome::TakenOver(_)),
            "no completion must have been recorded, got {after:?}"
        );
    }

    // 8. A lapsed holder must not release the key. If it could, it would
    //    discard a reservation another caller was entitled to seize, and the
    //    next arrival would look like a first attempt.
    {
        let (store, clock) = fresh().await;
        let lease_until = epoch() + Duration::seconds(30);
        let outcome = store
            .reserve(request("owner-a", "op-abandon-expired", lease_until))
            .await
            .unwrap();
        let fence = match outcome {
            ReservationOutcome::Fresh(lease) => fence_of(&lease),
            other => panic!("expected Fresh, got {other:?}"),
        };

        clock.advance(Duration::seconds(30));

        let result = store.abandon(&fence).await;
        assert_eq!(
            Err(ReservationError::StaleOwner),
            result,
            "an expired holder must not abandon"
        );

        // Unmodified: the reservation still exists, so the next owner takes it
        // over. Had the abandon landed, this would be Fresh.
        let after = store
            .reserve(request(
                "owner-b",
                "op-abandon-expired",
                lease_until + Duration::seconds(90),
            ))
            .await
            .unwrap();
        assert!(
            matches!(after, ReservationOutcome::TakenOver(_)),
            "the reservation must survive, got {after:?}"
        );
    }
}

/// The seven scenarios that pin [`OperationReservationStore::purge_completed_before`].
///
/// Observation goes through the port, never through an implementation's storage: a
/// purged reservation is one that `reserve` now reports as `Fresh`, and a surviving
/// completed one still replays its `Succeeded` response. That keeps the scenarios
/// meaningful against a durable store, which has no in-process map to inspect.
///
/// # Why no scenario names which rows a batch chose
///
/// Selection within a batch is not part of the contract. Asserting identities here
/// would turn an implementation detail into a promise callers could build on, and
/// would fail a durable store that orders its claim query for its own operational
/// reasons. What is asserted instead: the count, that ineligible rows survive, and
/// that successive calls drain the eligible set.
///
/// # Panics
///
/// Panics on the first divergence, naming the scenario and what it expected.
pub async fn assert_purge_conformance<S, F, Fut>(fresh: F)
where
    S: OperationReservationStore,
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = (S, Arc<TestClock>)>,
{
    /// Completes a reservation at exactly `at`, so `completed_at` is deterministic.
    ///
    /// The clock is positioned first because `complete` stamps `completed_at` from
    /// it; the lease is opened generously so the completion is never rejected for
    /// having lapsed, which would make the scenario assert the wrong thing.
    async fn completed_at<S: OperationReservationStore>(
        store: &S,
        clock: &TestClock,
        key: &str,
        at: DateTime<Utc>,
        response: &[u8],
    ) {
        position(clock, at);
        let outcome = store
            .reserve(request("owner-a", key, at + Duration::seconds(300)))
            .await
            .unwrap();
        let fence = match outcome {
            ReservationOutcome::Fresh(lease) => fence_of(&lease),
            other => panic!("setup: expected Fresh for {key}, got {other:?}"),
        };
        store
            .complete(&fence, StoredResponse::new(response.to_vec()))
            .await
            .unwrap();
    }

    /// Whether a completed reservation is still there, asked through the port: a
    /// survivor replays its response, a purged one reads as a fresh key.
    async fn survives<S: OperationReservationStore>(
        store: &S,
        clock: &TestClock,
        key: &str,
    ) -> bool {
        let outcome = store
            .reserve(request("probe", key, clock.now() + Duration::seconds(300)))
            .await
            .unwrap();
        match outcome {
            ReservationOutcome::Succeeded(_) => true,
            ReservationOutcome::Fresh(_) => false,
            other => panic!("probing {key} must observe Succeeded or Fresh, got {other:?}"),
        }
    }

    let t0 = epoch();
    let cutoff = t0 + Duration::seconds(100);

    // --- 1, 2. Eligibility is strictly before the cutoff --------------------
    {
        let (store, clock) = fresh().await;
        completed_at(
            &store,
            &clock,
            "op-before",
            t0 + Duration::seconds(50),
            b"old",
        )
        .await;
        completed_at(&store, &clock, "op-at", cutoff, b"boundary").await;
        completed_at(
            &store,
            &clock,
            "op-after",
            cutoff + Duration::seconds(50),
            b"new",
        )
        .await;

        let purged = store.purge_completed_before(cutoff, 10).await.unwrap();
        assert_eq!(
            purged, 1,
            "only the reservation completed strictly before the cutoff is eligible"
        );

        position(&clock, cutoff + Duration::seconds(500));
        assert!(
            !survives(&store, &clock, "op-before").await,
            "a reservation completed before the cutoff must be purged"
        );
        assert!(
            survives(&store, &clock, "op-at").await,
            "a reservation completed at exactly the cutoff must survive: eligibility is \
             strictly earlier, not earlier-or-equal"
        );
        assert!(
            survives(&store, &clock, "op-after").await,
            "a reservation completed after the cutoff must survive"
        );
    }

    // --- 3. An InProgress reservation is never purged, however old ----------
    {
        let (store, clock) = fresh().await;
        position(&clock, t0);
        store
            .reserve(request("owner-a", "op-live", t0 + Duration::seconds(30)))
            .await
            .unwrap();

        // Far past both its lease and the cutoff: age is irrelevant here. Only
        // lease expiry and takeover resolve an InProgress reservation.
        position(&clock, cutoff + Duration::seconds(10_000));
        let purged = store.purge_completed_before(clock.now(), 10).await.unwrap();
        assert_eq!(purged, 0, "an InProgress reservation is never purged");

        // It survives as a reservation, not as a completed record: a second owner
        // takes it over, which it could not if the row were gone.
        let after = store
            .reserve(request(
                "owner-b",
                "op-live",
                clock.now() + Duration::seconds(30),
            ))
            .await
            .unwrap();
        assert!(
            matches!(after, ReservationOutcome::TakenOver(_)),
            "the reservation must still be there to take over, got {after:?}"
        );
    }

    // --- 4, 5. At most `batch`, and the count is exactly what went ----------
    {
        let (store, clock) = fresh().await;
        for i in 0..5 {
            completed_at(
                &store,
                &clock,
                &format!("op-batch-{i}"),
                t0 + Duration::seconds(i),
                b"x",
            )
            .await;
        }

        let purged = store.purge_completed_before(cutoff, 2).await.unwrap();
        assert_eq!(purged, 2, "a call removes at most `batch` rows");

        // The count is the number actually removed, not the number requested:
        // three remain eligible, so a batch of 10 removes three.
        let rest = store.purge_completed_before(cutoff, 10).await.unwrap();
        assert_eq!(
            rest, 3,
            "the return value is the number of rows removed, not the batch size"
        );

        let none_left = store.purge_completed_before(cutoff, 10).await.unwrap();
        assert_eq!(none_left, 0, "nothing eligible remains");
    }

    // --- 6. A batch of zero removes nothing --------------------------------
    {
        let (store, clock) = fresh().await;
        completed_at(&store, &clock, "op-zero", t0, b"x").await;

        let purged = store.purge_completed_before(cutoff, 0).await.unwrap();
        assert_eq!(purged, 0, "a batch of zero removes nothing");

        position(&clock, cutoff + Duration::seconds(500));
        assert!(
            survives(&store, &clock, "op-zero").await,
            "a batch of zero must leave the eligible row in place"
        );
    }

    // --- 7. More eligible than the batch: count, preservation, drainage -----
    {
        let (store, clock) = fresh().await;
        for i in 0..4 {
            completed_at(
                &store,
                &clock,
                &format!("op-drain-{i}"),
                t0 + Duration::seconds(i),
                b"x",
            )
            .await;
        }
        // One ineligible row, completed after the cutoff, to prove a batch never
        // reaches past eligibility no matter how it chooses within it.
        completed_at(
            &store,
            &clock,
            "op-drain-ineligible",
            cutoff + Duration::seconds(10),
            b"keep",
        )
        .await;

        // Successive calls drain the eligible set. No scenario says which rows any
        // individual call took — only that every call removes at most `batch`, and
        // that the set is empty once the calls stop removing anything.
        let mut removed_total = 0u64;
        let mut calls = 0;
        loop {
            let removed = store.purge_completed_before(cutoff, 2).await.unwrap();
            assert!(
                removed <= 2,
                "no call may exceed its batch, removed {removed}"
            );
            if removed == 0 {
                break;
            }
            removed_total += removed;
            calls += 1;
            assert!(
                calls <= 4,
                "draining four eligible rows must not need more than four calls"
            );
        }
        assert_eq!(
            removed_total, 4,
            "successive calls must drain exactly the eligible set"
        );

        position(&clock, cutoff + Duration::seconds(500));
        assert!(
            survives(&store, &clock, "op-drain-ineligible").await,
            "draining the eligible set must never remove an ineligible row"
        );
    }
}

/// The whole shared contract: every scenario, in one call.
///
/// This function **composes** the groups and states nothing of its own. An
/// implementation that satisfies this satisfies everything the port specifies —
/// all five methods, including purge, within the bounds the purge group documents.
///
/// # Panics
///
/// Panics on the first divergence, naming the scenario and what it expected.
pub async fn assert_reservation_store_conformance<S, F, Fut>(fresh: F)
where
    S: OperationReservationStore,
    F: Fn() -> Fut + Copy,
    Fut: std::future::Future<Output = (S, Arc<TestClock>)>,
{
    assert_reserve_conformance(fresh).await;
    assert_lease_mutation_conformance(fresh).await;
    assert_purge_conformance(fresh).await;
}
