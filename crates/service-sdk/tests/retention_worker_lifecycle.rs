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
    OldestCompleted, OperationFingerprint, OperationKey, OperationReservationStore, OwnerFence,
    OwnerId, ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse,
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
    /// The purge count observed at each `oldest_completed` call, which is what
    /// makes "queried after the purge" checkable rather than assumed.
    oldest_queries: Mutex<Vec<usize>>,
}

impl RecordingStore {
    fn wrapping(inner: InMemoryOperationReservationStore) -> Arc<Self> {
        Arc::new(Self {
            inner,
            purges: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            oldest_queries: Mutex::new(Vec::new()),
        })
    }
    fn purges(&self) -> Vec<(DateTime<Utc>, usize)> {
        self.purges.lock().expect("not poisoned").clone()
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    /// How many purges had completed at each `oldest_completed` call.
    fn oldest_queries(&self) -> Vec<usize> {
        self.oldest_queries.lock().expect("not poisoned").clone()
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
    /// Delegated, never inherited.
    ///
    /// The port's default answers `Unsupported`, and taking it here would hide a
    /// capability the inner store really has. Every gauge assertion in this file
    /// would then pass while proving nothing: the worker would correctly emit no
    /// sample, and the test would correctly observe none.
    ///
    /// The call is also counted, so the "queried after the purge" ordering is a
    /// recorded fact rather than an inference from the value.
    async fn oldest_completed(&self) -> Result<OldestCompleted, ReservationError> {
        self.oldest_queries
            .lock()
            .expect("not poisoned")
            .push(self.calls.load(Ordering::SeqCst));
        self.inner.oldest_completed().await
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

// ---------------------------------------------------------------------------
// AD-10: the two purge metrics
// ---------------------------------------------------------------------------

/// Records every `metric` call in order, as `(name, value)`.
///
/// The value matters as much as the name here: `idempotency.purge.rows` is a number an
/// operator sizes a batch from, so a counter that always said one — or zero — would be
/// worse than none while passing any name-only assertion.
#[derive(Default)]
struct RecordingObservability {
    metrics: Mutex<Vec<RecordedMetric>>,
}

use ego_testkit::RecordedMetric;

impl RecordingObservability {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn metrics(&self) -> Vec<(String, f64)> {
        self.metrics
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|m| (m.name.clone(), m.value))
            .collect()
    }
    fn names(&self) -> Vec<String> {
        self.metrics().into_iter().map(|(n, _)| n).collect()
    }
    /// Whole records, so kind and attributes are compared and not only the value.
    fn records_of(&self, name: &str) -> Vec<RecordedMetric> {
        self.metrics
            .lock()
            .expect("not poisoned")
            .iter()
            .filter(|m| m.name == name)
            .cloned()
            .collect()
    }
    fn values_of(&self, name: &str) -> Vec<f64> {
        self.metrics()
            .into_iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v)
            .collect()
    }
}

impl ego_domain::Observability for RecordingObservability {
    fn trace(&self, _e: ego_domain::SemanticEvent) {}
    fn record_metric(&self, observation: ego_domain::MetricObservation<'_>) {
        self.metrics
            .lock()
            .expect("not poisoned")
            .push(RecordedMetric::capture(&observation));
    }
    fn log(&self, _l: ego_domain::Level, _m: &str) {}
}

/// This file's double preserves every field of what it is handed.
#[test]
fn the_double_preserves_metric_observations() {
    let obs = RecordingObservability::new();
    ego_testkit::assert_metric_observations_are_preserved(obs.as_ref(), || {
        obs.metrics.lock().expect("not poisoned").clone()
    });
}

async fn wait_for_a_metric(obs: &RecordingObservability, name: &str) {
    for _ in 0..500 {
        if obs.names().iter().any(|n| n == name) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the worker never emitted {name}; it was configured, instrumented and started");
}

/// Stages `count` completed reservations, all eligible for purging.
async fn stage_eligible(
    inner: &InMemoryOperationReservationStore,
    clock: &TestClock,
    count: usize,
) {
    use ego_domain::Clock as _;
    let now = clock.now();
    for i in 0..count {
        let outcome = inner
            .reserve(ReserveRequest {
                tenant: None,
                operation_key: OperationKey::parse(format!("op-{i}")).expect("valid"),
                fingerprint: OperationFingerprint::new("fp"),
                owner_id: OwnerId::new("seeder"),
                lease_until: now + chrono::Duration::seconds(30),
            })
            .await
            .expect("a fresh reservation");
        let lease = match outcome {
            ReservationOutcome::Fresh(lease) => lease,
            other => panic!("expected Fresh, got {other:?}"),
        };
        inner
            .complete(
                &OwnerFence {
                    operation_id: lease.operation_id.clone(),
                    owner_id: lease.owner_id.clone(),
                    fencing_token: lease.fencing_token,
                },
                StoredServiceResponse::new(b"done".to_vec()),
            )
            .await
            .expect("completion");
    }
    // Past the retention window, so every staged row is eligible.
    clock.advance(chrono::Duration::seconds(3_600));
}

/// A successful tick counts the rows it actually removed, and how long it took.
///
/// Three rows are staged so the expected value is not the degenerate zero or one that
/// a hard-coded counter would also produce.
#[tokio::test]
async fn a_successful_tick_counts_the_rows_it_removed_and_its_duration() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let inner = InMemoryOperationReservationStore::new(clock.clone());
    stage_eligible(&inner, &clock, 3).await;

    let store = RecordingStore::wrapping(inner);
    let obs = RecordingObservability::new();
    let policy = RetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .with_observability(obs.clone() as Arc<dyn ego_domain::Observability>)
        .build();
    runtime.start_retention().await.expect("the worker starts");

    wait_for_a_metric(&obs, "idempotency.purge.rows").await;
    let _ = runtime.shutdown_async().await;

    let rows = obs.values_of("idempotency.purge.rows");
    assert_eq!(
        rows.first().copied(),
        Some(3.0),
        "the first tick removed all three eligible reservations, so the counter must \
         say three — a number an operator sizes the batch from, which a counter \
         hard-coded to one or zero would pass a name-only check with"
    );

    let durations = obs.values_of("idempotency.purge.batch_duration");
    assert!(!durations.is_empty(), "the batch duration must be emitted");
    for d in &durations {
        assert!(
            d.is_finite() && *d >= 0.0,
            "a duration in seconds must be finite and non-negative, got {d}"
        );
    }
}

/// A failing purge reports its duration and **no** row count.
///
/// Both halves are the point. The duration because a failed batch still consumed time,
/// and dropping it would make failure look instantaneous exactly when the database is
/// the slow thing. The absent row count because a failed purge removed nothing —
/// emitting zero would claim work that did not happen and be indistinguishable from a
/// healthy tick with nothing eligible, two states that call for different actions.
#[tokio::test]
async fn a_failing_purge_reports_its_duration_and_no_rows() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let obs = RecordingObservability::new();
    let policy = RetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(Arc::new(FailingStore))
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .with_observability(obs.clone() as Arc<dyn ego_domain::Observability>)
        .build();
    runtime.start_retention().await.expect("the worker starts");

    wait_for_a_metric(&obs, "idempotency.purge.batch_duration").await;
    let _ = runtime.shutdown_async().await;

    assert!(
        !obs.values_of("idempotency.purge.batch_duration").is_empty(),
        "a failed batch still took time: {:?}",
        obs.names()
    );
    assert!(
        obs.values_of("idempotency.purge.rows").is_empty(),
        "a failed purge removed nothing, so a row count would claim work that did not \
         happen: {:?}",
        obs.names()
    );
}

/// A tick with nothing eligible counts zero rows — and that zero is meaningful.
///
/// The distinction the previous test protects, from the other side: an empty-but-healthy
/// tick *does* report a row count, of zero. So "no `purge.rows` sample" means failure and
/// "a sample of zero" means nothing to do, and the two are readable apart.
#[tokio::test]
async fn a_tick_with_nothing_eligible_counts_zero_rows() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let store = RecordingStore::wrapping(InMemoryOperationReservationStore::new(clock.clone()));
    let obs = RecordingObservability::new();
    let policy = RetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .with_observability(obs.clone() as Arc<dyn ego_domain::Observability>)
        .build();
    runtime.start_retention().await.expect("the worker starts");

