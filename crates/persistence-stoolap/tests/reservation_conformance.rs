//! Runs the shared [`OperationReservationStore`] conformance contract against
//! [`StoolapOperationReservationStore`] (STOOLAP-S3, `tasks.md` 2.11), plus
//! the Stoolap-specific properties the shared, implementation-agnostic
//! harness cannot exercise (`tasks.md` Phase 3 / design.md "Testing
//! Strategy"): reopen durability, same-process multi-instance concurrency,
//! tenant isolation, and the purge dialect trap.

use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use ego_domain::operation::{
    Lease, OperationFingerprint, OperationKey, OperationReservationStore, OwnerFence, OwnerId,
    ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse,
};
use ego_domain::{Clock, TenantId};
use ego_persistence_stoolap::StoolapOperationReservationStore;
use ego_testkit::{assert_reservation_store_conformance, TestClock};

/// The fixed instant every scenario in the shared harness starts from
/// (`testkit/src/reservation_conformance.rs::epoch`, duplicated here exactly
/// as `testkit/src/reservation.rs`'s own in-memory conformance test does —
/// the harness's `epoch()` is private to that module).
fn epoch() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

/// Builds a `ReserveRequest` for a systemwide-scope key, mirroring the
/// colocated unit tests' `request()` helper in
/// `src/operation/reservation.rs` (private to that module, so duplicated
/// here for this integration binary).
fn request(owner: &str, key: &str, lease_until: DateTime<Utc>) -> ReserveRequest {
    tenant_request(None, owner, key, lease_until)
}

/// The same builder, with an explicit tenant scope for the isolation tests.
fn tenant_request(
    tenant: Option<TenantId>,
    owner: &str,
    key: &str,
    lease_until: DateTime<Utc>,
) -> ReserveRequest {
    ReserveRequest {
        tenant,
        operation_key: OperationKey::parse(key).expect("a non-empty operation key parses"),
        fingerprint: OperationFingerprint::new("fp-1"),
        owner_id: OwnerId::new(owner),
        lease_until,
    }
}

fn fence_of(lease: &Lease) -> OwnerFence {
    OwnerFence {
        operation_id: lease.operation_id.clone(),
        owner_id: lease.owner_id.clone(),
        fencing_token: lease.fencing_token,
    }
}

async fn fresh() -> (StoolapOperationReservationStore, Arc<TestClock>) {
    // Leaked rather than returned: the harness's factory signature is
    // `Fn() -> Fut` with no teardown hook, so the tempdir must outlive the
    // scenario that opens the store. Each scenario gets a distinct directory
    // via `tempfile`'s random suffix, and the whole process exits once the
    // test binary finishes — this is a test-only leak, not a long-running
    // process concern.
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let clock = Arc::new(TestClock::new(epoch()));
    let store = StoolapOperationReservationStore::open(&dir, clock.clone())
        .await
        .expect("open StoolapOperationReservationStore");
    (store, clock)
}

#[tokio::test]
async fn stoolap_reservation_store_satisfies_the_shared_conformance_contract() {
    assert_reservation_store_conformance::<StoolapOperationReservationStore, _, _>(fresh).await;
}

