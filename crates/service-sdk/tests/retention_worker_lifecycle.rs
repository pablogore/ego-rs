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

// ---------------------------------------------------------------------------
// AD-10: idempotency.purge_batch
// ---------------------------------------------------------------------------

/// Records span names, their parents, and how each closed.
struct SpanRecordingTracer {
    started: Mutex<Vec<(String, Option<ego_domain::SpanId>, ego_domain::SpanId)>>,
    ended: Mutex<Vec<(ego_domain::SpanId, ego_domain::SpanOutcome)>>,
}

impl SpanRecordingTracer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            started: Mutex::new(Vec::new()),
            ended: Mutex::new(Vec::new()),
        })
    }

    fn started(&self) -> Vec<(String, Option<ego_domain::SpanId>, ego_domain::SpanId)> {
        self.started.lock().expect("not poisoned").clone()
    }

    fn ended(&self) -> Vec<(ego_domain::SpanId, ego_domain::SpanOutcome)> {
        self.ended.lock().expect("not poisoned").clone()
    }
}

impl ego_domain::Tracer for SpanRecordingTracer {
    fn start_span(
        &self,
        ctx: &ego_domain::TraceContext,
        name: &str,
        _attrs: ego_domain::SpanAttributes,
    ) {
        self.started.lock().expect("not poisoned").push((
            name.to_string(),
            ctx.parent_span_id(),
            ctx.span_id(),
        ));
    }
    fn end_span(&self, span: ego_domain::SpanId, outcome: ego_domain::SpanOutcome) {
        self.ended
            .lock()
            .expect("not poisoned")
            .push((span, outcome));
    }
}

/// A store that fails every purge, so the error classification has something to
/// classify.
struct FailingStore;

#[async_trait]
impl OperationReservationStore for FailingStore {
    async fn reserve(&self, _r: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        unreachable!("the retention worker only purges")
    }
    async fn renew(&self, _f: &OwnerFence, _u: DateTime<Utc>) -> Result<(), ReservationError> {
        unreachable!("the retention worker only purges")
    }
    async fn complete(
        &self,
        _f: &OwnerFence,
        _r: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        unreachable!("the retention worker only purges")
    }
    async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
        unreachable!("the retention worker only purges")
    }
    async fn purge_completed_before(
        &self,
        _c: DateTime<Utc>,
        _b: usize,
    ) -> Result<u64, ReservationError> {
        Err(ReservationError::Backend("boom".to_string()))
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        Ok(())
    }
}

/// A store whose purge announces entry and then never returns.
struct ParkingStore {
    entered: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl OperationReservationStore for ParkingStore {
    async fn reserve(&self, _r: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        unreachable!("the retention worker only purges")
    }
    async fn renew(&self, _f: &OwnerFence, _u: DateTime<Utc>) -> Result<(), ReservationError> {
        unreachable!("the retention worker only purges")
    }
    async fn complete(
        &self,
        _f: &OwnerFence,
        _r: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        unreachable!("the retention worker only purges")
    }
    async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
        unreachable!("the retention worker only purges")
    }
    async fn purge_completed_before(
        &self,
        _c: DateTime<Utc>,
        _b: usize,
    ) -> Result<u64, ReservationError> {
        // `notify_one`, not `notify_waiters`: the latter only wakes waiters already
        // registered, so a signal fired before the test polls its `Notified` is lost
        // forever. `notify_one` leaves a permit behind, which makes the handshake
        // independent of who gets scheduled first.
        self.entered.notify_one();
        std::future::pending().await
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        Ok(())
    }
}

fn traced_runtime(
    store: Arc<dyn OperationReservationStore>,
    clock: Arc<TestClock>,
    interval: Duration,
    tracer: Arc<SpanRecordingTracer>,
) -> ego_service_sdk::runtime::Runtime {
    let policy =
        RetentionPolicy::new(Duration::from_secs(300), interval, 7).expect("a valid policy");
    RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store)
        .with_reservation_clock(clock)
        .with_retention_policy(policy)
        .with_tracer(tracer as Arc<dyn ego_domain::Tracer>)
        .build()
}

async fn wait_for_a_span(tracer: &SpanRecordingTracer) {
    for _ in 0..500 {
        if !tracer.started().is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the worker never opened a span; it was configured, traced and started");
}

/// Every tick opens `idempotency.purge_batch` as a **root** span, and a successful
/// purge closes it `Ok`.
///
/// Root is the assertion carrying AD-10's reasoning: a background tick has no
/// request boundary to descend from, and a parent here would attribute shared batch
/// work to one caller's request.
///
/// `>= 1` rather than an exact count, because the worker ticks on its interval and
/// pinning a number would make this a race against the scheduler. What is pinned is
/// that ticks are traced, that each is rooted, and that each one closes.
#[tokio::test]
async fn every_purge_tick_opens_a_root_span_and_closes_it_ok() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let store = RecordingStore::wrapping(InMemoryOperationReservationStore::new(clock.clone()));
    let tracer = SpanRecordingTracer::new();

    let runtime = traced_runtime(
        store.clone(),
        clock.clone(),
        Duration::from_millis(20),
        tracer.clone(),
    );
    runtime.start_retention().await.expect("the worker starts");
    wait_for_a_purge(&store).await;
    wait_for_a_span(&tracer).await;
    let shutdown = runtime.shutdown_async().await;
    assert!(shutdown.is_ok(), "clean shutdown: {shutdown:?}");

    let started = tracer.started();
    assert!(!started.is_empty(), "a traced worker must report its ticks");
    for (name, parent, _) in &started {
        assert_eq!(name, "idempotency.purge_batch");
        assert_eq!(
            *parent, None,
            "a background tick has no request span to descend from, so its span must \
             be a root"
        );
    }

    let ended = tracer.ended();
    assert_eq!(
        ended.len(),
        started.len(),
        "every opened span must be closed, or the adapter's bounded table leaks one \
         per tick"
    );
    for (_, outcome) in &ended {
        assert_eq!(
            *outcome,
            ego_domain::SpanOutcome::Ok,
            "a purge that succeeded is not a failed span"
        );
    }
}

