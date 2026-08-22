//! The effect-retention worker (PROD-002 G12): off by default, validated,
//! runtime-owned, and stopped.
//!
//! Mirrors `retention_worker_lifecycle.rs`'s testing style for PROD-012's
//! reservation-retention worker, scoped to what this worker actually does:
//! no G13 metrics/gauge are wired here (see `effect_retention.rs`'s TODO —
//! `effect.cleanup.rows`/`effect.cleanup.batch_duration` are already fixed
//! names and used, but no backlog-age gauge name has been decided yet), so
//! this file does not assert one. Everything asserted here is observable
//! through the `RetentionMaintenance` port with a recording double and a
//! fake clock — no Postgres/Stoolap needed: the provider's own SQL is
//! already covered by `run_retention`'s existing tests and by
//! `crates/effect-store/tests/conformance.rs`'s
//! `retention_maintenance_purge_before_calls_through_to_run_retention`,
//! which proves the trait wiring calls through without reimplementing SQL.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use ego_runtime::effects::store::{EffectStoreError, Timestamp};
use ego_runtime::effects::RetentionMaintenance;
use ego_service_sdk::runtime::{
    EffectRetentionPolicy, EffectRetentionPolicyError, IdempotencyEnforcementMode, RuntimeBuilder,
};
use ego_testkit::TestClock;

fn epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

/// Records every purge the worker asks for.
struct RecordingStore {
    purges: Mutex<Vec<(Timestamp, usize)>>,
    calls: AtomicUsize,
    /// What `purge_before` returns each call — the deleted count, cycled if
    /// there are more calls than entries.
    results: Vec<Result<u64, EffectStoreError>>,
}

