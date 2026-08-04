//! `OperationReservationStore` test double, and the deterministic `TestClock`
//! it is driven by.
//!
//! Per the `testkit` delta spec's "Reservation-Store Test Double"
//! requirement, [`InMemoryOperationReservationStore`] satisfies the
//! identical [`ego_domain::operation::OperationReservationStore`] port
//! `service-sdk` registers in production — a test wires the double in and
//! exercises the real port contract, never a look-alike stand-in that can
//! silently drift from production.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ego_domain::operation::{
    FencingToken, Lease, OperationId, OperationReservationStore, OwnerFence, OwnerId,
    ReservationError, ReservationOutcome, ReserveRequest, StoredResponse,
};
use ego_domain::Clock;

/// A deterministic, manually-advanced [`Clock`] double for reservation tests.
///
/// Every call to [`TestClock::now`] returns exactly the instant last set —
/// there is no reliance on real elapsed time, so lease-expiry and takeover
/// scenarios are reproducible regardless of how fast the test machine runs.
pub struct TestClock {
    now: Mutex<DateTime<Utc>>,
}

impl TestClock {
    /// Creates a clock fixed at `start`.
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    /// Advances the clock forward by `duration`.
    pub fn advance(&self, duration: Duration) {
        let mut now = self.now.lock().expect("TestClock mutex poisoned");
        *now += duration;
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("TestClock mutex poisoned")
    }
}

/// One reservation's persisted state.
#[derive(Debug, Clone)]
enum RecordState {
    /// A lease is currently held (or was, and may now be expired).
    InProgress {
        owner_id: OwnerId,
        fencing_token: FencingToken,
        lease_until: DateTime<Utc>,
    },
    /// The operation completed; `response` is available for replay.
    Completed {
        response: StoredResponse,
        completed_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone)]
struct Record {
    fingerprint: ego_domain::operation::OperationFingerprint,
    state: RecordState,
}

/// The `testkit` double for [`OperationReservationStore`].
///
/// Same-contract principle: this is a real, full implementation of the real
/// production port, not a parallel model of it — every scenario a test
/// configures here is a real state transition the trait actually specifies.
pub struct InMemoryOperationReservationStore {
    clock: Arc<dyn Clock>,
    records: Mutex<HashMap<OperationId, Record>>,
}

impl InMemoryOperationReservationStore {
    /// Creates an empty store driven by `clock`.
    ///
    /// Production code drives an equivalent store with `SystemClock`; tests
    /// drive it with [`TestClock`] for deterministic lease-expiry and
    /// takeover scenarios (testkit spec: "Test configures a deterministic
    /// lease expiry").
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            records: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl OperationReservationStore for InMemoryOperationReservationStore {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        let operation_id = OperationId::new(req.tenant.clone(), req.operation_key.clone());
        let mut records = self
            .records
            .lock()
            .expect("reservation store mutex poisoned");

        let existing = records.get(&operation_id).cloned();
        match existing {
            None => {
                let lease = Lease {
                    operation_id: operation_id.clone(),
                    owner_id: req.owner_id.clone(),
                    fencing_token: FencingToken::initial(),
                    lease_until: req.lease_until,
                };
                records.insert(
                    operation_id,
                    Record {
                        fingerprint: req.fingerprint,
                        state: RecordState::InProgress {
                            owner_id: req.owner_id,
                            fencing_token: FencingToken::initial(),
                            lease_until: req.lease_until,
                        },
                    },
                );
                Ok(ReservationOutcome::Fresh(lease))
            }
            // A different fingerprint under the same key is a permanent
            // conflict — checked before anything else, so it is never
            // silently reinterpreted as a takeover, a replay, or a plain
            // in-progress collision (spec: "Same key, different fingerprint
            // is a permanent conflict"; the rule holds regardless of lease
            // or completion state).
            Some(record) if record.fingerprint != req.fingerprint => {
                Ok(ReservationOutcome::Conflict)
            }
            Some(record) => match record.state {
                RecordState::InProgress {
                    owner_id,
                    fencing_token,
                    lease_until,
                } => {
                    if self.clock.now() >= lease_until {
                        // The lease expired — atomically take it over with a
                        // strictly greater fencing token, fencing out the
                        // prior owner (spec: "Atomic takeover fences out the
                        // prior owner").
                        // Checked advance: exhaustion is surfaced rather than
                        // wrapped, because a wrapped token could compare equal
                        // to a fence the displaced owner still holds.
                        let new_token = fencing_token
                            .next()
                            .ok_or(ReservationError::FencingExhausted)?;
                        let lease = Lease {
                            operation_id: operation_id.clone(),
                            owner_id: req.owner_id.clone(),
                            fencing_token: new_token,
                            lease_until: req.lease_until,
                        };
                        records.insert(
                            operation_id,
                            Record {
                                fingerprint: record.fingerprint,
                                state: RecordState::InProgress {
                                    owner_id: req.owner_id,
                                    fencing_token: new_token,
                                    lease_until: req.lease_until,
                                },
                            },
                        );
                        Ok(ReservationOutcome::TakenOver(lease))
                    } else if owner_id == req.owner_id {
                        Ok(ReservationOutcome::OwnedInProgress(Lease {
                            operation_id,
                            owner_id,
                            fencing_token,
                            lease_until,
                        }))
                    } else {
                        Ok(ReservationOutcome::OtherInProgress)
                    }
                }
                RecordState::Completed { response, .. } => {
                    Ok(ReservationOutcome::Succeeded(response))
                }
            },
        }
    }

