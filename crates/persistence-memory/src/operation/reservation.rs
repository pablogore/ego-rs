use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ego_persistence_api::operation::reservation::{
    FencingToken, Lease, OldestCompleted, OperationId, OperationReservationStore, OwnerFence,
    OwnerId, ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse,
};
use ego_domain::Clock;

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
        response: StoredServiceResponse,
        completed_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone)]
struct Record {
    fingerprint: ego_persistence_api::operation::key::OperationFingerprint,
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
        response: StoredServiceResponse,
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

    /// A real implementation, not the `Unsupported` default.
    ///
    /// This double is what the shared conformance harness and the retention
    /// worker's tests run against, so inheriting the default would make every
    /// test of the gauge vacuous — the worker would correctly emit nothing, and
    /// the test would correctly observe nothing, while proving neither.
    ///
    /// `Empty` when no completed reservation is held: a genuine answer about the
    /// backlog, distinct from this store being unable to give one.
    async fn oldest_completed(&self) -> Result<OldestCompleted, ReservationError> {
        let records = self
            .records
            .lock()
            .expect("reservation store mutex poisoned");
        let oldest = records
            .values()
            .filter_map(|record| match &record.state {
                RecordState::Completed { completed_at, .. } => Some(*completed_at),
                _ => None,
            })
            .min();
        Ok(match oldest {
            Some(at) => OldestCompleted::At(at),
            None => OldestCompleted::Empty,
        })
    }

    async fn probe(&self) -> Result<(), ReservationError> {
        // A map in this process cannot become unreachable: if the caller is
        // running, so is the store. There is nothing to check and nothing that
        // could fail, so this is `Ok(())` rather than a lock acquisition —
        // taking the mutex would only add a way for a poisoned lock to make a
        // health probe panic.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::{DateTime, Duration, TimeZone, Utc};
    use ego_domain::Clock;
    use ego_persistence_api::operation::key::{OperationFingerprint, OperationKey};
    use ego_persistence_api::operation::reservation::{
        OperationReservationStore, OwnerFence, OwnerId, ReservationError, ReservationOutcome,
        ReserveRequest,
    };

    use super::InMemoryOperationReservationStore;

    /// Minimal deterministic clock double, colocated with the store it drives.
    ///
    /// `ego-persistence-memory` is a `foundation`-layer crate and cannot depend
    /// on `ego-testkit` (`tooling`, a dependency sink per `layers.toml`) to
    /// reuse its `TestClock` — the layer direction runs the other way. The
    /// scenario below needs direct access to `records`, which is only legal
    /// from inside this crate now that the store lives here rather than in
    /// `ego-testkit` — so the white-box test moves with the struct it
    /// inspects, and `TestClock` stays behind in `ego-testkit` for the
    /// black-box conformance tests that only need the `Clock` port.
    struct FixedClock(Mutex<DateTime<Utc>>);

    impl FixedClock {
        fn new(start: DateTime<Utc>) -> Self {
            Self(Mutex::new(start))
        }

        fn advance(&self, duration: Duration) {
            let mut now = self.0.lock().expect("clock mutex poisoned");
            *now += duration;
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            *self.0.lock().expect("clock mutex poisoned")
        }
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
        let clock = Arc::new(FixedClock::new(epoch()));
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
