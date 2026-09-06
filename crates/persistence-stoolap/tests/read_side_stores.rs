//! Integration tests for Stoolap-backed read-side stores (STOOLAP-RS-01).
//!
//! PR1 exercised `StoolapOffsetStore`; PR2 added `StoolapDedupStore`; PR4
//! (this revision) adds `StoolapReadSideClaimStore`'s concurrency race,
//! reopen-durability, and shared-engine sections. `StoolapReadSideClaimStore`
//! itself, and its colocated unit tests, shipped in PR3
//! (`src/read_side/claim.rs`).

use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};

use ego_domain::operation::OwnerId;
use ego_domain::Clock;
use ego_persistence_api::read_side::claim::{ClaimError, ClaimId, ReadSideClaimStore};
use ego_persistence_api::read_side::dedup::DedupStore;
use ego_persistence_api::read_side::event_tag::EventTag;
use ego_persistence_api::read_side::offset::{Offset, OffsetStore};
use ego_persistence_stoolap::{StoolapDedupStore, StoolapOffsetStore, StoolapReadSideClaimStore};
use ego_testkit::TestClock;

/// The fixed instant every claim scenario below starts from — mirrors
/// `src/read_side/claim.rs`'s colocated `epoch()` (private to that module,
/// so duplicated here for this integration binary, the same precedent
/// `tests/reservation_conformance.rs::epoch` sets).
fn epoch() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn claim_id(tenant: &str) -> ClaimId {
    ClaimId {
        projection_id: "proj".to_string(),
        tag: EventTag::new("users-by-tenant"),
        tenant: tenant.to_string(),
    }
}

/// spec `persistence-stoolap-read-side`: "Offset And Dedup State Survive
/// Close And Reopen" — "An offset survives a close/reopen cycle".
///
/// This proves **drop-and-reopen** durability only: the same OS process
/// drops every handle to the file, then reopens it. It proves nothing about
/// crash recovery, `kill -9`, or power loss (design.md "Restart contract" /
/// spec Non-Goals).
#[tokio::test]
async fn offset_survives_close_and_reopen_drop_reopen_only_not_crash_safety() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let tag = EventTag::new("users-by-tenant");

    let store = StoolapOffsetStore::open(&dir)
        .await
        .expect("open StoolapOffsetStore");
    store
        .write_offset("proj", &tag, "tenant-a", &Offset::sequence(42))
        .await
        .unwrap();

    // Close: the only handle to this DSN is dropped, so Stoolap's
    // process-global registry has nothing left to keep the engine alive
    // (design.md "Concurrency Scope"). A surviving handle would make this
    // test prove nothing.
    drop(store);

    let reopened = StoolapOffsetStore::open(&dir)
        .await
        .expect("reopen StoolapOffsetStore at the same path");

    assert_eq!(
        reopened
            .read_offset("proj", &tag, "tenant-a")
            .await
            .unwrap(),
        Some(Offset::sequence(42))
    );
    // A never-written sibling key stays absent: the reopened state is the
    // persisted rows, not a rebuilt empty table.
    assert_eq!(
        reopened
            .read_offset("proj", &tag, "tenant-b")
            .await
            .unwrap(),
        None
    );
    assert!(reopened.is_durable());
}

/// spec `persistence-stoolap-read-side`: "Offset And Dedup State Survive
/// Close And Reopen" — "A dedup mark survives a close/reopen cycle".
///
/// This proves **drop-and-reopen** durability only: the same OS process
/// drops every handle to the file, then reopens it. It proves nothing about
/// crash recovery, `kill -9`, or power loss (design.md "Restart contract" /
/// spec Non-Goals).
#[tokio::test]
async fn dedup_survives_close_and_reopen_drop_reopen_only_not_crash_safety() {
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let tag = EventTag::new("users-by-tenant");

    let store = StoolapDedupStore::open(&dir)
        .await
        .expect("open StoolapDedupStore");
    store.mark_seen("proj", &tag, "evt-1").await.unwrap();

    // Close: the only handle to this DSN is dropped, so Stoolap's
    // process-global registry has nothing left to keep the engine alive
    // (design.md "Concurrency Scope"). A surviving handle would make this
    // test prove nothing.
    drop(store);

    let reopened = StoolapDedupStore::open(&dir)
        .await
        .expect("reopen StoolapDedupStore at the same path");

    assert!(reopened.seen("proj", &tag, "evt-1").await.unwrap());
    // A never-marked sibling triple stays absent: the reopened state is the
    // persisted rows, not a rebuilt empty table.
    assert!(!reopened.seen("proj", &tag, "evt-2").await.unwrap());
    assert!(reopened.is_durable());
}