    async fn renew(
        &self,
        fence: &OwnerFence,
        until: DateTime<Utc>,
    ) -> Result<(), ReservationError> {
        let mut records = self
            .records
            .lock()
            .expect("reservation store mutex poisoned");
        // The clock is read *inside* the critical section, deliberately. Reading
        // it before acquiring the lock would let a caller that waited on the
        // mutex evaluate validity against an instant from before the wait, so a
        // lease that expired during the wait would still look live and the
        // mutation would land on a reservation another caller is already
        // entitled to seize. Validation and mutation have to be linearised
        // together, which is also how `reserve` reads it.
        let now = self.clock.now();
        match records.get_mut(&fence.operation_id) {
            Some(record) => match &mut record.state {
                // The full triple — operation id (the map key), owner id,
                // and fencing token — must all match the reservation's
                // current holder. Comparing only the fencing token would
                // satisfy "stores a token" but not "verifies it" (spec:
                // "Storing a token without verifying it is insufficient").
                RecordState::InProgress {
                    owner_id,
                    fencing_token,
                    lease_until,
                } if *owner_id == fence.owner_id
                    && *fencing_token == fence.fencing_token
                    && now < *lease_until =>
                {
                    *lease_until = until;
                    Ok(())
                }
                _ => Err(ReservationError::StaleOwner),
            },
            None => Err(ReservationError::StaleOwner),
        }
    }

    async fn complete(
        &self,
        fence: &OwnerFence,
        response: StoredResponse,
    ) -> Result<(), ReservationError> {
        let mut records = self
            .records
            .lock()
            .expect("reservation store mutex poisoned");
        // Read inside the critical section — see the note on `renew`.
        let now = self.clock.now();
        match records.get(&fence.operation_id) {
            Some(record) => match &record.state {
                RecordState::InProgress {
                    owner_id,
                    fencing_token,
                    lease_until,
                } if *owner_id == fence.owner_id
                    && *fencing_token == fence.fencing_token
                    && now < *lease_until =>
                {
                    let fingerprint = record.fingerprint.clone();
                    records.insert(
                        fence.operation_id.clone(),
                        Record {
                            fingerprint,
                            state: RecordState::Completed {
                                response,
                                completed_at: now,
                            },
                        },
                    );
                    Ok(())
                }
                _ => Err(ReservationError::StaleOwner),
            },
            None => Err(ReservationError::StaleOwner),
        }
    }