    wait_for_a_metric(&obs, "idempotency.purge.rows").await;
    let _ = runtime.shutdown_async().await;

    assert_eq!(
        obs.values_of("idempotency.purge.rows").first().copied(),
        Some(0.0),
        "nothing was eligible, so the count is a real zero rather than absent"
    );
}

/// An uninstrumented worker purges and counts nothing.
#[tokio::test]
async fn an_uninstrumented_worker_still_purges_and_counts_nothing() {
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
    assert!(
        store.calls() >= 1,
        "the purge happens without instrumentation"
    );
    let _ = runtime.shutdown_async().await;
}

// ---------------------------------------------------------------------------
// AD-10: idempotency.purge.oldest_completed_age
// ---------------------------------------------------------------------------

/// Completes one reservation at the clock's current instant.
///
/// Separate from `stage_eligible` because the point here is the opposite: these
/// rows must **survive** the purge, so they are staged inside the retention
/// window and the clock is advanced deliberately between them.
async fn complete_one(inner: &InMemoryOperationReservationStore, clock: &TestClock, key: &str) {
    use ego_domain::Clock as _;
    let now = clock.now();
    let outcome = inner
        .reserve(ReserveRequest {
            tenant: None,
            operation_key: OperationKey::parse(key).expect("valid"),
            fingerprint: OperationFingerprint::new("fp"),
            owner_id: OwnerId::new("seeder"),
            lease_until: now + chrono::Duration::seconds(30),
        })
        .await
        .expect("a fresh reservation");
    let lease = match outcome {
        ReservationOutcome::Fresh(lease) => lease,
        other => panic!("expected Fresh, got {other:?}"),
    };
    inner
        .complete(
            &OwnerFence {
                operation_id: lease.operation_id.clone(),
                owner_id: lease.owner_id.clone(),
                fencing_token: lease.fencing_token,
            },
            StoredServiceResponse::new(b"done".to_vec()),
        )
        .await
        .expect("completion");
}

