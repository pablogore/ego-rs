//! The retention worker: off by default, validated, runtime-owned, and stopped.
//!
//! No infrastructure. Everything asserted here — whether a worker starts, the
//! cutoff and batch it purges with, that shutdown ends it, and that an in-progress
//! reservation is untouched by shutdown — is observable through the port with an
//! in-memory store and a test clock. Standing up PostgreSQL would add a container
//! without adding evidence.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use ego_domain::operation::{
    OperationFingerprint, OperationKey, OperationReservationStore, OwnerFence, OwnerId,
    ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse,
};
use ego_service_sdk::runtime::{
    IdempotencyEnforcementMode, RetentionPolicy, RetentionPolicyError, RuntimeBuilder,
};
use ego_testkit::{InMemoryOperationReservationStore, TestClock};

fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

/// Records every purge the worker asks for, and delegates to a real store.
///
/// The cutoff and batch are the whole point: a worker that purges on schedule but
/// computes its cutoff from wall time instead of the configured clock would look
/// correct from the outside and silently disagree with the store that stamped the
/// rows.
struct RecordingStore {
    inner: InMemoryOperationReservationStore,
    purges: Mutex<Vec<(DateTime<Utc>, usize)>>,
    calls: AtomicUsize,
}

impl RecordingStore {
    fn wrapping(inner: InMemoryOperationReservationStore) -> Arc<Self> {
        Arc::new(Self {
            inner,
            purges: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        })
    }
    fn purges(&self) -> Vec<(DateTime<Utc>, usize)> {
        self.purges.lock().expect("not poisoned").clone()
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl OperationReservationStore for RecordingStore {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        self.inner.reserve(req).await
    }
    async fn renew(&self, f: &OwnerFence, until: DateTime<Utc>) -> Result<(), ReservationError> {
        self.inner.renew(f, until).await
    }
    async fn complete(
        &self,
        f: &OwnerFence,
        r: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        self.inner.complete(f, r).await
    }
    async fn abandon(&self, f: &OwnerFence) -> Result<(), ReservationError> {
        self.inner.abandon(f).await
    }
    async fn purge_completed_before(
        &self,
        cutoff: DateTime<Utc>,
        batch: usize,
    ) -> Result<u64, ReservationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.purges
            .lock()
            .expect("not poisoned")
            .push((cutoff, batch));
        self.inner.purge_completed_before(cutoff, batch).await
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        self.inner.probe().await
    }
}

fn request(owner: &str, key: &str, lease_until: DateTime<Utc>) -> ReserveRequest {
    ReserveRequest {
        tenant: None,
        operation_key: OperationKey::parse(key).expect("a valid key"),
        fingerprint: OperationFingerprint::new("f".repeat(64)),
        owner_id: OwnerId::new(owner),
        lease_until,
    }
}

/// Waits for the worker's first pass, or fails.
///
/// The worker purges before waiting, so this is a condition with a deadline rather
/// than a sleep standing in for one.
async fn wait_for_a_purge(store: &RecordingStore) {
    for _ in 0..500 {
        if store.calls() >= 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the retention worker never purged; it was configured and started");
}

// ---------------------------------------------------------------------------
// 1. No policy, no worker
// ---------------------------------------------------------------------------

#[tokio::test]
async fn without_a_policy_no_worker_starts_and_nothing_is_purged() {
    let clock = Arc::new(TestClock::new(epoch()));
    let store = RecordingStore::wrapping(InMemoryOperationReservationStore::new(clock.clone()));

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .build();
    runtime
        .start_retention()
        .await
        .expect("starting is a no-op");

    // Long enough that a worker running on any plausible interval would have acted.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        store.calls(),
        0,
        "retention is opt-in: an SDK upgrade must not begin deleting data on a \
         schedule nobody chose"
    );
}

// ---------------------------------------------------------------------------
// 2. A degenerate policy is refused where it is built
// ---------------------------------------------------------------------------