    async fn abandon(&self, fence: &OwnerFence) -> Result<(), ReservationError> {
        let mut records = self
            .records
            .lock()
            .expect("reservation store mutex poisoned");
        // Read inside the critical section — see the note on `renew`.
        let now = self.clock.now();
        match records.get(&fence.operation_id) {
            Some(record) => match &record.state {
                RecordState::InProgress {
                    owner_id,
                    fencing_token,
                    lease_until,
                } if *owner_id == fence.owner_id
                    && *fencing_token == fence.fencing_token
                    && now < *lease_until =>
                {
                    records.remove(&fence.operation_id);
                    Ok(())
                }
                _ => Err(ReservationError::StaleOwner),
            },
            None => Err(ReservationError::StaleOwner),
        }
    }

    async fn purge_completed_before(
        &self,
        cutoff: DateTime<Utc>,
        batch: usize,
    ) -> Result<u64, ReservationError> {
        // Retention/purge semantics (batching, concurrency safety, worker
        // ownership) are a later slice's job — this minimal implementation
        // only upholds the one invariant this port's contract already fixes:
        // an `InProgress` reservation is never purged, regardless of age
        // (spec: "InProgress reservations are never TTL-purged").
        let mut records = self
            .records
            .lock()
            .expect("reservation store mutex poisoned");
        let eligible: Vec<OperationId> = records
            .iter()
            .filter_map(|(id, record)| match &record.state {
                RecordState::Completed { completed_at, .. } if *completed_at < cutoff => {
                    Some(id.clone())
                }
                _ => None,
            })
            .take(batch)
            .collect();

        for id in &eligible {
            records.remove(id);
        }
        Ok(eligible.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{Duration, TimeZone, Utc};
    use ego_domain::operation::{
        OperationFingerprint, OperationKey, OperationReservationStore, OwnerFence, OwnerId,
        ReservationError, ReservationOutcome, ReserveRequest, StoredResponse,
    };
    use ego_domain::Clock;

    use super::{InMemoryOperationReservationStore, TestClock};

    fn epoch() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
    }

    fn request_with_fingerprint(
        owner: &str,
        key: &str,
        fingerprint: &str,
        lease_until: chrono::DateTime<Utc>,
    ) -> ReserveRequest {
        ReserveRequest {
            tenant: None,
            operation_key: OperationKey::parse(key).unwrap(),
            fingerprint: OperationFingerprint::new(fingerprint),
            owner_id: OwnerId::new(owner),
            lease_until,
        }
    }

    fn request(owner: &str, key: &str, lease_until: chrono::DateTime<Utc>) -> ReserveRequest {
        request_with_fingerprint(owner, key, "fp-1", lease_until)
    }

    #[tokio::test]
    async fn first_reservation_for_a_key_is_fresh() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());

        let outcome = store
            .reserve(request("owner-a", "op-1", epoch() + Duration::seconds(30)))
            .await
            .unwrap();