/// spec `persistence-stoolap-operation-reservation`: "Ownership and fencing
/// survive a close/reopen cycle" (Requirement: Reservations Survive Close
/// And Reopen).
///
/// `tasks.md` 3.1. Proves three things a fresh/empty database could not:
/// the takeover that happened before the close is what the reopened store
/// observes (not `Fresh`, which is what an empty table would answer), the
/// fencing token and tenant scope are exactly what they were, and the
/// displaced owner's now-stale fence is still rejected — not merely that a
/// row exists.
#[tokio::test]
async fn reservations_survive_close_and_reopen() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let clock = Arc::new(TestClock::new(epoch()));
    let tenant = TenantId::new("tenant-reopen").expect("a non-empty tenant id parses");
    let key = "op-reopen";

    let store = StoolapOperationReservationStore::open(&dir, clock.clone())
        .await
        .expect("open StoolapOperationReservationStore");

    let fresh = store
        .reserve(tenant_request(
            Some(tenant.clone()),
            "owner-a",
            key,
            epoch() + Duration::seconds(30),
        ))
        .await
        .unwrap();
    let stale_fence = match fresh {
        ReservationOutcome::Fresh(lease) => fence_of(&lease),
        other => panic!("setup: expected Fresh, got {other:?}"),
    };

    clock.advance(Duration::seconds(31));
    let taken_over = store
        .reserve(tenant_request(
            Some(tenant.clone()),
            "owner-b",
            key,
            clock.now() + Duration::seconds(30),
        ))
        .await
        .unwrap();
    let (current_fence, current_token) = match taken_over {
        ReservationOutcome::TakenOver(lease) => (fence_of(&lease), lease.fencing_token),
        other => panic!("setup: expected TakenOver, got {other:?}"),
    };

    // Close: the only handle to this DSN is dropped, so Stoolap's
    // process-global registry has nothing left to keep the engine alive
    // (design.md "Concurrency Scope").
    drop(store);

    let reopened = StoolapOperationReservationStore::open(&dir, clock.clone())
        .await
        .expect("reopen StoolapOperationReservationStore at the same path");

    // Not a fresh, empty state: the same key must still observe the
    // persisted owner and fencing token, never a second `Fresh` grant.
    let observed = reopened
        .reserve(tenant_request(
            Some(tenant.clone()),
            "owner-b",
            key,
            clock.now() + Duration::seconds(30),
        ))
        .await
        .unwrap();
    match observed {
        ReservationOutcome::OwnedInProgress(lease) => {
            assert_eq!(
                lease.fencing_token, current_token,
                "fencing token must be intact across close/reopen"
            );
            assert_eq!(lease.owner_id, OwnerId::new("owner-b"));
        }
        other => panic!(
            "expected the reopened store to observe the persisted OwnedInProgress state, \
             not a fresh empty one; got {other:?}"
        ),
    }

    // The pre-takeover owner's fence, stale before the close, is still
    // rejected after reopen.
    assert_eq!(
        reopened
            .renew(&stale_fence, clock.now() + Duration::seconds(60))
            .await,
        Err(ReservationError::StaleOwner)
    );

    // The current owner's fence still verifies after reopen.
    reopened
        .complete(
            &current_fence,
            StoredServiceResponse::new(b"reopened".to_vec()),
        )
        .await
        .expect("the current owner's fence must still verify after reopen");
}

/// spec: "A fresh reservation is exclusive". `tasks.md` 3.2.
///
/// Two real concurrent tasks — not a sequential simulation — race
/// `reserve` for the identical, brand-new `operation_key`. Exactly one may
/// be granted `Fresh`. The other must observe a state the port's own
/// contract calls honest: either `OtherInProgress`, or — design.md AD-5, "a
/// lost MVCC race maps to `Backend`, never to `StaleOwner`" — the
/// documented transient retry signal a genuinely lost race under real
/// concurrency may surface instead of inventing an outcome. Never a second
/// `Fresh` grant, never `StaleOwner`, never a corrupted/double-owned result.
#[tokio::test(flavor = "multi_thread")]
async fn two_concurrent_reserves_for_the_same_key_grant_exactly_one_fresh_reservation() {
    let (store, _clock) = fresh().await;
    let store = Arc::new(store);
    let lease_until = epoch() + Duration::seconds(30);

    let a = {
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            store
                .reserve(request("owner-a", "op-race", lease_until))
                .await
        })
    };
    let b = {
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            store
                .reserve(request("owner-b", "op-race", lease_until))
                .await
        })
    };

    let (a_result, b_result) = tokio::join!(a, b);
    let a_outcome = a_result.expect("task a must not panic");
    let b_outcome = b_result.expect("task b must not panic");

    let is_fresh = |r: &Result<ReservationOutcome, ReservationError>| {
        matches!(r, Ok(ReservationOutcome::Fresh(_)))
    };
    let fresh_count = [&a_outcome, &b_outcome]
        .iter()
        .filter(|r| is_fresh(r))
        .count();
    assert_eq!(
        fresh_count, 1,
        "exactly one concurrent contender must be granted Fresh, got {a_outcome:?} / {b_outcome:?}"
    );

    let loser = if is_fresh(&a_outcome) {
        &b_outcome
    } else {
        &a_outcome
    };
    match loser {
        Ok(ReservationOutcome::OtherInProgress) => {}
        Err(ReservationError::Backend(msg)) if msg.contains("retry") => {}
        other => panic!(
            "the losing contender must observe OtherInProgress or the documented \
             transient retry signal (AD-5), never a corrupted or double-owned \
             result; got {other:?}"
        ),
    }
}