impl RecordingStore {
    fn wrapping(results: Vec<Result<u64, EffectStoreError>>) -> Arc<Self> {
        Arc::new(Self {
            purges: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            results,
        })
    }
    fn always_ok(deleted: u64) -> Arc<Self> {
        Self::wrapping(vec![Ok(deleted)])
    }
    fn purges(&self) -> Vec<(Timestamp, usize)> {
        self.purges.lock().expect("not poisoned").clone()
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl RetentionMaintenance for RecordingStore {
    async fn purge_before(&self, cutoff: Timestamp, batch: usize) -> Result<u64, EffectStoreError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.purges.lock().expect("not poisoned").push((cutoff, batch));
        self.results
            .get(n.min(self.results.len().saturating_sub(1)))
            .cloned()
            .unwrap_or(Ok(0))
    }
}

/// Waits for the worker's first pass, or fails.
async fn wait_for_a_purge(store: &RecordingStore) {
    for _ in 0..500 {
        if store.calls() >= 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the effect retention worker never purged; it was configured and started");
}

// ---------------------------------------------------------------------------
// 1. No policy, no worker
// ---------------------------------------------------------------------------

#[tokio::test]
async fn without_a_policy_no_worker_starts_and_nothing_is_purged() {
    let store = RecordingStore::always_ok(0);

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("starting is a no-op");

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        store.calls(),
        0,
        "effect retention is opt-in: an SDK upgrade must not begin deleting data on \
         a schedule nobody chose"
    );
}

// ---------------------------------------------------------------------------
// 2. A degenerate policy is refused where it is built
// ---------------------------------------------------------------------------

#[test]
fn a_policy_with_a_zero_value_is_refused() {
    let ok = Duration::from_secs(1);
    assert_eq!(
        EffectRetentionPolicy::new(Duration::ZERO, ok, 1),
        Err(EffectRetentionPolicyError::ZeroRetention)
    );
    assert_eq!(
        EffectRetentionPolicy::new(ok, Duration::ZERO, 1),
        Err(EffectRetentionPolicyError::ZeroInterval)
    );
    assert_eq!(
        EffectRetentionPolicy::new(ok, ok, 0),
        Err(EffectRetentionPolicyError::ZeroBatch)
    );
}

/// A policy with no store to purge cannot mean what it says.
#[test]
#[should_panic(expected = "no RetentionMaintenance was registered")]
fn a_policy_without_a_retention_store_is_refused_at_build() {
    let policy = EffectRetentionPolicy::new(Duration::from_secs(60), Duration::from_secs(1), 10)
        .expect("a valid policy");
    let _ = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_policy(policy)
        .build();
}

// ---------------------------------------------------------------------------
// 3 & 4. It purges with the configured cutoff and batch, and shutdown ends it
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_configured_worker_purges_with_the_exact_cutoff_and_batch_then_stops() {
    let now = epoch() + chrono::Duration::seconds(1_000);
    let clock = Arc::new(TestClock::new(now));
    let store = RecordingStore::always_ok(0);

    let retention = Duration::from_secs(300);
    let policy = EffectRetentionPolicy::new(retention, Duration::from_millis(20), 7)
        .expect("a valid policy");

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("the worker starts");

    wait_for_a_purge(&store).await;

    let (cutoff, batch) = store.purges()[0];
    assert_eq!(
        cutoff,
        Timestamp::from_utc(now - chrono::Duration::seconds(300)),
        "the cutoff is the runtime's clock minus the retention window — not wall time"
    );
    assert_eq!(batch, 7, "the batch is the configured one");

    runtime.shutdown_async().await.expect("shutdown succeeds");
    let after_shutdown = store.calls();
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(
        store.calls(),
        after_shutdown,
        "no tick happened after shutdown returned: the teardown hook cancelled the \
         loop and waited for it"
    );
}

// ---------------------------------------------------------------------------
// 5. A worker that overruns its deadline is cancelled, not detached
// ---------------------------------------------------------------------------

struct HangingStore {
    entered: Arc<AtomicUsize>,
    resumed: Arc<AtomicUsize>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl RetentionMaintenance for HangingStore {
    async fn purge_before(&self, _cutoff: Timestamp, _batch: usize) -> Result<u64, EffectStoreError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        self.release.notified().await;
        self.resumed.fetch_add(1, Ordering::SeqCst);
        Ok(0)
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

    let policy = EffectRetentionPolicy::new(Duration::from_secs(60), Duration::from_nanos(1), 1)
        .expect("a valid policy");

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("the worker starts");

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

    let outcome = runtime.shutdown_async().await;
    assert!(
        outcome.is_err(),
        "an overrunning worker must be surfaced, not swallowed: got {outcome:?}"
    );

    release.notify_waiters();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        resumed.load(Ordering::SeqCst),
        0,
        "the parked purge resumed after shutdown returned, so the task was detached \
         rather than aborted"
    );
}

// ---------------------------------------------------------------------------
// 6. Starting twice starts one worker
// ---------------------------------------------------------------------------

#[tokio::test]
async fn starting_effect_retention_twice_starts_one_worker() {
    let clock = Arc::new(TestClock::new(epoch()));
    let store = RecordingStore::always_ok(0);

    let policy = EffectRetentionPolicy::new(Duration::from_secs(60), Duration::from_secs(3_600), 5)
        .expect("a valid policy");

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .build();

    runtime
        .start_retention_effects()
        .await
        .expect("the first call starts");
    runtime
        .start_retention_effects()
        .await
        .expect("the second call is a no-op");

    wait_for_a_purge(&store).await;
    tokio::time::sleep(Duration::from_millis(120)).await;

    assert_eq!(
        store.calls(),
        1,
        "one worker, one first pass — two would mean a second loop purging on the \
         same schedule"
    );

    runtime.shutdown_async().await.expect("shutdown succeeds");
}

// ---------------------------------------------------------------------------
// 7. effect.purge_batch tracing
// ---------------------------------------------------------------------------

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

#[tokio::test]
async fn every_purge_tick_opens_a_root_effect_purge_batch_span() {
    let clock = Arc::new(TestClock::new(epoch()));
    let store = RecordingStore::always_ok(0);
    let tracer = SpanRecordingTracer::new();

    let policy = EffectRetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .with_tracer(tracer.clone() as Arc<dyn ego_domain::Tracer>)
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("the worker starts");
    wait_for_a_purge(&store).await;

    for _ in 0..500 {
        if !tracer.started().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let _ = runtime.shutdown_async().await;

    let started = tracer.started();
    assert!(!started.is_empty(), "a traced worker must report its ticks");
    for (name, parent, _) in &started {
        assert_eq!(name, "effect.purge_batch");
        assert_eq!(
            *parent, None,
            "a background tick has no request span to descend from"
        );
    }
}