#[test]
fn a_policy_with_a_zero_value_is_refused() {
    let ok = Duration::from_secs(1);
    assert_eq!(
        RetentionPolicy::new(Duration::ZERO, ok, 1),
        Err(RetentionPolicyError::ZeroRetention)
    );
    assert_eq!(
        RetentionPolicy::new(ok, Duration::ZERO, 1),
        Err(RetentionPolicyError::ZeroInterval)
    );
    assert_eq!(
        RetentionPolicy::new(ok, ok, 0),
        Err(RetentionPolicyError::ZeroBatch)
    );
}

/// A policy with no store to purge cannot mean what it says.
#[test]
#[should_panic(expected = "no OperationReservationStore is registered")]
fn a_policy_without_a_reservation_store_is_refused_at_build() {
    let policy = RetentionPolicy::new(Duration::from_secs(60), Duration::from_secs(1), 10)
        .expect("a valid policy");
    let _ = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_retention_policy(policy)
        .build();
}

// ---------------------------------------------------------------------------
// 3 & 4. It purges with the configured cutoff and batch, and shutdown ends it
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_configured_worker_purges_with_the_exact_cutoff_and_batch_then_stops() {
    let now = epoch() + chrono::Duration::seconds(1_000);
    let clock = Arc::new(TestClock::new(now));
    let store = RecordingStore::wrapping(InMemoryOperationReservationStore::new(clock.clone()));

    let retention = Duration::from_secs(300);
    let policy =
        RetentionPolicy::new(retention, Duration::from_millis(20), 7).expect("a valid policy");

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .build();
    runtime.start_retention().await.expect("the worker starts");

    wait_for_a_purge(&store).await;

    let first = store.purges()[0];
    assert_eq!(
        first,
        (now - chrono::Duration::seconds(300), 7),
        "the cutoff is the runtime's clock minus the retention window, and the \
         batch is the configured one — computed from the same clock the store \
         stamps rows with, not from wall time"
    );

    // Shutdown ends it. The count is taken after shutdown returns and then
    // re-read, so a worker that kept ticking would be caught by the second read
    // rather than by a hopeful single sample.
    runtime.shutdown_async().await.expect("shutdown succeeds");
    let after_shutdown = store.calls();
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(
        store.calls(),
        after_shutdown,
        "no tick happened after shutdown returned: the teardown hook cancelled the \
         loop and waited for it, so a later purge would mean the worker outlived \
         the runtime that owns it"
    );
}

// ---------------------------------------------------------------------------
// 5. Shutdown does not touch a reservation somebody is holding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn shutdown_leaves_an_in_progress_reservation_exactly_as_it_was() {
    let clock = Arc::new(TestClock::new(epoch()));
    let store = RecordingStore::wrapping(InMemoryOperationReservationStore::new(clock.clone()));

    let policy = RetentionPolicy::new(Duration::from_secs(1), Duration::from_millis(20), 10)
        .expect("a valid policy");

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .build();
    runtime.start_retention().await.expect("the worker starts");

    // Somebody holds a reservation, and keeps holding it.
    let outcome = store
        .reserve(request(
            "owner-a",
            "op-held",
            epoch() + chrono::Duration::seconds(600),
        ))
        .await
        .expect("the store answers");
    assert!(
        matches!(outcome, ReservationOutcome::Fresh(_)),
        "setup: expected Fresh, got {outcome:?}"
    );

    wait_for_a_purge(&store).await;
    runtime.shutdown_async().await.expect("shutdown succeeds");

    // Asked through the port: still in progress under its original owner. If
    // shutdown had abandoned, completed or renewed it, this would answer
    // something else — and none of those are a purge worker's business, because it
    // holds no lease of its own.
    let after = store
        .reserve(request(
            "owner-a",
            "op-held",
            epoch() + chrono::Duration::seconds(600),
        ))
        .await
        .expect("the store answers");
    assert!(
        matches!(after, ReservationOutcome::OwnedInProgress(_)),
        "the held reservation must survive shutdown untouched — neither abandoned, \
         completed nor renewed. Got {after:?}"
    );
}

// ---------------------------------------------------------------------------
// 6. A worker that overruns its deadline is cancelled, not detached
// ---------------------------------------------------------------------------

