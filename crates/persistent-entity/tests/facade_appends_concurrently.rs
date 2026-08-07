//! Two appends through one `PersistenceFacade` overlap instead of taking turns.
//!
//! This is what narrowing `EventStore::append` to `&self` bought. The facade used
//! to hold the event store behind a lock — not because the store needed one, but
//! because `append` demanded `&mut self` and a lock was how a shared facade
//! produced that exclusive borrow. The lock was then held across the whole append,
//! so every entity sharing a facade queued behind every other one.
//!
//! Asserting "the lock is gone" by reading the struct would prove nothing about
//! behaviour: a reviewer can see the field, and a future change could reintroduce
//! serialisation somewhere else. This asserts the property instead, by making one
//! append block until a second has started. Under the old shape it deadlocks;
//! under the new one both proceed.

use std::sync::Arc;

use async_trait::async_trait;
use ego_domain::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};
use parking_lot::Mutex;
use persistent_entity::persistence::{InMemorySnapshotStore, PersistenceFacade};
use persistent_entity::testing::TestEvent;
use tokio::sync::Barrier;

/// A store whose `append` waits for a second append to arrive before returning.
///
/// The barrier is the whole mechanism: it can only be cleared by two appends being
/// in flight at once. If the facade serialises them, the first waits forever and
/// the test times out rather than passing quietly.
struct RendezvousEventStore {
    both_arrived: Arc<Barrier>,
}

#[async_trait]
impl EventStore<TestEvent> for RendezvousEventStore {
    async fn append(
        &self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _expected_version: i64,
        _events: Vec<StoredEvent<TestEvent>>,
    ) -> Result<i64, PersistenceError> {
        self.both_arrived.wait().await;
        Ok(1)
    }

    async fn load(
        &self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<TestEvent>>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_aggregate_ids(
        &self,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        Ok(Vec::new())
    }

    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<TestEvent>>, PersistenceError> {
        Err(PersistenceError::Internal(
            "this test double does not implement unit-of-work semantics".to_string(),
        ))
    }
}

/// Two appends through the same facade are in flight simultaneously.
///
/// The timeout is the assertion's teeth. Without it a serialising facade would hang
/// and the test would look like an infrastructure problem instead of a failure; with
/// it, the failure names what went wrong.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_appends_through_one_facade_overlap() {
    let both_arrived = Arc::new(Barrier::new(2));
    let facade: Arc<PersistenceFacade<TestEvent>> = Arc::new(PersistenceFacade::with_stores(
        Arc::new(RendezvousEventStore {
            both_arrived: Arc::clone(&both_arrived),
        }),
        Arc::new(Mutex::new(InMemorySnapshotStore::new())),
    ));

    let first = {
        let facade = Arc::clone(&facade);
        tokio::spawn(async move {
            facade
                .persist_events(
                    "counter",
                    "a",
                    Some("default"),
                    0,
                    &[TestEvent::Incremented(1)],
                )
                .await
        })
    };
    let second = {
        let facade = Arc::clone(&facade);
        tokio::spawn(async move {
            facade
                .persist_events(
                    "counter",
                    "b",
                    Some("default"),
                    0,
                    &[TestEvent::Incremented(1)],
                )
                .await
        })
    };

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        (
            first.await.expect("the first task must not panic"),
            second.await.expect("the second task must not panic"),
        )
    })
    .await
    .expect(
        "two appends through one facade must be able to overlap. A timeout here means they were \
         serialised — the first is waiting for the second to arrive, and the second cannot start \
         until the first releases something it should not be holding",
    );

    assert!(
        outcome.0.is_ok() && outcome.1.is_ok(),
        "both appends must succeed: {outcome:?}"
    );
}
