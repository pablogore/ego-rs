//! Recovery absorbs an absent stream and nothing else.
//!
//! Treating "this aggregate has no events yet" as an empty history is what lets a
//! new entity activate. Treating "the history could not be read" the same way
//! would be far worse than the defect that motivated it: the entity would recover
//! to its initial state and start appending from version zero over a stream that
//! already exists, forking it.
//!
//! The integration test that fixed the absence case cannot see this distinction —
//! it passes just as well against a facade that swallows every error — so the
//! boundary is pinned here, hermetically, with a store that fails on purpose.

use std::sync::Arc;

use async_trait::async_trait;
use ego_domain::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};
use parking_lot::Mutex;
use persistent_entity::persistence::{InMemorySnapshotStore, PersistenceFacade};
use persistent_entity::testing::TestEvent;
use tokio::sync::Mutex as AsyncMutex;

/// A store whose `load` fails for a reason that is not absence.
///
/// `Internal` stands in for every way reading a history can fail without implying
/// there is none: a dropped connection, a malformed row, a deserializer that
/// rejects a payload.
struct UnreadableEventStore;

#[async_trait]
impl EventStore<TestEvent> for UnreadableEventStore {
    async fn append(
        &mut self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _expected_version: i64,
        _events: Vec<StoredEvent<TestEvent>>,
    ) -> Result<i64, PersistenceError> {
        Err(PersistenceError::Internal("unreadable".to_string()))
    }

    async fn load(
        &self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<TestEvent>>, PersistenceError> {
        Err(PersistenceError::Internal(
            "the connection dropped mid-read".to_string(),
        ))
    }

    async fn list_aggregate_ids(
        &self,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        Err(PersistenceError::Internal("unreadable".to_string()))
    }

    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<TestEvent>>, PersistenceError> {
        Err(PersistenceError::Internal(
            "this double does not implement unit-of-work semantics".to_string(),
        ))
    }
}

fn facade_over(store: UnreadableEventStore) -> PersistenceFacade<TestEvent> {
    PersistenceFacade::with_stores(
        Arc::new(AsyncMutex::new(store)),
        Arc::new(Mutex::new(InMemorySnapshotStore::new())),
    )
}

/// A store that cannot read its history fails recovery, rather than reporting an
/// empty one.
///
/// Without this, absorbing `NotFound` and absorbing every error are
/// indistinguishable to the suite — and the second one silently converts an
/// unreadable stream into a fresh entity that will append from version zero over
/// history it never saw.
#[tokio::test]
async fn a_store_that_cannot_be_read_fails_recovery_instead_of_reporting_no_history() {
    let facade = facade_over(UnreadableEventStore);

    let outcome = facade
        .load_for_recovery("counter", "unreadable", Some("tenant-1"))
        .await;

    let message = match outcome {
        Err(message) => message,
        Ok((_, events)) => panic!(
            "a store that could not be read must fail recovery, not report an empty history; \
             got {} event(s)",
            events.len()
        ),
    };
    assert!(
        message.contains("the connection dropped mid-read"),
        "the failure must carry the store's own reason so an operator can act on it: {message}"
    );
}
