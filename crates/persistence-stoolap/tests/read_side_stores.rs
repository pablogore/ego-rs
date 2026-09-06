//! Integration tests for Stoolap-backed read-side stores (STOOLAP-RS-01).
//!
//! PR1 exercised `StoolapOffsetStore`; PR2 (this revision) adds
//! `StoolapDedupStore`. `StoolapReadSideClaimStore` lands in a later PR of
//! the same change and adds its own section to this binary.

use ego_persistence_api::read_side::dedup::DedupStore;
use ego_persistence_api::read_side::event_tag::EventTag;
use ego_persistence_api::read_side::offset::{Offset, OffsetStore};
use ego_persistence_stoolap::{StoolapDedupStore, StoolapOffsetStore};

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