/// spec `persistence-stoolap-read-side`: "Claim Correctness Holds Under Real
/// Intra-Process Concurrency" — "Concurrent claimants yield exactly one
/// winner". design.md AD-5 / AD-8.
///
/// Several real concurrent tasks — not a sequential simulation — race
/// `try_claim` on one fresh `claim_id` with no existing live lease. Exactly
/// one may be granted `Ok(Some(fence))`. Every other must observe `Ok(None)`
/// or a retry-safe `ClaimError::Transient` (classified via
/// `stoolap_common::is_write_conflict`, AD-8) — never a second grant. A
/// `Transient` outcome here is an expected, correct result of a genuine
/// write-conflict race, not a bug.
///
/// Mirrors `tests/reservation_conformance.rs:192-240`'s
/// `two_concurrent_reserves_for_the_same_key_grant_exactly_one_fresh_reservation`.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_claimants_yield_exactly_one_winner() {
    let dir = tempfile::tempdir().expect("tempdir");
    let clock = Arc::new(TestClock::new(epoch()));
    let store = Arc::new(
        StoolapReadSideClaimStore::open(dir.path(), clock.clone())
            .await
            .expect("open StoolapReadSideClaimStore"),
    );
    let id = claim_id("tenant-concurrent-fresh");
    let lease_until = epoch() + Duration::seconds(30);

    let mut tasks = Vec::new();
    for i in 0..5 {
        let store = Arc::clone(&store);
        let id = id.clone();
        let owner = OwnerId::new(format!("owner-{i}"));
        tasks.push(tokio::spawn(async move {
            store.try_claim(&id, &owner, lease_until).await
        }));
    }

    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.expect("spawned claimant must not panic"));
    }

    let granted = outcomes.iter().filter(|r| matches!(r, Ok(Some(_)))).count();
    assert_eq!(
        granted, 1,
        "exactly one concurrent claimant must be granted Ok(Some(fence)); got {outcomes:?}"
    );

    for outcome in &outcomes {
        match outcome {
            Ok(Some(_)) | Ok(None) => {}
            Err(ClaimError::Transient(_)) => {}
            other => panic!(
                "every losing claimant must observe Ok(None) or a retry-safe \
                 ClaimError::Transient — never a second grant or any other \
                 outcome; got {other:?}"
            ),
        }
    }
}

/// spec `persistence-stoolap-read-side`: "Claim Grant And Refusal Are
/// Mutually Exclusive" / "Claim Correctness Holds Under Real Intra-Process
/// Concurrency". design.md AD-5's "why no lost update is possible".
///
/// Two real concurrent tasks race a takeover attempt against one `claim_id`
/// whose lease has already lapsed (`ego_testkit::TestClock` is advanced past
/// the original lease before the racing attempts are spawned). The loser
/// must resolve to `Ok(None)` (a peer won the takeover, or the original
/// owner renewed inside the race window) or a retry-safe `Transient` — never
/// a third outcome, and never two simultaneous grants.
#[tokio::test(flavor = "multi_thread")]
async fn takeover_after_expiration_under_real_concurrency_mints_a_strictly_greater_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    let clock = Arc::new(TestClock::new(epoch()));
    let store = Arc::new(
        StoolapReadSideClaimStore::open(dir.path(), clock.clone())
            .await
            .expect("open StoolapReadSideClaimStore"),
    );
    let id = claim_id("tenant-concurrent-takeover");
    let owner_original = OwnerId::new("owner-original");

    let original = store
        .try_claim(&id, &owner_original, epoch() + Duration::seconds(30))
        .await
        .unwrap()
        .expect("setup: fresh grant must succeed");

    // Advance past the original lease so the takeover race below is racing
    // over a genuinely lapsed lease, not a live one.
    clock.advance(Duration::seconds(31));
    let takeover_lease_until = clock.now() + Duration::seconds(30);

    let mut tasks = Vec::new();
    for i in 0..3 {
        let store = Arc::clone(&store);
        let id = id.clone();
        let owner = OwnerId::new(format!("owner-takeover-{i}"));
        tasks.push(tokio::spawn(async move {
            store.try_claim(&id, &owner, takeover_lease_until).await
        }));
    }

    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.expect("spawned takeover racer must not panic"));
    }

    let granted: Vec<_> = outcomes
        .iter()
        .filter_map(|r| match r {
            Ok(Some(fence)) => Some(fence),
            _ => None,
        })
        .collect();
    assert_eq!(
        granted.len(),
        1,
        "exactly one concurrent takeover racer may be granted; got {outcomes:?}"
    );
    assert!(
        granted[0].fencing_token > original.fencing_token,
        "the winning takeover must mint a strictly greater fencing token"
    );

    for outcome in &outcomes {
        match outcome {
            Ok(Some(_)) | Ok(None) => {}
            Err(ClaimError::Transient(_)) => {}
            other => panic!(
                "every losing racer must observe Ok(None) or a retry-safe \
                 ClaimError::Transient — never a third outcome; got {other:?}"
            ),
        }
    }
}

