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

use ego_domain::context::TenantId;
use ego_domain::event::DomainEvent;
use ego_domain::operation::reservation::StoredResponse;
use ego_domain::operation::{OperationFingerprint, OperationKey, OperationReceipt};
use ego_domain::persistence::{EventStore, PersistenceError, StoredEvent};

/// Asserts that an [`EventStore`] implementation honours the parts of the
/// contract that are about *identity* — which rows belong to which stream, and
/// how a stream's version advances.
///
/// The caller passes a store and a way to build an event. Every scenario uses
/// its own aggregate id, so one store instance serves them all and the harness
/// needs no way to construct a fresh one — which matters because building a
/// store is asynchronous for some adapters and synchronous for others.
///
/// # What is checked, and what is deliberately not
///
/// Checked: version advance, rejection of a stale expected version, ordered
/// readback, that a tenant partition and the systemwide partition are separate
/// streams even when they share a type and an id, that the aggregate listing
/// reports each partition's own streams and only those, the unit-of-work
/// semantics — staged appends invisible until commit, durable after it, and
/// discarded when the unit of work is dropped — and that an attached
/// `operation_key` survives the round trip.
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
pub async fn assert_event_store_conformance<E, S, F>(store: &mut S, make_event: F)
where
    E: DomainEvent,
    S: EventStore<E> + ?Sized,
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
        .await
        .expect("appending two events to a fresh stream at expected version 0 must succeed");
    assert_eq!(
        advanced, 2,
        "appending two events must advance the stream's version by two"
    );

    let loaded = store
        .load("conformance", "advances", tenant)
        .await
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
        .await
        .expect("the first append must succeed");

    let stale = store
        .append(
            "conformance",
            "stale",
            tenant,
            0,
            vec![StoredEvent::without_correlation(make_event("Duplicate"))],
        )
        .await;
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
    match store.load("conformance", "never-written", tenant).await {
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
        .await
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
        .await
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
        .await
        .expect(
            "a tenant stream sharing a type and an id with a systemwide stream must be \
             independent of it",
        );

    let systemwide = store
        .load("conformance", "shared-identity", None)
        .await
        .expect("the systemwide stream must load");
    assert_eq!(
        systemwide.len(),
        2,
        "the systemwide read must return its own two events and not the tenant's"
    );
    let tenanted = store
        .load("conformance", "shared-identity", tenant)
        .await
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
        .await
        .expect("listing the systemwide partition must succeed");
    systemwide_listing.sort();
    assert_eq!(
        systemwide_listing,
        vec![("conformance".to_string(), "shared-identity".to_string())],
        "the systemwide listing must hold exactly the streams written without a tenant"
    );

    // --- An attached operation key survives the round trip -------------------
    // Storage that silently drops it would leave a later reader unable to tell
    // which operation wrote which history, which is the question a
    // duplicate-suppression decision has to answer about events that already
    // exist. The in-memory stores keep whole `StoredEvent` values, so they pass
    // this by construction; the durable one has to write a column and read it
    // back, and putting the assertion here is what stops those two from drifting.
    {
        let key = OperationKey::parse("conformance-operation-key")
            .expect("the harness's own key must be valid");
        store
            .append(
                "conformance",
                "carries-a-key",
                tenant,
                0,
                vec![
                    StoredEvent::without_correlation(make_event("Keyed"))
                        .with_operation_key(key.clone()),
                    // A second event in the same batch without a key, so the
                    // assertion cannot pass by attaching the key to everything.
                    StoredEvent::without_correlation(make_event("Unkeyed")),
                ],
            )
            .await
            .expect("appending an event that carries an operation key must succeed");

        let loaded = store
            .load("conformance", "carries-a-key", tenant)
            .await
            .expect("the stream must load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded[0].operation_key.as_ref(),
            Some(&key),
            "the operation key attached to the first event must come back exactly"
        );
        assert_eq!(
            loaded[1].operation_key, None,
            "an event appended without an operation key must not acquire one"
        );
    }

    // --- A unit of work either lands whole or not at all ---------------------
    // Asserted here rather than only against the durable store, because a staging
    // implementation and a transactional one can only be trusted to agree if the
    // same assertions are put to both. This is the "in-memory store does not
    // silently diverge" obligation, and the divergence it guards against is not
    // hypothetical: these two implementations already disagreed once about the
    // tenant-less partition while both satisfying the trait's signature.
    {
        let mut uow = store
            .begin()
            .await
            .expect("a conforming store must be able to open a unit of work");
        let staged_version = uow
            .append(
                "conformance",
                "abandoned",
                tenant,
                0,
                vec![StoredEvent::without_correlation(make_event("Staged"))],
            )
            .await
            .expect("appending inside a unit of work must succeed");
        assert_eq!(
            staged_version, 1,
            "a unit of work must report the version it advanced to, provisional though it is"
        );

        // Still invisible: the append has not been committed.
        match store.load("conformance", "abandoned", tenant).await {
            Err(PersistenceError::NotFound { .. }) => {}
            Ok(events) => panic!(
                "an uncommitted append must not be visible to a reader, saw {} event(s)",
                events.len()
            ),
            Err(other) => panic!("expected the stream to read as absent, got {other:?}"),
        }
        // Dropped without committing.
    }

    match store.load("conformance", "abandoned", tenant).await {
        Err(PersistenceError::NotFound { .. }) => {}
        Ok(events) => panic!(
            "dropping a unit of work without committing must discard its appends, but {} \
             event(s) survived",
            events.len()
        ),
        Err(other) => panic!("expected the abandoned stream to read as absent, got {other:?}"),
    }

    // Committing is the other half. Without it, "discards on drop" would also be
    // satisfied by a unit of work that never records anything at all.
    let mut uow = store
        .begin()
        .await
        .expect("a conforming store must be able to open a unit of work");
    // The first event carries an operation key: a unit of work writes through its
    // own code path, so the key surviving a direct append proves nothing about it.
    let uow_key =
        OperationKey::parse("conformance-uow-key").expect("the harness's own key must be valid");
    uow.append(
        "conformance",
        "committed-uow",
        tenant,
        0,
        vec![
            StoredEvent::without_correlation(make_event("One")).with_operation_key(uow_key.clone()),
            StoredEvent::without_correlation(make_event("Two")),
        ],
    )
    .await
    .expect("appending inside a unit of work must succeed");
    // A second append to the same stream inside one unit of work must see the
    // first: a version check that consulted only committed state would reject
    // this, which is the mistake a staging implementation is most likely to make.
    let second = uow
        .append(
            "conformance",
            "committed-uow",
            tenant,
            2,
            vec![StoredEvent::without_correlation(make_event("Three"))],
        )
        .await
        .expect("a second append in the same unit of work must see the first one's version");
    assert_eq!(second, 3);
    uow.commit()
        .await
        .expect("committing a unit of work must succeed");

    let committed = store
        .load("conformance", "committed-uow", tenant)
        .await
        .expect("the committed stream must load");
    assert_eq!(
        committed.len(),
        3,
        "every append made in the unit of work must be durable after the commit"
    );
    assert_eq!(committed[0].event.event_type(), "One");
    assert_eq!(committed[2].event.event_type(), "Three");
    assert_eq!(
        committed[0].operation_key.as_ref(),
        Some(&uow_key),
        "an operation key attached inside a unit of work must survive its commit"
    );
    assert_eq!(
        committed[1].operation_key, None,
        "and must not spread to the events appended beside it"
    );

    let mut tenant_listing = store
        .list_aggregate_ids(tenant)
        .await
        .expect("listing a tenant partition must succeed");
    tenant_listing.sort();
    assert_eq!(
        tenant_listing,
        vec![
            ("conformance".to_string(), "advances".to_string()),
            ("conformance".to_string(), "carries-a-key".to_string()),
            ("conformance".to_string(), "committed-uow".to_string()),
            ("conformance".to_string(), "shared-identity".to_string()),
            ("conformance".to_string(), "stale".to_string()),
        ],
        "the tenant listing must hold exactly that tenant's streams"
    );
    // --- A receipt shares the fate of the unit of work that confirmed it -----
    //
    // The receipt is what makes "did this already happen?" answerable, so a
    // store where it can become durable independently of the events it
    // describes would report operations as done whose effects never landed.
    // These four assertions pin that it cannot.
    let receipt_key =
        OperationKey::parse("conformance-receipt").expect("a non-empty operation key must parse");
    let receipt_tenant =
        TenantId::new("conformance-tenant").expect("a non-empty tenant id must parse");
    let receipt = OperationReceipt::new(
        "conformance",
        "receipted",
        Some(receipt_tenant.clone()),
        receipt_key.clone(),
        OperationFingerprint::new("fingerprint-a"),
        StoredResponse::new(b"the recorded outcome".to_vec()),
    );

    assert_eq!(
        store
            .find_receipt("conformance", "receipted", tenant, receipt_key.as_str())
            .await
            .expect("looking up an absent receipt must succeed, not error"),
        None,
        "an operation that never ran must report no receipt: a miss is the \
         ordinary first-execution case, not a failure"
    );

    // Dropped without committing. Nothing it staged may survive.
    {
        let mut discarded = store
            .begin()
            .await
            .expect("opening a unit of work must succeed");
        discarded
            .confirm_receipt(&receipt)
            .await
            .expect("confirming a receipt inside a unit of work must succeed");
        assert_eq!(
            store
                .find_receipt("conformance", "receipted", tenant, receipt_key.as_str())
                .await
                .expect("a lookup during an open unit of work must succeed"),
            None,
            "a receipt confirmed in an open unit of work must be invisible until \
             that unit of work commits"
        );
    }
    assert_eq!(
        store
            .find_receipt("conformance", "receipted", tenant, receipt_key.as_str())
            .await
            .expect("a lookup after a discarded unit of work must succeed"),
        None,
        "dropping a unit of work without committing must discard its receipt, \
         exactly as it discards its appends"
    );

    // Committed with zero events. This is the case the receipt exists for: a
    // success that emits nothing has no event row to carry its completion, so
    // without a receipt it is indistinguishable from a command that never ran.
    let mut empty = store
        .begin()
        .await
        .expect("opening a unit of work must succeed");
    empty
        .confirm_receipt(&receipt)
        .await
        .expect("confirming a receipt must succeed");
    empty
        .commit()
        .await
        .expect("committing a unit of work that appended no events must succeed");

    let found = store
        .find_receipt("conformance", "receipted", tenant, receipt_key.as_str())
        .await
        .expect("a lookup after commit must succeed")
        .expect("a committed receipt must be found, even though no event was appended");
    assert_eq!(
        found.fingerprint().as_str(),
        "fingerprint-a",
        "the receipt must round-trip the fingerprint it was confirmed with"
    );
    assert_eq!(
        found.response().as_bytes(),
        b"the recorded outcome",
        "the receipt must round-trip the response a matching retry will replay"
    );

    // A different request reusing the operation key is refused, not answered
    // with someone else's result and not silently overwriting the receipt.
    let mut conflicting = store
        .begin()
        .await
        .expect("opening a unit of work must succeed");
    let other_fingerprint = OperationReceipt::new(
        "conformance",
        "receipted",
        Some(receipt_tenant),
        receipt_key.clone(),
        OperationFingerprint::new("fingerprint-b"),
        StoredResponse::new(b"a different outcome".to_vec()),
    );
    let refused = conflicting.confirm_receipt(&other_fingerprint).await;
    assert!(
        matches!(refused, Err(PersistenceError::Conflict { .. })),
        "confirming a receipt for an existing identity under a different \
         fingerprint must be refused as a conflict, never overwrite what is \
         stored: overwriting would hand one caller another caller's result"
    );
    drop(conflicting);

    let unchanged = store
        .find_receipt("conformance", "receipted", tenant, receipt_key.as_str())
        .await
        .expect("a lookup after a refused confirmation must succeed")
        .expect("the original receipt must still be there");
    assert_eq!(
        unchanged.response().as_bytes(),
        b"the recorded outcome",
        "a refused confirmation must leave the stored response untouched"
    );
}