/// design.md Concurrency Scope: "Same process, two runtime instances / two
/// store instances at the same path — Yes ... Must be tested, not assumed
/// (TS-4)". `tasks.md` 3.2.
///
/// Two independent `StoolapOperationReservationStore` handles are opened
/// and kept alive at once against the same path. A reservation made
/// through one instance, taken over through the other after its lease
/// expires, must be visible — with a strictly greater fencing token and the
/// prior owner locked out — through either instance: proof the two share
/// Stoolap's one process-global engine for that DSN rather than two
/// independent databases.
#[tokio::test]
async fn two_live_store_instances_at_one_path_share_the_same_engine() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let clock = Arc::new(TestClock::new(epoch()));
    let store_a = StoolapOperationReservationStore::open(&dir, clock.clone())
        .await
        .expect("open store_a");
    let store_b = StoolapOperationReservationStore::open(&dir, clock.clone())
        .await
        .expect("open store_b, a separate instance at the same path");

    let key = "op-two-instances";
    let fresh = store_a
        .reserve(request("owner-a", key, epoch() + Duration::seconds(30)))
        .await
        .unwrap();
    let original_fence = match fresh {
        ReservationOutcome::Fresh(lease) => fence_of(&lease),
        other => panic!("setup: expected Fresh, got {other:?}"),
    };

    clock.advance(Duration::seconds(31));

    let taken_over = store_b
        .reserve(request("owner-b", key, clock.now() + Duration::seconds(30)))
        .await
        .unwrap();
    let new_fence = match taken_over {
        ReservationOutcome::TakenOver(lease) => {
            assert!(
                lease.fencing_token > original_fence.fencing_token,
                "takeover through the second instance must still mint a strictly \
                 greater fencing token"
            );
            fence_of(&lease)
        }
        other => panic!("expected TakenOver via store_b, got {other:?}"),
    };

    // The displaced owner's fence is rejected through the OTHER instance —
    // proof both observe one shared engine, not two independent databases.
    assert_eq!(
        store_a
            .renew(&original_fence, clock.now() + Duration::seconds(60))
            .await,
        Err(ReservationError::StaleOwner)
    );

    // The new owner's fence, minted via store_b, verifies through store_a.
    store_a
        .complete(&new_fence, StoredServiceResponse::new(b"done".to_vec()))
        .await
        .expect("the takeover fence minted via store_b must verify via store_a");
}

/// spec: "Two tenants with the identical operation key remain isolated".
/// `tasks.md` 3.3.
///
/// Tenant A, tenant B, and the systemwide scope each reserve the identical
/// `operation_key`. All three are independently `Fresh`; completing one
/// does not affect the others' in-progress state.
#[tokio::test]
async fn identical_operation_key_under_different_tenants_and_systemwide_are_isolated() {
    let (store, _clock) = fresh().await;
    let tenant_a = TenantId::new("tenant-a").expect("a non-empty tenant id parses");
    let tenant_b = TenantId::new("tenant-b").expect("a non-empty tenant id parses");
    let key = "shared-op-key";
    let lease_until = epoch() + Duration::seconds(30);

    let outcome_a = store
        .reserve(tenant_request(
            Some(tenant_a.clone()),
            "owner-a",
            key,
            lease_until,
        ))
        .await
        .unwrap();
    let outcome_b = store
        .reserve(tenant_request(
            Some(tenant_b.clone()),
            "owner-b",
            key,
            lease_until,
        ))
        .await
        .unwrap();
    let outcome_system = store
        .reserve(tenant_request(None, "owner-sys", key, lease_until))
        .await
        .unwrap();

    assert!(
        matches!(outcome_a, ReservationOutcome::Fresh(_)),
        "tenant A's reservation must be Fresh, got {outcome_a:?}"
    );
    assert!(
        matches!(outcome_b, ReservationOutcome::Fresh(_)),
        "tenant B's identical key must be its own Fresh reservation, got {outcome_b:?}"
    );
    assert!(
        matches!(outcome_system, ReservationOutcome::Fresh(_)),
        "the systemwide scope's identical key must be its own Fresh reservation, \
         got {outcome_system:?}"
    );

    // Completing tenant A's reservation must not be observable through
    // tenant B's or the systemwide scope's identical key.
    let fence_a = match outcome_a {
        ReservationOutcome::Fresh(lease) => fence_of(&lease),
        _ => unreachable!(),
    };
    store
        .complete(
            &fence_a,
            StoredServiceResponse::new(b"tenant-a-response".to_vec()),
        )
        .await
        .unwrap();

    let observe_b = store
        .reserve(tenant_request(
            Some(tenant_b.clone()),
            "owner-b",
            key,
            lease_until,
        ))
        .await
        .unwrap();
    assert!(
        matches!(observe_b, ReservationOutcome::OwnedInProgress(_)),
        "tenant B's reservation must remain independently in progress, unaffected by \
         tenant A's completion; got {observe_b:?}"
    );

    let observe_system = store
        .reserve(tenant_request(None, "owner-sys", key, lease_until))
        .await
        .unwrap();
    assert!(
        matches!(observe_system, ReservationOutcome::OwnedInProgress(_)),
        "the systemwide reservation must remain independently in progress, unaffected \
         by tenant A's completion; got {observe_system:?}"
    );
}