/// spec `persistence-stoolap-read-side`: "Claim Durability Is
/// Drop-And-Reopen, Not Crash Recovery".
///
/// This proves **drop-and-reopen** durability only: the same OS process
/// drops every handle to the file, then reopens it. It proves nothing about
/// crash recovery, `kill -9`, or power loss (design.md "Restart contract" /
/// spec Non-Goals) — this test is NOT a crash-safety or power-loss-safety
/// test.
///
/// Two independent claims are exercised at the same path: one held under a
/// still-valid fence at the moment of the drop, and one released before the
/// drop. After reopen: the held claim's lease and fence survive (a different
/// owner's `try_claim` still refuses, and the original fence still verifies
/// through `renew`); the released claim reopens as immediately reclaimable,
/// and the next takeover mints a strictly greater fencing token than the one
/// that was released — proving the fencing sequence itself, not just the
/// lease state, survived reopen.
#[tokio::test]
async fn claim_state_survives_close_and_reopen() {
    let dir = tempfile::tempdir().expect("tempdir").keep();

    let clock = Arc::new(TestClock::new(epoch()));
    let store = StoolapReadSideClaimStore::open(&dir, clock.clone())
        .await
        .expect("open StoolapReadSideClaimStore");

    let id_held = claim_id("tenant-held");
    let owner_a = OwnerId::new("owner-a");
    let held_fence = store
        .try_claim(&id_held, &owner_a, epoch() + Duration::seconds(30))
        .await
        .unwrap()
        .expect("setup: fresh grant for the held claim");

    let id_released = claim_id("tenant-released");
    let owner_x = OwnerId::new("owner-x");
    let released_fence = store
        .try_claim(&id_released, &owner_x, epoch() + Duration::seconds(30))
        .await
        .unwrap()
        .expect("setup: fresh grant for the released claim");
    store
        .release(&released_fence)
        .await
        .expect("setup: release the second claim before the drop");

    // Close: the only handle to this DSN is dropped, so Stoolap's
    // process-global registry has nothing left to keep the engine alive
    // (design.md "Concurrency Scope"). A surviving handle would make this
    // test prove nothing.
    drop(store);

    // Reopen at the same path with a fresh clock pinned to the identical
    // epoch, so "still live" / "already lapsed" below is judged against the
    // same instant the original lease and release were computed from.
    let reopened_clock = Arc::new(TestClock::new(epoch()));
    let reopened = StoolapReadSideClaimStore::open(&dir, reopened_clock.clone())
        .await
        .expect("reopen StoolapReadSideClaimStore at the same path");

    // The held claim's lease survived reopen: a different owner is refused.
    let owner_b = OwnerId::new("owner-b");
    let refusal = reopened
        .try_claim(&id_held, &owner_b, epoch() + Duration::seconds(30))
        .await
        .unwrap();
    assert!(
        refusal.is_none(),
        "the reopened store must still refuse a live lease it did not create"
    );
    // The original fence still verifies through `renew` against the
    // reopened state.
    reopened
        .renew(&held_fence, epoch() + Duration::seconds(60))
        .await
        .expect("the fence held before the drop must still verify after reopen");

    // The released claim reopens as immediately reclaimable, and the next
    // takeover mints a strictly greater token than the one released —
    // proving the fencing sequence, not just the lease state, survived.
    let owner_y = OwnerId::new("owner-y");
    let reclaimed = reopened
        .try_claim(&id_released, &owner_y, epoch() + Duration::seconds(30))
        .await
        .unwrap()
        .expect("a claim released before the drop must reopen as immediately reclaimable");
    assert!(
        reclaimed.fencing_token > released_fence.fencing_token,
        "the fencing sequence itself must survive reopen: the next takeover's \
         token must be strictly greater than the released one"
    );
}