/// The gauge reports the age of the **oldest** surviving completion, in seconds,
/// measured against the injected clock.
///
/// Two rows at different instants, so a store returning the newest instead of the
/// oldest produces a different number rather than the same one. The ages are 120s
/// and 60s and neither is zero or one, so a hard-coded value fails too.
///
/// The whole record is compared, not just the value: `Gauge` and not `Counter`,
/// the literal name, and no attributes — this row of AD-10's table carries none.
#[tokio::test]
async fn the_gauge_reports_the_age_of_the_oldest_surviving_completion() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(10_000)));
    let inner = InMemoryOperationReservationStore::new(clock.clone());

    // The older row, which is the one the gauge must describe.
    complete_one(&inner, &clock, "op-old").await;
    clock.advance(chrono::Duration::seconds(60));
    complete_one(&inner, &clock, "op-new").await;
    clock.advance(chrono::Duration::seconds(60));
    // Retention is 300s and the oldest row is 120s old, so nothing is eligible:
    // whatever the gauge reports is backlog that survived the purge.

    let store = RecordingStore::wrapping(inner);
    let obs = RecordingObservability::new();
    let policy = RetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .with_observability(obs.clone())
        .build();
    runtime.start_retention().await.expect("the worker starts");

    wait_for_a_metric(&obs, "idempotency.purge.oldest_completed_age").await;
    let _ = runtime.shutdown_async().await;

    let records = obs.records_of("idempotency.purge.oldest_completed_age");
    assert_eq!(
        records
            .first()
            .map(|r| (r.kind, r.value, r.attributes.clone())),
        Some((ego_domain::MetricKind::Gauge, 120.0, Vec::new())),
        "the oldest surviving completion is 120s old on the injected clock: a gauge, \
         not a counter, carrying no dimensions — got {records:?}"
    );
}

/// A cleared backlog emits no sample at all.
///
/// `Empty` is a real answer, and the honest gauge reading for it is absence. A
/// `0.0` would claim the oldest completed reservation was written this instant,
/// which is the opposite of what an empty backlog means.
#[tokio::test]
async fn an_empty_backlog_emits_no_sample() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(10_000)));
    let inner = InMemoryOperationReservationStore::new(clock.clone());
    stage_eligible(&inner, &clock, 3).await;

    let store = RecordingStore::wrapping(inner);
    let obs = RecordingObservability::new();
    let policy = RetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .with_observability(obs.clone())
        .build();
    runtime.start_retention().await.expect("the worker starts");

    // The purge itself is the signal that a full tick ran, gauge included.
    wait_for_a_metric(&obs, "idempotency.purge.rows").await;
    let _ = runtime.shutdown_async().await;

    assert!(
        obs.values_of("idempotency.purge.oldest_completed_age")
            .is_empty(),
        "every staged row was purged, so there is no oldest completion and no age to \
         report: {:?}",
        obs.values_of("idempotency.purge.oldest_completed_age")
    );
}

