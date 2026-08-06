//! `assert_event_store_conformance` — a shared conformance check for
//! [`EventStore`] implementations.
//!
//! It belongs here rather than beside any single adapter, for a reason that was
//! demonstrated rather than anticipated: two implementations of this port
//! disagreed about the tenant-less ("systemwide") partition, and the hermetic
//! suite happened to test the one that was right.
//!
//! The in-memory store keys streams by a tuple holding an `Option<String>`,
//! where `None == None`, so a systemwide stream always found its own history.
//! The PostgreSQL store compared `tenant_id` to a bound SQL NULL with `=`,
//! which in three-valued logic is never true — so a systemwide stream was
//! invisible to its own version check, and every append to one rewrote version
//! 1 and duplicated history silently. Both implementations satisfied the trait's
//! signature. Only one satisfied its meaning, and nothing in the workspace
//! compared them.
//!
//! That is what this function is for: the contract lives in one place, and each
//! adapter is judged against it instead of against its own author's reading of
//! it.

use ego_domain::event::DomainEvent;
use ego_domain::persistence::{EventStore, PersistenceError, StoredEvent};

/// Asserts that an [`EventStore`] implementation honours the parts of the
/// contract that are about *identity* — which rows belong to which stream, and
/// how a stream's version advances.
///
/// The caller passes a store and a way to build an event. Every scenario uses
/// its own aggregate id, so one store instance serves them all and the harness
/// needs no way to construct a fresh one — which matters because building a
/// store is asynchronous for some adapters and synchronous for others, while
/// this trait is synchronous for all of them.
///
/// # What is checked, and what is deliberately not
///
/// Checked: version advance, rejection of a stale expected version, ordered
/// readback, that a tenant partition and the systemwide partition are separate
/// streams even when they share a type and an id, and that the aggregate listing
/// reports each partition's own streams and only those.
///
/// Not checked: durability, concurrency, snapshotting, or anything an
/// implementation may reasonably decide for itself. A conformance harness that
/// asserts more than the contract turns every adapter into a copy of whichever
/// one it was written against.
///
/// # Why the systemwide cases are not optional
///
/// A store could pass every tenant-scoped assertion here and still be the
/// broken implementation described above. The systemwide partition is the case
/// that diverged, so a harness that let an adapter skip it would enforce exactly
/// the part of the contract that was never in doubt.
///
/// The pair of assertions about it also has to work in both directions. "A
/// systemwide read sees its own history" is satisfiable by a comparison that
/// matches every row — which would return another tenant's events, an isolation
/// breach worse than the invisibility it replaces. So separation is asserted
/// alongside visibility, never on its own.
///
/// # Panics
///
/// Panics with a descriptive message on the first divergence from the contract.
pub fn assert_event_store_conformance<E, S, F>(store: &mut S, make_event: F)
where
    E: DomainEvent,
    S: EventStore<E>,
    F: Fn(&str) -> E,
{
    let tenant = Some("conformance-tenant");

    // --- A tenant-scoped stream advances and reads back in order -------------
    let advanced = store
        .append(
            "conformance",
            "advances",
            tenant,
            0,
            vec![
                StoredEvent::without_correlation(make_event("First")),
                StoredEvent::without_correlation(make_event("Second")),
            ],
        )
        .expect("appending two events to a fresh stream at expected version 0 must succeed");
    assert_eq!(
        advanced, 2,
        "appending two events must advance the stream's version by two"
    );

    let loaded = store
        .load("conformance", "advances", tenant)
        .expect("a stream that was just appended to must be loadable");
    assert_eq!(
        loaded.len(),
        2,
        "load must return exactly the events that were appended"
    );
    assert_eq!(
        loaded[0].event.event_type(),
        "First",
        "load must return events in append order"
    );
    assert_eq!(loaded[1].event.event_type(), "Second");

    // --- A stale expected version is rejected, and the report is truthful ----
    store
        .append(
            "conformance",
            "stale",
            tenant,
            0,
            vec![StoredEvent::without_correlation(make_event("Only"))],
        )
        .expect("the first append must succeed");

    let stale = store.append(
        "conformance",
        "stale",
        tenant,
        0,
        vec![StoredEvent::without_correlation(make_event("Duplicate"))],
    );
    match stale {
        Err(PersistenceError::Conflict {
            expected, actual, ..
        }) => {
            assert_eq!(
                expected, 0,
                "the conflict must report the version the caller expected"
            );
            assert_eq!(
                actual, 1,
                "the conflict must report the version the stream really has, not the expected one"
            );
        }
        other => panic!(
            "re-appending at an already-consumed expected version must be a conflict, got {other:?}"
        ),
    }

    // --- An absent stream is absent -----------------------------------------
    // The event type is not required to be `Debug`, so the diagnostic reports the
    // shape of the unexpected result rather than its contents.
    match store.load("conformance", "never-written", tenant) {
        Err(PersistenceError::NotFound { .. }) => {}
        Ok(events) => panic!(
            "loading a stream that was never appended to must report it as not found, \
             got {} event(s)",
            events.len()
        ),
        Err(other) => panic!(
            "loading a stream that was never appended to must report it as not found, \
             got {other:?}"
        ),
    }

    // --- The systemwide partition is a first-class partition -----------------
    let systemwide_first = store
        .append(
            "conformance",
            "shared-identity",
            None,
            0,
            vec![StoredEvent::without_correlation(make_event("Systemwide"))],
        )
        .expect("a systemwide append at expected version 0 must succeed");
    assert_eq!(systemwide_first, 1);

    let systemwide_second = store
        .append(
            "conformance",
            "shared-identity",
            None,
            1,
            vec![StoredEvent::without_correlation(make_event(
                "SystemwideAgain",
            ))],
        )
        .expect(
            "a systemwide stream must see the history it just wrote: appending at expected \
             version 1 must succeed, not be rejected as though the stream were empty",
        );
    assert_eq!(
        systemwide_second, 2,
        "a systemwide stream must advance rather than restart from an apparently empty history"
    );

    // Same type, same id, different partition: a fresh stream whose own expected
    // version is 0. If this fails as a conflict, the two partitions are being
    // treated as one stream.
    store
        .append(
            "conformance",
            "shared-identity",
            tenant,
            0,
            vec![StoredEvent::without_correlation(make_event("Tenanted"))],
        )
        .expect(
            "a tenant stream sharing a type and an id with a systemwide stream must be \
             independent of it",
        );

    let systemwide = store
        .load("conformance", "shared-identity", None)
        .expect("the systemwide stream must load");
    assert_eq!(
        systemwide.len(),
        2,
        "the systemwide read must return its own two events and not the tenant's"
    );
    let tenanted = store
        .load("conformance", "shared-identity", tenant)
        .expect("the tenant stream must load");
    assert_eq!(
        tenanted.len(),
        1,
        "the tenant read must return its own event and not the systemwide ones"
    );
    assert_eq!(tenanted[0].event.event_type(), "Tenanted");

    // --- The listing reports each partition's own streams, and only those ----
    let mut systemwide_listing = store
        .list_aggregate_ids(None)
        .expect("listing the systemwide partition must succeed");
    systemwide_listing.sort();
    assert_eq!(
        systemwide_listing,
        vec![("conformance".to_string(), "shared-identity".to_string())],
        "the systemwide listing must hold exactly the streams written without a tenant"
    );

    let mut tenant_listing = store
        .list_aggregate_ids(tenant)
        .expect("listing a tenant partition must succeed");
    tenant_listing.sort();
    assert_eq!(
        tenant_listing,
        vec![
            ("conformance".to_string(), "advances".to_string()),
            ("conformance".to_string(), "shared-identity".to_string()),
            ("conformance".to_string(), "stale".to_string()),
        ],
        "the tenant listing must hold exactly that tenant's streams"
    );
}
