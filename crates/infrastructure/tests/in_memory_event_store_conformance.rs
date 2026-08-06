//! The in-memory event store, judged against the shared `EventStore`
//! conformance contract.
//!
//! This store keys streams by a tuple holding an `Option<String>`, and in Rust
//! `None == None`, so it has always handled the tenant-less ("systemwide")
//! partition correctly. The PostgreSQL store, comparing the same identity in
//! SQL, had not — and nothing compared the two. Running both against one
//! harness is what turns "these implement the same trait" into "these agree
//! about what the trait means".
//!
//! Hermetic on purpose: no container, no database. If the contract can be
//! stated without external resources, the implementation that needs none should
//! be checked without them.

use chrono::{DateTime, Utc};
use ego_domain::event::DomainEvent;
use ego_infrastructure::persistence::in_memory::InMemoryEventStore;
use ego_testkit::assert_event_store_conformance;

#[derive(Debug, Clone)]
struct ConformanceEvent {
    kind: String,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

impl DomainEvent for ConformanceEvent {
    fn aggregate_id(&self) -> &str {
        // The store takes the aggregate identity as explicit arguments to
        // `append`, never from the event, so nothing in the contract reads this.
        ""
    }

    fn event_type(&self) -> &str {
        &self.kind
    }

    fn payload(&self) -> &serde_json::Value {
        &self.payload
    }

    fn occurred_at(&self) -> &DateTime<Utc> {
        &self.occurred_at
    }
}

// A plain `#[tokio::test]`, current-thread: the contract is asynchronous now, but
// this store does no I/O, so nothing here needs a multi-thread runtime. The
// PostgreSQL suite used to require `flavor = "multi_thread"` because the store
// bridged async to sync with `block_in_place`, which panics on a current-thread
// runtime — a storage detail that had leaked into test attributes.
#[tokio::test]
async fn the_in_memory_event_store_conforms() {
    let mut store: InMemoryEventStore<ConformanceEvent> = InMemoryEventStore::new();

    assert_event_store_conformance(&mut store, |kind| ConformanceEvent {
        kind: kind.to_string(),
        payload: serde_json::json!({ "kind": kind }),
        occurred_at: Utc::now(),
    })
    .await;
}