/// A purge the store refuses closes its span `Error`.
///
/// The other half of the classification, and the first signal a failing purge
/// produces at all: the loop discarded the `Result` outright before this.
///
/// Scoped precisely, because the span is the only reporter: this holds **when a
/// tracer is configured**. An untraced runtime still discards the failure and carries
/// on, unchanged. Closing that gap is a metric's job, and metrics are B7.10/B7.11's
/// remaining work.
#[tokio::test]
async fn a_failing_purge_closes_its_span_as_an_error() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let tracer = SpanRecordingTracer::new();

    let runtime = traced_runtime(
        Arc::new(FailingStore),
        clock.clone(),
        Duration::from_millis(20),
        tracer.clone(),
    );
    runtime.start_retention().await.expect("the worker starts");
    wait_for_a_span(&tracer).await;
    // Let the first tick close before reading.
    for _ in 0..500 {
        if !tracer.ended().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let _ = runtime.shutdown_async().await;

    let ended = tracer.ended();
    assert!(
        !ended.is_empty(),
        "a failing purge must still close its span"
    );
    for (_, outcome) in &ended {
        match outcome {
            ego_domain::SpanOutcome::Error { status_message } => assert!(
                !status_message.is_empty(),
                "a failed purge needs a message an operator can read"
            ),
            ego_domain::SpanOutcome::Ok => {
                panic!("a purge the store refused is not a success")
            }
        }
    }
}

/// A shutdown that cancels an in-flight purge still closes the span, exactly once,
/// as an error.
///
/// This is not a hypothetical: `RetentionWorker::stop` aborts the task when a purge
/// overruns the shutdown deadline, which drops the future mid-`await`. Without the
/// guard every such shutdown would leak an entry in the adapter's bounded table —
/// and that table drops *new* spans at capacity rather than evicting, so repeated
/// restarts under a slow database would quietly end tracing.
///
/// Construction: the store announces entry and then pends, so the worker is parked
/// at exactly the cancellation point. The span is asserted open and not yet closed
/// before shutdown, so the single close afterwards can only be the guard's.
#[tokio::test]
async fn a_shutdown_that_cancels_an_in_flight_purge_closes_the_span_once_as_an_error() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let entered = Arc::new(tokio::sync::Notify::new());
    let tracer = SpanRecordingTracer::new();

    let runtime = traced_runtime(
        Arc::new(ParkingStore {
            entered: entered.clone(),
        }),
        clock.clone(),
        Duration::from_millis(20),
        tracer.clone(),
    );
    runtime.start_retention().await.expect("the worker starts");

    // Wait for the store's own signal, not merely for the span. The span is opened
    // *before* the store is called, so `wait_for_a_span` alone would let this proceed
    // while the worker had not yet reached the `.await` — and a later edit adding any
    // yield in between would make that the common case rather than a latent one. The
    // property under test is a drop *at* the cancellation point, so the handshake has
    // to be with the code that owns it.
    entered.notified().await;

    let started = tracer.started();
    assert_eq!(started.len(), 1, "one parked tick, got {started:?}");
    assert!(
        tracer.ended().is_empty(),
        "the span must still be open while the purge is parked, or this test would \
         be about the normal path"
    );

    // The cancellation: the parked purge cannot finish, so the bounded wait in
    // `stop` expires and the task is aborted — dropping the future.
    let _ = runtime.shutdown_async().await;

    let ended = tracer.ended();
    assert_eq!(
        ended.len(),
        1,
        "a cancelled purge must close its span exactly once, got {ended:?}"
    );
    assert_eq!(
        ended[0].0, started[0].2,
        "the closed span must be the one that was opened"
    );
    match &ended[0].1 {
        ego_domain::SpanOutcome::Error { status_message } => assert!(
            !status_message.is_empty(),
            "an abandoned purge needs a message naming what happened"
        ),
        ego_domain::SpanOutcome::Ok => panic!(
            "an abandoned purge never learned its result, so Ok would claim something \
             nobody observed"
        ),
    }
}

/// A runtime with no tracer purges exactly as before.
///
/// The negative control, and the configuration most deployments run: instrumentation
/// must not become a precondition for retention.
#[tokio::test]
async fn an_untraced_worker_still_purges() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let store = RecordingStore::wrapping(InMemoryOperationReservationStore::new(clock.clone()));

    let policy = RetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .build();
    runtime.start_retention().await.expect("the worker starts");

    wait_for_a_purge(&store).await;
    assert!(store.calls() >= 1, "the purge happens without tracing");
    let _ = runtime.shutdown_async().await;
}