/// A store that does not offer the query emits no sample.
///
/// This double inherits the port's default deliberately — it is the twenty
/// fixtures that have no ordered scan to offer, and the gauge must stay silent
/// for them rather than reporting a number nobody computed.
struct UnsupportingStore {
    inner: InMemoryOperationReservationStore,
    purges: AtomicUsize,
}

#[async_trait]
impl OperationReservationStore for UnsupportingStore {
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
        self.purges.fetch_add(1, Ordering::SeqCst);
        self.inner.purge_completed_before(cutoff, batch).await
    }
    // `oldest_completed` deliberately omitted: this is the default path.
    async fn probe(&self) -> Result<(), ReservationError> {
        self.inner.probe().await
    }
}

#[tokio::test]
async fn a_store_that_does_not_support_the_query_emits_no_sample() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(10_000)));
    let inner = InMemoryOperationReservationStore::new(clock.clone());
    complete_one(&inner, &clock, "op-survivor").await;
    clock.advance(chrono::Duration::seconds(120));

    let store = Arc::new(UnsupportingStore {
        inner,
        purges: AtomicUsize::new(0),
    });
    let obs = RecordingObservability::new();
    let policy = RetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .with_observability(obs.clone())
        .build();
    runtime.start_retention().await.expect("the worker starts");

    wait_for_a_metric(&obs, "idempotency.purge.batch_duration").await;
    let _ = runtime.shutdown_async().await;

    assert!(
        obs.values_of("idempotency.purge.oldest_completed_age")
            .is_empty(),
        "the store answered Unsupported, so no age was determined and none may be \
         reported — there is a surviving completion, which is what makes this \
         different from the empty case: {:?}",
        obs.values_of("idempotency.purge.oldest_completed_age")
    );
}

/// A query that fails emits no sample and does not disturb the existing handling.
struct FailingQueryStore {
    inner: InMemoryOperationReservationStore,
}

#[async_trait]
impl OperationReservationStore for FailingQueryStore {
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
        self.inner.purge_completed_before(cutoff, batch).await
    }
    async fn oldest_completed(&self) -> Result<OldestCompleted, ReservationError> {
        Err(ReservationError::Backend("the backlog query failed".into()))
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        self.inner.probe().await
    }
}

#[tokio::test]
async fn a_failing_backlog_query_emits_no_sample_and_does_not_stop_the_worker() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(10_000)));
    let inner = InMemoryOperationReservationStore::new(clock.clone());
    complete_one(&inner, &clock, "op-survivor").await;
    clock.advance(chrono::Duration::seconds(120));

    let store = Arc::new(FailingQueryStore { inner });
    let obs = RecordingObservability::new();
    let policy = RetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .with_observability(obs.clone())
        .build();
    runtime.start_retention().await.expect("the worker starts");

    wait_for_a_metric(&obs, "idempotency.purge.batch_duration").await;
    let _ = runtime.shutdown_async().await;

    assert!(
        obs.values_of("idempotency.purge.oldest_completed_age")
            .is_empty(),
        "a failed query determined no age, so it may report none: {:?}",
        obs.values_of("idempotency.purge.oldest_completed_age")
    );
    assert!(
        !obs.values_of("idempotency.purge.batch_duration").is_empty(),
        "the rest of the tick is unaffected — the purge still ran and still reported"
    );
}

/// The backlog is queried after the purge, never before.
///
/// Asserted from the store's own recording rather than inferred from the value: a
/// query issued first would describe the rows the batch was about to delete, and
/// on a healthy deployment would report a stale age forever.
#[tokio::test]
async fn the_backlog_is_queried_after_the_purge_not_before() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(10_000)));
    let inner = InMemoryOperationReservationStore::new(clock.clone());
    complete_one(&inner, &clock, "op-survivor").await;
    clock.advance(chrono::Duration::seconds(120));

    let store = RecordingStore::wrapping(inner);
    let obs = RecordingObservability::new();
    let policy = RetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_operation_reservation_store(store.clone())
        .with_reservation_clock(clock.clone())
        .with_retention_policy(policy)
        .with_observability(obs.clone())
        .build();
    runtime.start_retention().await.expect("the worker starts");

    wait_for_a_metric(&obs, "idempotency.purge.oldest_completed_age").await;
    let _ = runtime.shutdown_async().await;

    let queries = store.oldest_queries();
    assert!(
        queries.first().copied().is_some_and(|purges| purges >= 1),
        "the first backlog query must see at least one completed purge; saw {queries:?}"
    );
}