/// spec: "An in-progress reservation is never purged"; design.md "Purge
/// dialect constraint" — `DELETE ... WHERE col IN (SELECT ... LIMIT n)`
/// silently deletes zero rows against Stoolap 0.4.0
/// (`effect-store/src/stoolap/mod.rs:292-301`), which is exactly why
/// `purge_completed_before` selects the eligible batch and deletes each row
/// by its own re-asserted equality predicate instead. `tasks.md` 3.4.
///
/// Proves the two-step workaround actually deletes real, eligible rows —
/// not the zero the single-statement form would silently return — while
/// leaving every ineligible row (not-yet-completed, or completed after the
/// cutoff) untouched, and that a purged key is genuinely gone rather than
/// soft-deleted: it is immediately reservable again.
#[tokio::test]
async fn purge_deletes_eligible_rows_via_the_two_step_workaround_and_spares_the_rest() {
    let (store, clock) = fresh().await;
    let cutoff = epoch() + Duration::seconds(100);

    async fn complete_at(
        store: &StoolapOperationReservationStore,
        clock: &TestClock,
        key: &str,
        at: DateTime<Utc>,
    ) {
        clock.advance(at - clock.now());
        let outcome = store
            .reserve(request("owner-a", key, at + Duration::seconds(300)))
            .await
            .unwrap();
        let fence = match outcome {
            ReservationOutcome::Fresh(lease) => fence_of(&lease),
            other => panic!("setup: expected Fresh for {key}, got {other:?}"),
        };
        store
            .complete(&fence, StoredServiceResponse::new(b"old".to_vec()))
            .await
            .unwrap();
    }

    // Three rows eligible for purge: completed strictly before the cutoff.
    for i in 0..3 {
        complete_at(
            &store,
            &clock,
            &format!("op-purge-eligible-{i}"),
            epoch() + Duration::seconds(i),
        )
        .await;
    }
    // One completed AFTER the cutoff: must survive.
    complete_at(
        &store,
        &clock,
        "op-purge-too-new",
        cutoff + Duration::seconds(10),
    )
    .await;
    // One still in progress, however old: must survive purge entirely.
    clock.advance(Duration::seconds(1));
    store
        .reserve(request(
            "owner-a",
            "op-purge-live",
            clock.now() + Duration::seconds(30),
        ))
        .await
        .unwrap();

    let purged = store.purge_completed_before(cutoff, 10).await.unwrap();
    assert_eq!(
        purged, 3,
        "the two-step select-then-delete workaround must delete every eligible row \
         in one call, not the zero rows Stoolap's single-statement \
         DELETE...WHERE IN (SELECT...LIMIT n) silently returns"
    );

    // A purged key is hard-deleted, not soft-deleted: immediately reusable.
    let recreated = store
        .reserve(request(
            "owner-b",
            "op-purge-eligible-0",
            clock.now() + Duration::seconds(300),
        ))
        .await
        .unwrap();
    assert!(
        matches!(recreated, ReservationOutcome::Fresh(_)),
        "a purged operation_key must be fully removed and immediately reusable, \
         got {recreated:?}"
    );

    // Ineligible rows are untouched.
    let too_new = store
        .reserve(request(
            "probe",
            "op-purge-too-new",
            clock.now() + Duration::seconds(300),
        ))
        .await
        .unwrap();
    assert!(
        matches!(too_new, ReservationOutcome::Succeeded(_)),
        "a reservation completed after the cutoff must survive purge, got {too_new:?}"
    );

    let live = store
        .reserve(request(
            "owner-a",
            "op-purge-live",
            clock.now() + Duration::seconds(30),
        ))
        .await
        .unwrap();
    assert!(
        matches!(live, ReservationOutcome::OwnedInProgress(_)),
        "an in-progress reservation must never be purged, got {live:?}"
    );
}
