//! `OperationReservationStore` test double, and the deterministic `TestClock`
//! it is driven by.
//!
//! Per the `testkit` delta spec's "Reservation-Store Test Double"
//! requirement, [`InMemoryOperationReservationStore`] satisfies the
//! identical [`ego_domain::operation::OperationReservationStore`] port
//! `service-sdk` registers in production — a test wires the double in and
//! exercises the real port contract, never a look-alike stand-in that can
//! silently drift from production.

use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use ego_domain::Clock;

pub use ego_persistence_memory::operation::reservation::InMemoryOperationReservationStore;

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::{TimeZone, Utc};

    use super::{InMemoryOperationReservationStore, TestClock};
    use crate::assert_reservation_store_conformance;

    fn epoch() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
    }

    /// This store satisfies the shared `OperationReservationStore` contract.
    ///
    /// The scenarios used to live here, one test function each, where they tested
    /// *this store* rather than *the port*. They now live in
    /// `reservation_conformance`, so a durable implementation is judged against
    /// the same definitions instead of a copy of them — which is what kept the
    /// two `EventStore` implementations from drifting after four divergences were
    /// found exactly that way.
    #[tokio::test]
    async fn the_in_memory_reservation_store_conforms() {
        assert_reservation_store_conformance(|| async {
            let clock = Arc::new(TestClock::new(epoch()));
            let store = InMemoryOperationReservationStore::new(clock.clone());
            (store, clock)
        })
        .await;
    }

    // `a_lock_wait_that_spans_expiry_rejects_the_lapsed_holder` moved to
    // `ego_persistence_memory::operation::reservation::tests` alongside
    // `InMemoryOperationReservationStore` (CORE-PERSIST-B S2, design.md AD-8
    // amendment): it locks the store's private `records` field directly, which
    // stopped compiling from this crate once the struct relocated. `TestClock`
    // stays here for this conformance test and for other test doubles that only
    // need the `Clock` port.
}

#[cfg(test)]
mod oldest_completed_contract {
    use std::sync::Arc;

    use chrono::{Duration, TimeZone, Utc};
    use ego_domain::operation::{
        OldestCompleted, OperationFingerprint, OperationKey, OperationReservationStore, OwnerFence,
        OwnerId, ReservationOutcome, ReserveRequest, StoredServiceResponse,
    };
    use ego_domain::Clock as _;

    use super::{InMemoryOperationReservationStore, TestClock};

    fn epoch() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
    }

    async fn complete(store: &InMemoryOperationReservationStore, clock: &TestClock, key: &str) {
        let outcome = store
            .reserve(ReserveRequest {
                tenant: None,
                operation_key: OperationKey::parse(key).expect("a valid key"),
                fingerprint: OperationFingerprint::new("fp"),
                owner_id: OwnerId::new("owner"),
                lease_until: clock.now() + Duration::seconds(30),
            })
            .await
            .expect("a fresh reservation");
        let lease = match outcome {
            ReservationOutcome::Fresh(lease) => lease,
            other => panic!("expected Fresh, got {other:?}"),
        };
        store
            .complete(
                &OwnerFence {
                    operation_id: lease.operation_id.clone(),
                    owner_id: lease.owner_id.clone(),
                    fencing_token: lease.fencing_token,
                },
                StoredServiceResponse::new(b"done".to_vec()),
            )
            .await
            .expect("completion");
    }

    /// A store that supports the query and holds nothing answers `Empty`.
    ///
    /// The distinction from `Unsupported` is invisible to any emitter — both
    /// produce no gauge sample — so it can only be held here, at the port. This
    /// store *can* look; it looked; the backlog is clear.
    #[tokio::test]
    async fn an_empty_store_answers_empty_and_never_unsupported() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock);

        assert_eq!(
            store.oldest_completed().await,
            Ok(OldestCompleted::Empty),
            "this store supports the query, so an empty backlog is a real answer — \
             reporting Unsupported would claim it cannot look"
        );
    }

    /// An in-progress reservation is not backlog, however long it has been running.
    ///
    /// Same predicate the purge uses: the guarantee is stated in terms of state.
    #[tokio::test]
    async fn an_in_progress_reservation_is_not_an_oldest_completion() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());
        store
            .reserve(ReserveRequest {
                tenant: None,
                operation_key: OperationKey::parse("op-running").expect("a valid key"),
                fingerprint: OperationFingerprint::new("fp"),
                owner_id: OwnerId::new("owner"),
                lease_until: clock.now() + Duration::seconds(30),
            })
            .await
            .expect("a fresh reservation");

        assert_eq!(
            store.oldest_completed().await,
            Ok(OldestCompleted::Empty),
            "nothing has completed, so there is no oldest completion to report"
        );
    }

    /// The answer is the earliest `completed_at`, not the latest.
    #[tokio::test]
    async fn the_answer_is_the_earliest_completion() {
        let clock = Arc::new(TestClock::new(epoch()));
        let store = InMemoryOperationReservationStore::new(clock.clone());

        complete(&store, &clock, "op-first").await;
        let first = clock.now();
        clock.advance(Duration::seconds(60));
        complete(&store, &clock, "op-second").await;

        assert_eq!(
            store.oldest_completed().await,
            Ok(OldestCompleted::At(first)),
            "two completions exist; the oldest is the earlier one"
        );
    }
}