        assert!(
            matches!(outcome, ReservationOutcome::Fresh(_)),
            "expected Fresh, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn same_owner_mid_lease_observes_owned_in_progress() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
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
            "expected OwnedInProgress, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn different_owner_mid_lease_observes_other_in_progress() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
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
    async fn expired_lease_is_atomically_taken_over_with_a_strictly_greater_fencing_token() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        let first_lease_until = epoch() + Duration::seconds(30);

        let fresh = store
            .reserve(request("owner-a", "op-4", first_lease_until))
            .await
            .unwrap();
        let original_fence = match fresh {
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

    /// Reserves a key, advances the clock past its lease, and lets a second
    /// owner take it over. Returns the stale (original) and current
    /// (post-takeover) `OwnerFence`s for a stale-rejection assertion.
    async fn reserve_then_take_over(
        store: &InMemoryOperationReservationStore,
        clock: &TestClock,
        key: &str,
    ) -> (OwnerFence, OwnerFence) {
        let first_lease_until = clock.now() + Duration::seconds(30);
        let fresh = store
            .reserve(request("owner-a", key, first_lease_until))
            .await
            .unwrap();
        let stale_fence = match fresh {
            ReservationOutcome::Fresh(lease) => OwnerFence {
                operation_id: lease.operation_id,
                owner_id: lease.owner_id,
                fencing_token: lease.fencing_token,
            },
            other => panic!("expected Fresh, got {other:?}"),
        };

        clock.advance(Duration::seconds(31));
        let second_lease_until = clock.now() + Duration::seconds(30);
        let taken_over = store
            .reserve(request("owner-b", key, second_lease_until))
            .await
            .unwrap();
        let current_fence = match taken_over {
            ReservationOutcome::TakenOver(lease) => OwnerFence {
                operation_id: lease.operation_id,
                owner_id: lease.owner_id,
                fencing_token: lease.fencing_token,
            },
            other => panic!("expected TakenOver, got {other:?}"),
        };

        (stale_fence, current_fence)
    }

    #[tokio::test]
    async fn stale_owner_complete_is_rejected_and_does_not_modify_the_reservation() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        let (stale_fence, current_fence) = reserve_then_take_over(&store, &clock, "op-5").await;

        let stale_result = store
            .complete(&stale_fence, StoredResponse::new(b"stale".to_vec()))
            .await;
        assert_eq!(stale_result, Err(ReservationError::StaleOwner));

        // Prove the stale attempt did not modify the reservation: the
        // current (post-takeover) owner can still legitimately complete it.
        let current_result = store
            .complete(&current_fence, StoredResponse::new(b"current".to_vec()))
            .await;
        assert!(
            current_result.is_ok(),
            "current owner's complete must still succeed after the stale attempt: {current_result:?}"
        );
    }

    #[tokio::test]
    async fn stale_owner_renew_is_rejected_and_does_not_modify_the_reservation() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        let (stale_fence, current_fence) = reserve_then_take_over(&store, &clock, "op-6").await;

        let stale_result = store
            .renew(&stale_fence, clock.now() + Duration::seconds(60))
            .await;
        assert_eq!(stale_result, Err(ReservationError::StaleOwner));

        let current_result = store
            .renew(&current_fence, clock.now() + Duration::seconds(60))
            .await;
        assert!(
            current_result.is_ok(),
            "current owner's renew must still succeed after the stale attempt: {current_result:?}"
        );
    }

    #[tokio::test]
    async fn stale_owner_abandon_is_rejected_and_does_not_modify_the_reservation() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        let (stale_fence, current_fence) = reserve_then_take_over(&store, &clock, "op-7").await;

        let stale_result = store.abandon(&stale_fence).await;
        assert_eq!(stale_result, Err(ReservationError::StaleOwner));