/// design.md "Concurrency Scope": "One process, three store instances at one
/// path (the production composition) — Yes ... Re-proved here rather than
/// assumed." Mirrors `tests/reservation_conformance.rs:258-311`'s
/// `two_live_store_instances_at_one_path_share_the_same_engine`.
///
/// An offset, a dedup, and a claim store are opened at one identical
/// filesystem path. A write through each typed store is proven visible to a
/// fourth, independently opened raw `stoolap::Database` handle at the exact
/// same DSN — without closing any of the three typed stores first — showing
/// all four handles observe one shared, live, process-global Stoolap engine
/// for that DSN rather than four independent databases.
#[tokio::test]
async fn three_stores_at_one_path_share_one_engine() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tag = EventTag::new("users-by-tenant");

    let offset_store = StoolapOffsetStore::open(dir.path())
        .await
        .expect("open StoolapOffsetStore");
    let dedup_store = StoolapDedupStore::open(dir.path())
        .await
        .expect("open StoolapDedupStore");
    let clock = Arc::new(TestClock::new(epoch()));
    let claim_store = StoolapReadSideClaimStore::open(dir.path(), clock.clone())
        .await
        .expect("open StoolapReadSideClaimStore");

    offset_store
        .write_offset("proj", &tag, "tenant-shared", &Offset::sequence(99))
        .await
        .unwrap();
    dedup_store
        .mark_seen("proj", &tag, "evt-shared")
        .await
        .unwrap();
    let id = claim_id("tenant-shared");
    claim_store
        .try_claim(
            &id,
            &OwnerId::new("owner-shared"),
            epoch() + Duration::seconds(30),
        )
        .await
        .unwrap()
        .expect("fresh grant on the claim table");

    // A fourth, independently opened raw handle at the identical DSN
    // (`file://{path}?sync=full`, the exact construction
    // `stoolap_common::dsn_for` uses — `pub(crate)` and so replicated here
    // only to build this out-of-band, read-only proof handle, not to
    // duplicate any store logic). All three typed stores above are still
    // alive, so if this raw handle observes their writes without any of
    // them being closed or reopened, all four share one live engine.
    let dsn = format!("file://{}?sync=full", dir.path().display());
    let raw = stoolap::Database::open(&dsn).expect("open a fourth raw handle at the same DSN");

    let mut offset_rows = raw
        .query(
            "SELECT offset_value FROM projection_offsets
             WHERE projection_id = $1 AND tag = $2 AND tenant = $3",
            (
                "proj".to_string(),
                tag.value().to_string(),
                "tenant-shared".to_string(),
            ),
        )
        .expect("query projection_offsets through the raw shared-engine handle");
    let offset_row = offset_rows
        .next()
        .expect("the offset write must be visible through the raw handle")
        .expect("row read");
    let offset_value: i64 = offset_row.get(0).expect("offset_value column");
    assert_eq!(offset_value, 99);

    let mut dedup_rows = raw
        .query(
            "SELECT 1 FROM projection_dedup
             WHERE projection_id = $1 AND tag = $2 AND event_id = $3",
            (
                "proj".to_string(),
                tag.value().to_string(),
                "evt-shared".to_string(),
            ),
        )
        .expect("query projection_dedup through the raw shared-engine handle");
    assert!(
        dedup_rows.next().is_some(),
        "the dedup mark must be visible through the raw handle"
    );

    let mut claim_rows = raw
        .query(
            "SELECT owner_id FROM projection_claims
             WHERE projection_id = $1 AND tag = $2 AND tenant = $3",
            (
                "proj".to_string(),
                tag.value().to_string(),
                "tenant-shared".to_string(),
            ),
        )
        .expect("query projection_claims through the raw shared-engine handle");
    let claim_row = claim_rows
        .next()
        .expect("the claim grant must be visible through the raw handle")
        .expect("row read");
    let owner: String = claim_row.get(0).expect("owner_id column");
    assert_eq!(owner, "owner-shared");
}
