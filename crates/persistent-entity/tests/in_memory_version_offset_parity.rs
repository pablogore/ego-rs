//! The direct append path and the unit-of-work path must agree about a stream's
//! version, including when a version offset is in play.
//!
//! `InMemoryEventStore` supports declaring that `n` events already existed before
//! the store was created — the shape a test uses to pretend a snapshot covers
//! earlier history. `EventStore::append` adds that offset to the stream length
//! when it decides whether an expected version matches.
//!
//! When the unit of work arrived it did not, and that is what these tests exist to
//! keep from happening again: the same stream, the same argument, one path
//! accepting and the other rejecting. A divergence like that is invisible until a
//! caller switches paths, which is precisely what the following slices do.

use ego_domain::persistence::{EventStore, PersistenceError, StoredEvent};
use persistent_entity::persistence::InMemoryEventStore;
use persistent_entity::testing::TestEvent;

const AGGREGATE_TYPE: &str = "counter";
const AGGREGATE_ID: &str = "offset-parity";
const OFFSET: i64 = 5;

fn store() -> InMemoryEventStore<TestEvent> {
    InMemoryEventStore::<TestEvent>::new().with_version_offset(AGGREGATE_TYPE, AGGREGATE_ID, OFFSET)
}

fn events(count: usize) -> Vec<StoredEvent<TestEvent>> {
    (1..=count as u64)
        .map(|v| StoredEvent::new(TestEvent::Incremented(v)))
        .collect()
}

/// The direct path counts the offset: an empty stream with offset 5 is at version
/// 5, so that is the expected version it accepts.
///
/// Characterizes the behaviour the other path has to match, rather than assuming
/// it. If this ever changes, the parity test below fails for a reason that is
/// visible here first.
#[tokio::test]
async fn the_direct_path_treats_the_offset_as_part_of_the_version() {
    let store = store();

    let rejected = store
        .append(AGGREGATE_TYPE, AGGREGATE_ID, None, 0, events(1))
        .await;
    assert!(
        matches!(
            rejected,
            Err(PersistenceError::Conflict {
                expected: 0,
                actual: OFFSET,
                ..
            })
        ),
        "expected version 0 must be rejected against an offset of {OFFSET}, got {rejected:?}"
    );

    let accepted = store
        .append(AGGREGATE_TYPE, AGGREGATE_ID, None, OFFSET, events(1))
        .await
        .expect("expected version 5 must be accepted against an offset of 5");
    assert_eq!(
        accepted,
        OFFSET + 1,
        "the returned version must include the offset"
    );
}

/// The unit-of-work path agrees, on the same stream with the same argument.
///
/// This is the assertion the reviewed implementation failed: it computed
/// `committed + staged` and reported `actual: 0`, rejecting an append the direct
/// path accepts.
#[tokio::test]
async fn the_unit_of_work_path_treats_the_offset_as_part_of_the_version() {
    let store = store();

    let mut uow = store.begin().await.expect("beginning must succeed");

    let rejected = uow
        .append(AGGREGATE_TYPE, AGGREGATE_ID, None, 0, events(1))
        .await;
    assert!(
        matches!(
            rejected,
            Err(PersistenceError::Conflict {
                expected: 0,
                actual: OFFSET,
                ..
            })
        ),
        "the unit of work must report the offset as the current version, got {rejected:?}"
    );

    let accepted = uow
        .append(AGGREGATE_TYPE, AGGREGATE_ID, None, OFFSET, events(1))
        .await
        .expect("expected version 5 must be accepted against an offset of 5");
    assert_eq!(
        accepted,
        OFFSET + 1,
        "the returned version must include the offset"
    );

    uow.commit().await.expect("committing must succeed");

    // And the committed result is what the direct path would then read: the offset
    // is a property of the stream, not of the path that wrote to it.
    let store_again = store;
    let next = store_again
        .append(AGGREGATE_TYPE, AGGREGATE_ID, None, OFFSET + 1, events(1))
        .await
        .expect("the direct path must continue from where the unit of work left off");
    assert_eq!(next, OFFSET + 2);
}

/// Both paths reject and accept the identical expected versions, asserted by
/// comparing their outcomes rather than by restating the numbers.
///
/// The two tests above each pin one path against literal values, which would let
/// both drift together if someone changed the offset semantics in both places.
/// This one compares the paths to each other, so it fails when they disagree
/// regardless of what either one decides the version is.
#[tokio::test]
async fn the_two_paths_agree_on_every_expected_version_around_the_offset() {
    for expected in 0..=(OFFSET + 1) {
        let direct = store();
        let direct_outcome = direct
            .append(AGGREGATE_TYPE, AGGREGATE_ID, None, expected, events(1))
            .await
            .is_ok();

        let staged = store();
        let mut uow = staged.begin().await.expect("beginning must succeed");
        let uow_outcome = uow
            .append(AGGREGATE_TYPE, AGGREGATE_ID, None, expected, events(1))
            .await
            .is_ok();

        assert_eq!(
            direct_outcome, uow_outcome,
            "the two paths disagreed at expected version {expected}: direct accepted = \
             {direct_outcome}, unit of work accepted = {uow_outcome}"
        );
    }
}

/// A stream with no declared offset behaves the same through both paths.
///
/// Guards against a fix that reads the offset map for the wrong key, or that
/// defaults to something other than zero when no offset was declared — both of
/// which would pass the tests above while breaking every ordinary stream.
#[tokio::test]
async fn a_stream_without_an_offset_starts_at_zero_through_both_paths() {
    let direct = store();
    let accepted = direct
        .append(AGGREGATE_TYPE, "no-offset-declared", None, 0, events(1))
        .await
        .expect("an undeclared stream starts at version 0 on the direct path");
    assert_eq!(accepted, 1);

    let staged = store();
    let mut uow = staged.begin().await.expect("beginning must succeed");
    let accepted = uow
        .append(AGGREGATE_TYPE, "no-offset-declared", None, 0, events(1))
        .await
        .expect("an undeclared stream starts at version 0 in a unit of work too");
    assert_eq!(accepted, 1);
}
