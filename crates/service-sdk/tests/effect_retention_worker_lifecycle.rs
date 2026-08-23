//! The effect-retention worker (PROD-002 G12/G13): off by default, validated,
//! runtime-owned, stopped, and — since G13 — instrumented.
//!
//! Mirrors `retention_worker_lifecycle.rs`'s testing style for PROD-012's
//! reservation-retention worker. Everything asserted here is observable
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

/// Records every purge the worker asks for, and answers `oldest_terminal`
/// with whatever this test configured (defaulting to the trait's own
/// `Ok(None)`, exactly as a provider that never overrides it would).
struct RecordingStore {
    purges: Mutex<Vec<(Timestamp, usize)>>,
    calls: AtomicUsize,
    /// What `purge_before` returns each call — the deleted count, cycled if
    /// there are more calls than entries.
    results: Vec<Result<u64, EffectStoreError>>,
    /// What `oldest_terminal` returns on every call (G13).
    oldest_terminal: Mutex<Result<Option<Timestamp>, EffectStoreError>>,
}

impl RecordingStore {
    fn wrapping(results: Vec<Result<u64, EffectStoreError>>) -> Arc<Self> {
        Arc::new(Self {
            purges: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            results,
            oldest_terminal: Mutex::new(Ok(None)),
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
    /// Configures what `oldest_terminal` answers from here on.
    fn set_oldest_terminal(&self, answer: Result<Option<Timestamp>, EffectStoreError>) {
        *self.oldest_terminal.lock().expect("not poisoned") = answer;
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

    async fn oldest_terminal(&self) -> Result<Option<Timestamp>, EffectStoreError> {
        self.oldest_terminal.lock().expect("not poisoned").clone()
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

// ---------------------------------------------------------------------------
// 8. PROD-002 G13: effect.cleanup.rows / effect.cleanup.batch_duration
// ---------------------------------------------------------------------------

/// Records every `metric` call in order, whole — mirrors
/// `retention_worker_lifecycle.rs`'s own fixture of the same name for
/// PROD-012's reservation-retention worker.
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

async fn wait_for_a_metric(obs: &RecordingObservability, name: &str) {
    for _ in 0..500 {
        if obs.names().iter().any(|n| n == name) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the worker never emitted {name}; it was configured, instrumented and started");
}

/// A successful tick counts the rows it actually removed, and how long it
/// took — the exact two-metric contract `RetentionWorker` established for
/// reservations (`retention_worker_lifecycle.rs`), now proven for the effect
/// worker too.
#[tokio::test]
async fn a_successful_tick_counts_the_rows_it_removed_and_its_duration() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let store = RecordingStore::always_ok(3);
    let obs = RecordingObservability::new();
    let policy = EffectRetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .with_observability(obs.clone() as Arc<dyn ego_domain::Observability>)
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("the worker starts");

    wait_for_a_metric(&obs, "effect.cleanup.rows").await;
    let _ = runtime.shutdown_async().await;

    assert_eq!(
        obs.values_of("effect.cleanup.rows").first().copied(),
        Some(3.0),
        "the first tick removed three settled effects, so the counter must say three"
    );
    let durations = obs.values_of("effect.cleanup.batch_duration");
    assert!(!durations.is_empty(), "the batch duration must be emitted");
    for d in &durations {
        assert!(
            d.is_finite() && *d >= 0.0,
            "a duration in seconds must be finite and non-negative, got {d}"
        );
    }
}

/// A failing purge reports its duration and **no** row count — the failed
/// batch removed nothing, so a row count would claim work that did not
/// happen.
#[tokio::test]
async fn a_failing_purge_reports_its_duration_and_no_rows() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let store = RecordingStore::wrapping(vec![Err(EffectStoreError::Conflict(
        "purge unavailable".to_string(),
    ))]);
    let obs = RecordingObservability::new();
    let policy = EffectRetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .with_observability(obs.clone() as Arc<dyn ego_domain::Observability>)
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("the worker starts");

    wait_for_a_metric(&obs, "effect.cleanup.batch_duration").await;
    let _ = runtime.shutdown_async().await;

    assert!(
        !obs.values_of("effect.cleanup.batch_duration").is_empty(),
        "a failed batch still took time: {:?}",
        obs.names()
    );
    assert!(
        obs.values_of("effect.cleanup.rows").is_empty(),
        "a failed purge removed nothing, so a row count would claim work that did not \
         happen: {:?}",
        obs.names()
    );
}

/// No `Observability` registered at all: the worker still purges — the
/// metric sites are silent no-ops, not a hard dependency.
#[tokio::test]
async fn an_uninstrumented_worker_still_purges_and_counts_nothing() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let store = RecordingStore::always_ok(0);
    let policy = EffectRetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
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
    assert!(store.calls() >= 1, "the purge happens without instrumentation");
    let _ = runtime.shutdown_async().await;
}

// ---------------------------------------------------------------------------
// 9. PROD-002 G13: effect.cleanup.oldest_terminal_age
// ---------------------------------------------------------------------------

/// The gauge reports the age of the oldest surviving settled effect, in
/// seconds, measured against the injected clock — mirrors
/// `idempotency.purge.oldest_completed_age`'s own contract exactly.
#[tokio::test]
async fn the_gauge_reports_the_age_of_the_oldest_surviving_settlement() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(10_000)));
    let store = RecordingStore::always_ok(0);
    store.set_oldest_terminal(Ok(Some(Timestamp::from_utc(
        epoch() + chrono::Duration::seconds(9_880),
    ))));
    let obs = RecordingObservability::new();
    let policy = EffectRetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .with_observability(obs.clone() as Arc<dyn ego_domain::Observability>)
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("the worker starts");

    wait_for_a_metric(&obs, "effect.cleanup.oldest_terminal_age").await;
    let _ = runtime.shutdown_async().await;

    let records = obs.records_of("effect.cleanup.oldest_terminal_age");
    assert_eq!(
        records
            .first()
            .map(|r| (r.kind, r.value, r.attributes.clone())),
        Some((ego_domain::MetricKind::Gauge, 120.0, Vec::new())),
        "the oldest surviving settlement is 120s old on the injected clock: a gauge, not \
         a counter, carrying no dimensions — got {records:?}"
    );
}

/// A settlement stamped *ahead* of the observing clock reports `0.0`, never a
/// negative age — same clock-skew clamp `RetentionWorker` applies for
/// reservations.
#[tokio::test]
async fn a_settlement_ahead_of_the_observing_clock_reports_zero_not_a_negative_age() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(10_000)));
    let store = RecordingStore::always_ok(0);
    // Stamped 10_000s ahead of the reader's clock.
    store.set_oldest_terminal(Ok(Some(Timestamp::from_utc(
        epoch() + chrono::Duration::seconds(20_000),
    ))));
    let obs = RecordingObservability::new();
    let policy = EffectRetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .with_observability(obs.clone() as Arc<dyn ego_domain::Observability>)
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("the worker starts");

    wait_for_a_metric(&obs, "effect.cleanup.oldest_terminal_age").await;
    let _ = runtime.shutdown_async().await;

    let records = obs.records_of("effect.cleanup.oldest_terminal_age");
    assert_eq!(
        records
            .first()
            .map(|r| (r.kind, r.value, r.attributes.clone())),
        Some((ego_domain::MetricKind::Gauge, 0.0, Vec::new())),
        "a settlement 10_000s ahead of the reader must report exactly 0.0 — got {records:?}"
    );
    for record in &records {
        assert!(
            record.value >= 0.0,
            "no sample of a backlog age may be negative, got {record:?}"
        );
    }
}