        let current_result = store.abandon(&current_fence).await;
        assert!(
            current_result.is_ok(),
            "current owner's abandon must still succeed after the stale attempt: {current_result:?}"
        );
    }

    // ---- Lease renewal cadence and owner (open-question B2.7) --------------
    //
    // Chosen default (documented on `OperationReservationStore::renew` and
    // `OwnerFence`'s module docs): lease length is caller/deployment
    // configuration, and the store never renews a lease on its own. A
    // long-running operation either finishes inside its configured lease or
    // is legitimately taken over. `renew` exists purely as a caller-invoked
    // capability — no runtime component in this change calls it
    // automatically. These two tests pin both halves of that contract.

    #[tokio::test]
    async fn an_explicit_renew_call_is_the_only_thing_that_extends_a_lease() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        let first_lease_until = clock.now() + Duration::seconds(30);

        let fresh = store
            .reserve(request("owner-a", "op-8", first_lease_until))
            .await
            .unwrap();
        let fence = match fresh {
            ReservationOutcome::Fresh(lease) => OwnerFence {
                operation_id: lease.operation_id,
                owner_id: lease.owner_id,
                fencing_token: lease.fencing_token,
            },
            other => panic!("expected Fresh, got {other:?}"),
        };

        // The owner explicitly renews before its original 30s lease elapses.
        clock.advance(Duration::seconds(20));
        let extended_until = clock.now() + Duration::seconds(30);
        store.renew(&fence, extended_until).await.unwrap();

        // Now past the ORIGINAL lease_until (20s + 15s = 35s > 30s), but
        // still inside the renewed one (20s + 15s = 35s < 20s + 30s = 50s).
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

    #[tokio::test]
    async fn without_any_renew_call_the_lease_expires_at_exactly_its_configured_length() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        let lease_until = clock.now() + Duration::seconds(30);
        store
            .reserve(request("owner-a", "op-9", lease_until))
            .await
            .unwrap();

        // No renewal ever called — nothing in this store or clock extends
        // the lease on its own. Advance exactly to the configured deadline.
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
            "no background renewal exists in this change: an un-renewed lease \
             is takeover-eligible exactly at its configured length, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn same_key_different_fingerprint_is_a_permanent_conflict_not_a_silent_dedupe() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
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
        // operation — this is a permanent conflict, checked independently of
        // ownership or lease state.
        let outcome = store
            .reserve(request_with_fingerprint(
                "owner-b",
                "op-10",
                "fp-different",
                lease_until,
            ))
            .await
            .unwrap();

        assert_eq!(outcome, ReservationOutcome::Conflict);
    }

    #[tokio::test]
    async fn same_key_same_fingerprint_from_the_same_owner_still_observes_owned_in_progress() {
        // Triangulation: prove the fingerprint check does not turn every
        // matching-fingerprint retry into a false conflict — only a mismatch
        // conflicts.
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
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
            "identical fingerprint must not be treated as a conflict, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_completed_reservation_replayed_with_the_same_fingerprint_returns_the_stored_response(
    ) {
        // Triangulation: the flip side of "same key, different fingerprint is
        // a conflict" — same key, same fingerprint, already completed,
        // returns the stored outcome instead of re-executing (spec: "Same
        // key, same fingerprint replays").
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        let lease_until = epoch() + Duration::seconds(30);

        let fresh = store
            .reserve(request_with_fingerprint(
                "owner-a",
                "op-12",
                "fp-same",
                lease_until,
            ))
            .await
            .unwrap();
        let fence = match fresh {
            ReservationOutcome::Fresh(lease) => OwnerFence {
                operation_id: lease.operation_id,
                owner_id: lease.owner_id,
                fencing_token: lease.fencing_token,
            },
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

    // --- A lapsed holder is not an owner: every mutation must reject an
    //     expired lease, even when its fence triple still matches. Each case
    //     asserts the reservation is left untouched, not merely that the call
    //     errored, because "rejected but mutated" would still corrupt state.
    //
    //     The clock is positioned at exactly `lease_until`. That is the instant
    //     `reserve` already treats as expired and eligible for takeover, so
    //     testing the boundary itself pins the two decisions to one definition
    //     of expiry rather than leaving a one-instant window where a lease is
    //     simultaneously seizable and renewable.

    /// A lapsed holder must not resurrect a dead lease. If it could, a takeover
    /// that had already become legitimate would silently lose the race.
    #[tokio::test]
    async fn renew_at_the_exact_expiry_instant_is_rejected_and_leaves_the_lease_expired() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        let lease_until = epoch() + Duration::seconds(30);

        let outcome = store
            .reserve(request("owner-a", "op-renew-expired", lease_until))
            .await
            .unwrap();
        let lease = match outcome {
            ReservationOutcome::Fresh(lease) => lease,
            other => panic!("expected Fresh, got {other:?}"),
        };
        let fence = OwnerFence {
            operation_id: lease.operation_id.clone(),
            owner_id: lease.owner_id.clone(),
            fencing_token: lease.fencing_token,
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

    /// A lapsed holder must not record a completion. If it could, a later replay
    /// would serve, as authoritative, a result produced for an operation the
    /// caller no longer owned.
    #[tokio::test]
    async fn complete_at_the_exact_expiry_instant_is_rejected_and_does_not_store_a_response() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        let lease_until = epoch() + Duration::seconds(30);

        let outcome = store
            .reserve(request("owner-a", "op-complete-expired", lease_until))
            .await
            .unwrap();
        let lease = match outcome {
            ReservationOutcome::Fresh(lease) => lease,
            other => panic!("expected Fresh, got {other:?}"),
        };
        let fence = OwnerFence {
            operation_id: lease.operation_id.clone(),
            owner_id: lease.owner_id.clone(),
            fencing_token: lease.fencing_token,
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

    /// A lapsed holder must not release the key. If it could, it would discard a
    /// reservation another caller was entitled to seize, and the next arrival
    /// would look like a first attempt.
    #[tokio::test]
    async fn abandon_at_the_exact_expiry_instant_is_rejected_and_keeps_the_reservation() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        let lease_until = epoch() + Duration::seconds(30);

        let outcome = store
            .reserve(request("owner-a", "op-abandon-expired", lease_until))
            .await
            .unwrap();
        let lease = match outcome {
            ReservationOutcome::Fresh(lease) => lease,
            other => panic!("expected Fresh, got {other:?}"),
        };
        let fence = OwnerFence {
            operation_id: lease.operation_id.clone(),
            owner_id: lease.owner_id.clone(),
            fencing_token: lease.fencing_token,
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

    /// A mutation that waits on the store's lock must evaluate the lease against
    /// an instant read *after* it acquires that lock, not before.
    ///
    /// The failure this guards against: read the clock while the lease is still
    /// live, block on the mutex, and the lease lapses during the wait. The
    /// caller then enters the critical section carrying a stale instant, sees a
    /// lease that no longer exists as valid, and mutates a reservation another
    /// caller is already entitled to seize. Validation and mutation have to be
    /// linearised together.
    ///
    /// Construction: the test itself holds the store's lock, starts the
    /// mutation, advances the clock to exactly the lease bound while still
    /// holding it, and only then releases. Because the advance happens strictly
    /// before the release, any clock read taken inside the critical section
    /// necessarily observes the expired lease.
    ///
    /// Honest limitation, stated rather than implied: as a *regression detector*
    /// for the read-before-lock ordering this is not airtight. If the spawned
    /// task has not yet reached its clock read when the advance happens, a
    /// read-before-lock implementation would also observe the expired instant
    /// and reject, so the test would pass against the defect. Guaranteeing
    /// otherwise needs either a sleep or a synchronisation seam inside the
    /// store, and this suite forbids the first and does not warrant the second.
    /// What the test does establish deterministically is the property itself:
    /// a lock wait spanning expiry rejects and leaves the reservation intact.
    /// The ordering is additionally held by the comments at each call site.
    ///
    /// The multi-threaded flavour is required: the spawned task blocks on a
    /// synchronous mutex, which would deadlock a current-thread runtime.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_lock_wait_that_spans_expiry_rejects_the_lapsed_holder() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = Arc::new(InMemoryOperationReservationStore::new(clock.clone()));
        let lease_until = epoch() + Duration::seconds(30);

        let outcome = store
            .reserve(request("owner-a", "op-lock-wait", lease_until))
            .await
            .unwrap();
        let lease = match outcome {
            ReservationOutcome::Fresh(lease) => lease,
            other => panic!("expected Fresh, got {other:?}"),
        };
        let fence = OwnerFence {
            operation_id: lease.operation_id.clone(),
            owner_id: lease.owner_id.clone(),
            fencing_token: lease.fencing_token,
        };

        // Hold the critical section the mutation needs.
        let guard = store
            .records
            .lock()
            .expect("reservation store mutex poisoned");

        let renewing = {
            let store = Arc::clone(&store);
            let fence = fence.clone();
            let until = lease_until + Duration::seconds(60);
            tokio::spawn(async move { store.renew(&fence, until).await })
        };

        // The lease lapses while the mutation is waiting for the lock.
        clock.advance(Duration::seconds(30));
        assert_eq!(clock.now(), lease_until, "positioned exactly at expiry");

        drop(guard);

        let result = renewing.await.expect("the renew task must not panic");
        assert_eq!(
            Err(ReservationError::StaleOwner),
            result,
            "a lease that lapsed during the lock wait must not be renewable"
        );

        // Intact: another owner still takes it over, which it could not if the
        // renewal had landed.
        let after = store
            .reserve(request(
                "owner-b",
                "op-lock-wait",
                lease_until + Duration::seconds(90),
            ))
            .await
            .unwrap();
        assert!(
            matches!(after, ReservationOutcome::TakenOver(_)),
            "the reservation must still be seizable, got {after:?}"
        );
    }
}
