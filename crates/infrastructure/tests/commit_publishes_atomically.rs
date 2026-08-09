//! `commit` must publish events and their receipt as one visible step.
//!
//! This is not the "dropping rolls back" property, which is about an *aborted*
//! unit of work. It is about a **successful** one: while it publishes, no other
//! task may observe the events without the receipt that records them.
//!
//! Why that window is dangerous rather than untidy: a reader seeing committed
//! events but no receipt finds no evidence the operation ran, and is free to run
//! it again. The receipt exists to prevent exactly that, so a store that can
//! expose the gap has a hole in the guarantee — not a cosmetic ordering flaw.
//!
//! The durable store gets this from its transaction. The in-memory one has to
//! build it, which is why it needs a test and the durable one does not.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use ego_domain::context::TenantId;
use ego_domain::event::DomainEvent;
use ego_domain::operation::{
    AggregateOutcome, OperationFingerprint, OperationKey, OperationReceipt,
};
use ego_domain::persistence::{EventStore, StoredEvent};
use ego_infrastructure::persistence::in_memory::InMemoryEventStore;

const TYPE: &str = "atomic";
const TENANT: Option<&str> = Some("t1");
const ROUNDS: usize = 300;

#[derive(Debug, Clone, PartialEq)]
struct Marked {
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
}

impl Marked {
    fn new() -> Self {
        Self {
            payload: serde_json::json!({}),
            occurred_at: Utc::now(),
        }
    }
}

impl DomainEvent for Marked {
    fn aggregate_id(&self) -> &str {
        // The store takes the aggregate identity as explicit arguments to
        // `append`, never from the event.
        ""
    }

    fn event_type(&self) -> &str {
        "Marked"
    }

    fn payload(&self) -> &serde_json::Value {
        &self.payload
    }

    fn occurred_at(&self) -> &DateTime<Utc> {
        &self.occurred_at
    }
}

fn receipt_for(id: &str) -> OperationReceipt {
    OperationReceipt::new(
        TYPE,
        id,
        Some(TenantId::new("t1").expect("a non-empty tenant parses")),
        OperationKey::parse(format!("op-{id}")).expect("a non-empty key parses"),
        OperationFingerprint::new("fp"),
        AggregateOutcome::events(1, 1).expect("an ascending inclusive range is valid"),
    )
}

/// A writer publishing events and receipts, and a reader checking the invariant
/// that binds them, running against each other.
///
/// The reader's rule is one line: **if the stream has events, the receipt must
/// be there too.** Under a `commit` that publishes in two steps, the reader can
/// land between them and see the first without the second. Under one that
/// publishes atomically it never can, however often it looks.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reader_never_sees_committed_events_without_their_receipt() {
    let store: Arc<InMemoryEventStore<Marked>> = Arc::new(InMemoryEventStore::new());
    let done = Arc::new(AtomicBool::new(false));
    let violations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(AtomicUsize::new(0));

    let reader = {
        let store = Arc::clone(&store);
        let done = Arc::clone(&done);
        let violations = Arc::clone(&violations);
        let observed = Arc::clone(&observed);
        tokio::spawn(async move {
            while !done.load(Ordering::SeqCst) {
                for i in 0..ROUNDS {
                    let id = i.to_string();
                    let has_events = store
                        .load(TYPE, &id, TENANT)
                        .await
                        .map(|events| !events.is_empty())
                        .unwrap_or(false);
                    if !has_events {
                        continue;
                    }
                    observed.fetch_add(1, Ordering::SeqCst);

                    let key = format!("op-{id}");
                    let has_receipt = store
                        .find_receipt(TYPE, &id, TENANT, &key)
                        .await
                        .expect("a receipt lookup must not fail")
                        .is_some();
                    if !has_receipt {
                        violations.fetch_add(1, Ordering::SeqCst);
                    }
                }
                tokio::task::yield_now().await;
            }
        })
    };

    for i in 0..ROUNDS {
        let id = i.to_string();
        let mut uow = store
            .begin()
            .await
            .expect("opening a unit of work succeeds");
        uow.append(
            TYPE,
            &id,
            TENANT,
            0,
            vec![StoredEvent::without_correlation(Marked::new())],
        )
        .await
        .expect("staging one event succeeds");
        uow.confirm_receipt(&receipt_for(&id))
            .await
            .expect("confirming its receipt succeeds");
        uow.commit().await.expect("committing succeeds");
        tokio::task::yield_now().await;
    }

    done.store(true, Ordering::SeqCst);
    reader.await.expect("the reader task must not panic");

    // A reader that never looked would report zero violations too, so the test
    // states what it actually saw before claiming the invariant held.
    assert!(
        observed.load(Ordering::SeqCst) > 0,
        "the reader observed no committed stream at all, so it proved nothing — \
         the interleaving this test depends on did not happen"
    );
    assert_eq!(
        violations.load(Ordering::SeqCst),
        0,
        "a reader observed committed events whose receipt was not yet visible: \
         commit published in more than one step, and in that window the operation \
         looks like it never ran"
    );
}
