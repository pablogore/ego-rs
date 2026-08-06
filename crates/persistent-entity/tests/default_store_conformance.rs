//! The store the runtime builder installs by default, judged against the shared
//! `EventStore` conformance contract.
//!
//! This store sat outside the harness while the other two implementations were in
//! it, and everything the harness would have caught, it was getting wrong: streams
//! shared one namespace across every tenant, and a stream it had never seen was
//! reported as empty rather than absent. Both were reachable without anyone
//! choosing this store, because it is the one installed when no store is supplied.
//!
//! Being inside the harness is the point of this file. The assertions live in
//! `ego-testkit` alongside the other implementations', so this store is judged
//! against the same contract rather than against its own author's reading of it.

use chrono::{DateTime, Utc};
use ego_domain::event::DomainEvent;
use ego_testkit::assert_event_store_conformance;
use persistent_entity::persistence::InMemoryEventStore;

/// A local event type rather than `TestEvent`, because the harness identifies
/// events by the name `event_type()` returns and needs to distinguish several
/// within one stream. `TestEvent`'s variants each report a fixed name, so it
/// cannot carry the harness's labels.
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

/// The default store conforms.
///
/// Hermetic: no container, no database. The contract is asynchronous, but this
/// store does no I/O, so the default current-thread runtime is all it needs.
#[tokio::test]
async fn the_default_in_memory_store_conforms() {
    let mut store: InMemoryEventStore<ConformanceEvent> = InMemoryEventStore::new();

    assert_event_store_conformance(&mut store, |kind| ConformanceEvent {
        kind: kind.to_string(),
        payload: serde_json::json!({ "kind": kind }),
        occurred_at: Utc::now(),
    })
    .await;
}