/// A store whose purge never returns until released.
///
/// This is how the timeout branch is reached deterministically: the worker is
/// parked inside `purge_completed_before`, so cancellation cannot be acknowledged
/// through the loop's `select!` and the bounded wait must expire.
struct HangingStore {
    entered: Arc<AtomicUsize>,
    /// Incremented **after** the hang is released, which is the observable that
    /// distinguishes an aborted task from a detached one. A cancelled task is
    /// dropped at its await point and never gets here; a detached one resumes and
    /// does. Counting only entries could not tell them apart — the cancel permit
    /// left by shutdown makes even a detached loop exit at its next `select!`, so
    /// it never enters a second purge either way.
    resumed: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl OperationReservationStore for HangingStore {
    async fn reserve(&self, _r: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        unreachable!("this store only ever purges")
    }
    async fn renew(&self, _f: &OwnerFence, _u: DateTime<Utc>) -> Result<(), ReservationError> {
        unreachable!("shutdown must never renew")
    }
    async fn complete(
        &self,
        _f: &OwnerFence,
        _r: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        unreachable!("shutdown must never complete")
    }
    async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
        unreachable!("shutdown must never abandon")
    }
    async fn purge_completed_before(
        &self,
        _cutoff: DateTime<Utc>,
        _batch: usize,
    ) -> Result<u64, ReservationError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        self.release.notified().await;
        self.resumed.fetch_add(1, Ordering::SeqCst);
        Ok(0)
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        Ok(())
    }
}

#[tokio::test]
async fn a_worker_that_overruns_its_deadline_is_aborted_rather_than_left_running() {
    let clock = Arc::new(TestClock::new(epoch()));
    let entered = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(tokio::sync::Notify::new());
    let resumed = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(HangingStore {
        entered: entered.clone(),
        resumed: resumed.clone(),
        release: release.clone(),
    });

    // The smallest interval the policy admits, so the shutdown deadline derived
    // from it is short and this test does not wait on a long one.
    let policy = RetentionPolicy::new(Duration::from_secs(60), Duration::from_nanos(1), 1)
        .expect("a valid policy");

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .build();
    runtime.start_retention().await.expect("the worker starts");

    // It is inside the purge and will not come out.
    for _ in 0..500 {
        if entered.load(Ordering::SeqCst) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        entered.load(Ordering::SeqCst) >= 1,
        "setup: the worker must be parked inside the purge"
    );

    // Shutdown reports the overrun rather than hiding it.
    let outcome = runtime.shutdown_async().await;
    assert!(
        outcome.is_err(),
        "an overrunning worker must be surfaced, not swallowed: got {outcome:?}"
    );

    // And the worker is genuinely gone, not merely unreferenced. Dropping a
    // `JoinHandle` detaches the task in Tokio rather than cancelling it, so the
    // question is whether the parked purge can still resume. Releasing the hang
    // answers it: a detached task wakes and increments `resumed`; an aborted one
    // was dropped at that await point and never can.
    release.notify_waiters();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        resumed.load(Ordering::SeqCst),
        0,
        "the parked purge resumed after shutdown returned, so the task was detached \
         rather than aborted — shutdown reported a failure while the thing it was \
         shutting down carried on"
    );
}

// ---------------------------------------------------------------------------
// 7. Starting twice starts one worker
// ---------------------------------------------------------------------------

#[tokio::test]
async fn starting_retention_twice_starts_one_worker() {
    let clock = Arc::new(TestClock::new(epoch()));
    let store = RecordingStore::wrapping(InMemoryOperationReservationStore::new(clock.clone()));

    // Long interval: each worker purges once promptly and then waits, so the count
    // after both calls distinguishes one worker from two without racing ticks.
    let policy = RetentionPolicy::new(Duration::from_secs(60), Duration::from_secs(3_600), 5)
        .expect("a valid policy");

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .build();

    runtime
        .start_retention()
        .await
        .expect("the first call starts");
    runtime
        .start_retention()
        .await
        .expect("the second call is a no-op");

    wait_for_a_purge(&store).await;
    tokio::time::sleep(Duration::from_millis(120)).await;

    assert_eq!(
        store.calls(),
        1,
        "one worker, one first pass. Two would mean a second loop purging on the \
         same schedule — and a second teardown hook to stop it, which the first \
         shutdown would not know about"
    );

    runtime.shutdown_async().await.expect("shutdown succeeds");
}