/// `Ok(None)` — the trait's own default for "empty or unsupported" — emits no
/// sample at all. A `0.0` would claim the oldest settled effect was written
/// this instant, which is the opposite of what "nothing to report" means.
#[tokio::test]
async fn no_sample_when_the_store_reports_none() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let store = RecordingStore::always_ok(0); // oldest_terminal defaults to Ok(None)
    let obs = RecordingObservability::new();
    let policy = EffectRetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .with_observability(obs.clone() as Arc<dyn ego_domain::Observability>)
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("the worker starts");

    // The purge itself is the signal that a full tick ran, gauge included.
    wait_for_a_metric(&obs, "effect.cleanup.rows").await;
    let _ = runtime.shutdown_async().await;

    assert!(
        obs.values_of("effect.cleanup.oldest_terminal_age").is_empty(),
        "a provider that does not track this (the default) must add no sample: {:?}",
        obs.values_of("effect.cleanup.oldest_terminal_age")
    );
}

/// An `Err` from `oldest_terminal` adds no sample of its own — the existing
/// duration/rows handling already covers the tick; a gauge failure must not
/// invent a value.
#[tokio::test]
async fn no_sample_when_oldest_terminal_errors() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(1_000)));
    let store = RecordingStore::always_ok(0);
    store.set_oldest_terminal(Err(EffectStoreError::Conflict(
        "oldest_terminal unavailable".to_string(),
    )));
    let obs = RecordingObservability::new();
    let policy = EffectRetentionPolicy::new(Duration::from_secs(300), Duration::from_millis(20), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .with_observability(obs.clone() as Arc<dyn ego_domain::Observability>)
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("the worker starts");

    wait_for_a_metric(&obs, "effect.cleanup.rows").await;
    let _ = runtime.shutdown_async().await;

    assert!(
        obs.values_of("effect.cleanup.oldest_terminal_age").is_empty(),
        "an error from oldest_terminal must add no sample: {:?}",
        obs.values_of("effect.cleanup.oldest_terminal_age")
    );
}

/// No duplicate emission: exactly one sample of each metric per tick, all
/// from the worker — nothing in `RetentionMaintenance`/the provider path
/// emits a second copy.
#[tokio::test]
async fn each_metric_is_emitted_exactly_once_per_tick_not_duplicated() {
    let clock = Arc::new(TestClock::new(epoch() + chrono::Duration::seconds(10_000)));
    let store = RecordingStore::always_ok(2);
    store.set_oldest_terminal(Ok(Some(Timestamp::from_utc(
        epoch() + chrono::Duration::seconds(9_940),
    ))));
    let obs = RecordingObservability::new();
    // An interval long enough that only the immediate first-pass tick fires
    // before shutdown below.
    let policy = EffectRetentionPolicy::new(Duration::from_secs(300), Duration::from_secs(3_600), 7)
        .expect("a valid policy");
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_retention_store(store.clone())
        .with_effect_retention_clock(clock.clone())
        .with_effect_retention_policy(policy)
        .with_observability(obs.clone() as Arc<dyn ego_domain::Observability>)
        .build();
    runtime
        .start_retention_effects()
        .await
        .expect("the worker starts");

    wait_for_a_metric(&obs, "effect.cleanup.oldest_terminal_age").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = runtime.shutdown_async().await;

    for name in [
        "effect.cleanup.rows",
        "effect.cleanup.batch_duration",
        "effect.cleanup.oldest_terminal_age",
    ] {
        assert_eq!(
            obs.values_of(name).len(),
            1,
            "{name} must be emitted exactly once for the one tick that ran: {:?}",
            obs.values_of(name)
        );
    }
}
