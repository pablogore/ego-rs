//! Tag scheduler — manages per-projection polling and dispatch.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

use crate::read_side::backpressure::Backpressure;
use crate::read_side::batch_executor::BatchExecutor;
use ego_domain::read_side::config::ReadSideConfig;
use ego_domain::read_side::dedup::DedupStore;
use ego_domain::read_side::event_tag::EventTag;
use ego_domain::read_side::handler::Handler;
use ego_domain::read_side::offset::OffsetStore;
use ego_domain::read_side::progress::ProgressReporter;
use ego_domain::read_side::scheduler::TagScheduler;
use ego_domain::read_side::store::ReadSideStore;

/// Scheduler for managing tag-based projection processing.
///
/// Handles per-projection polling intervals and dispatches tag streams
/// respecting concurrency limits.
pub struct TagSchedulerImpl<E>
where
    E: Clone + Send + Sync,
{
    config: ReadSideConfig,
    backpressure: Arc<Backpressure>,
    batch_executor: BatchExecutor<E>,
    /// Tracks active projections and their tag processing state
    active_projections: HashMap<String, ProjectionState>,
}

/// State tracking for active projections
struct ProjectionState {
    /// Tags currently being processed
    _active_tags: Vec<EventTag>,
    /// Whether the projection is currently running
    _is_running: bool,
}

impl<E> TagSchedulerImpl<E>
where
    E: Clone + Send + Sync,
{
    /// Creates a new tag scheduler with the given configuration.
    pub fn new(config: ReadSideConfig) -> Self {
        let backpressure = Arc::new(Backpressure::new(config.max_in_flight));
        let batch_executor = BatchExecutor::new(config.clone(), backpressure.clone());

        Self {
            config,
            backpressure,
            batch_executor,
            active_projections: HashMap::new(),
        }
    }
}

#[async_trait]
impl<E> TagScheduler<E> for TagSchedulerImpl<E>
where
    E: Clone + Send + Sync,
{
    async fn start_projection(
        &mut self,
        projection_id: String,
        tags: Vec<EventTag>,
        tenant: String,
        handler: impl Handler<E> + Clone,
        read_store: impl ReadSideStore<E> + Send + Sync + Clone,
        dedup_store: impl DedupStore + Send + Sync + Clone,
        offset_store: impl OffsetStore + Send + Sync + Clone,
        reporter: impl ProgressReporter + Clone,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Store projection state
        self.active_projections.insert(
            projection_id.clone(),
            ProjectionState {
                _active_tags: tags.clone(),
                _is_running: true,
            },
        );

        // Process tags in parallel with backpressure
        for tag in tags {
            // Check if we can process this tag (respect concurrency limits)
            if self.backpressure.can_process().await {
                // Create a session for this tag
                let session = ego_domain::read_side::session::ReadSideSession::new(
                    projection_id.clone(),
                    tag.clone(),
                    tenant.clone(),
                    self.config.clone(),
                    handler.clone(),
                    read_store.clone(),
                    dedup_store.clone(),
                    offset_store.clone(),
                    reporter.clone(),
                );

                // Execute the session
                self.batch_executor.execute_session(session).await?;
            }
        }

        Ok(())
    }
}

impl<E> TagSchedulerImpl<E>
where
    E: Clone + Send + Sync + 'static,
{
    /// Additive convenience wrapper (post-CORE-018 review, Finding 8):
    /// `start_projection` above processes one batch per call and returns —
    /// this wraps it in the persistent poll-loop-with-graceful-stop pattern
    /// that callers (e.g. `examples/reference-app`'s `ReadSideRuntime`) would
    /// otherwise have to hand-roll themselves.
    ///
    /// `tag_provider` is called fresh on every iteration rather than being
    /// captured as a fixed `Vec<EventTag>` once — required for dynamic
    /// per-tenant tag discovery, where a tenant's tag only exists once its
    /// first event has been written.
    ///
    /// On stop (`stop_signal` observes `true`), the loop finishes draining
    /// whatever batch is currently in flight before the returned
    /// `JoinHandle` resolves — the stop check only happens between
    /// iterations, never during an in-progress `start_projection` call.
    ///
    /// `on_error` receives every error `start_projection` returns, so
    /// callers can plug in their own logging/observability (this crate has
    /// no logger dependency of its own).
    #[allow(clippy::too_many_arguments)]
    pub fn run_until_stopped<F, H, S, D, O, R>(
        mut self,
        tag_provider: F,
        interval: Duration,
        mut stop_signal: watch::Receiver<bool>,
        projection_id: String,
        tenant: String,
        handler: H,
        read_store: S,
        dedup_store: D,
        offset_store: O,
        reporter: R,
        on_error: impl Fn(Box<dyn std::error::Error>) + Send + Sync + 'static,
    ) -> tokio::task::JoinHandle<()>
    where
        F: Fn() -> Vec<EventTag> + Send + Sync + 'static,
        H: Handler<E> + Clone + Send + Sync + 'static,
        S: ReadSideStore<E> + Send + Sync + Clone + 'static,
        D: DedupStore + Send + Sync + Clone + 'static,
        O: OffsetStore + Send + Sync + Clone + 'static,
        R: ProgressReporter + Clone + Send + Sync + 'static,
    {
        tokio::spawn(async move {
            loop {
                if *stop_signal.borrow() {
                    break;
                }

                let tags = tag_provider();
                if let Err(e) = self
                    .start_projection(
                        projection_id.clone(),
                        tags,
                        tenant.clone(),
                        handler.clone(),
                        read_store.clone(),
                        dedup_store.clone(),
                        offset_store.clone(),
                        reporter.clone(),
                    )
                    .await
                {
                    on_error(e);
                }

                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    changed = stop_signal.changed() => {
                        match changed {
                            // Value changed to `true`: stop now.
                            Ok(()) if *stop_signal.borrow() => break,
                            // Value changed to something other than `true`: keep polling.
                            Ok(()) => {}
                            // Every `Sender` was dropped — no stop signal can ever
                            // arrive again. Treat disconnection as an implicit stop
                            // request; otherwise `changed()` on a closed channel
                            // resolves immediately forever, turning this into an
                            // unbounded busy loop that never respects `interval`.
                            Err(_) => break,
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_domain::read_side::event_stream::EventStreamElement;
    use ego_domain::read_side::offset::Offset;
    use ego_domain::read_side::progress::NoopProgressReporter;
    use ego_domain::read_side::store::ReadSideStoreError;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    #[derive(Clone, Default)]
    struct CountingProvider {
        calls: Arc<AtomicUsize>,
    }

    impl CountingProvider {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    /// Store that returns no events for empty tag lists (fast path used by
    /// the "fresh tag_provider per iteration" test) and, when given a tag,
    /// sleeps before returning one real event (slow path used by the
    /// "drains the in-flight batch" test).
    #[derive(Clone, Default)]
    struct FakeStore {
        fetch_delay: Duration,
    }

    #[async_trait]
    impl ReadSideStore<serde_json::Value> for FakeStore {
        async fn fetch(
            &self,
            tag: &EventTag,
            _offset: Option<&Offset>,
            _batch_size: usize,
        ) -> Result<Vec<EventStreamElement<serde_json::Value>>, ReadSideStoreError> {
            if !self.fetch_delay.is_zero() {
                tokio::time::sleep(self.fetch_delay).await;
            }
            Ok(vec![EventStreamElement::new(
                "event-1",
                "agg-1",
                "tenant-a",
                "Something",
                serde_json::json!({}),
                1,
                chrono::Utc::now(),
                vec![tag.clone()],
            )])
        }
    }

    #[derive(Clone, Default)]
    struct FakeDedup;

    #[async_trait]
    impl DedupStore for FakeDedup {
        async fn seen(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _event_id: &str,
        ) -> Result<bool, ego_domain::read_side::dedup::DedupStoreError> {
            Ok(false)
        }

        async fn mark_seen(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _event_id: &str,
        ) -> Result<(), ego_domain::read_side::dedup::DedupStoreError> {
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct FakeOffset;

    #[async_trait]
    impl OffsetStore for FakeOffset {
        async fn read_offset(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _tenant: &str,
        ) -> Result<Option<Offset>, ego_domain::read_side::offset::OffsetStoreError> {
            Ok(None)
        }

        async fn write_offset(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _tenant: &str,
            _offset: &Offset,
        ) -> Result<(), ego_domain::read_side::offset::OffsetStoreError> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct CountingHandler {
        handled: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Handler<serde_json::Value> for CountingHandler {
        async fn handle(
            &self,
            _events: &[EventStreamElement<serde_json::Value>],
        ) -> Result<(), ego_domain::read_side::error::ProjectionError> {
            self.handled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// (a) `tag_provider` is called fresh every iteration — not captured
    /// once — proven by letting the loop run for several poll intervals
    /// with an empty tag list (so no store/handler machinery is exercised)
    /// and observing multiple calls before stop.
    /// (b) stops gracefully via the watch signal.
    #[tokio::test]
    async fn run_until_stopped_calls_tag_provider_fresh_each_iteration_and_stops_gracefully() {
        let provider = CountingProvider::default();
        let provider_for_closure = provider.clone();

        let (stop_tx, stop_rx) = watch::channel(false);
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());
        let handled = Arc::new(AtomicUsize::new(0));

        let handle = scheduler.run_until_stopped(
            move || {
                provider_for_closure.calls.fetch_add(1, Ordering::SeqCst);
                Vec::new() // empty tags: start_projection returns instantly
            },
            Duration::from_millis(5),
            stop_rx,
            "proj".to_string(),
            "all-tenants".to_string(),
            CountingHandler { handled: handled.clone() },
            FakeStore::default(),
            FakeDedup,
            FakeOffset,
            NoopProgressReporter,
            |_e| {},
        );

        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = stop_tx.send(true);
        handle.await.expect("task joins cleanly");

        assert!(
            provider.calls() >= 3,
            "expected several fresh tag_provider calls across multiple poll iterations, got {}",
            provider.calls()
        );
    }

    /// (c) drains any in-flight batch before returning: the stop signal is
    /// sent while `start_projection`'s (simulated slow) fetch is still
    /// running, and the handler must still have been invoked — proving the
    /// loop awaited the in-progress batch to completion rather than
    /// aborting it — before the returned `JoinHandle` resolves.
    #[tokio::test]
    async fn run_until_stopped_drains_in_flight_batch_before_returning() {
        let (stop_tx, stop_rx) = watch::channel(false);
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());
        let handled = Arc::new(AtomicUsize::new(0));
        let seen_tags: Arc<StdMutex<Vec<()>>> = Arc::new(StdMutex::new(Vec::new()));
        let seen_tags_for_closure = seen_tags.clone();

        let handle = scheduler.run_until_stopped(
            move || {
                seen_tags_for_closure.lock().unwrap().push(());
                vec![EventTag::new("tenant-a")]
            },
            Duration::from_millis(5),
            stop_rx,
            "proj".to_string(),
            "all-tenants".to_string(),
            CountingHandler { handled: handled.clone() },
            FakeStore {
                fetch_delay: Duration::from_millis(60),
            },
            FakeDedup,
            FakeOffset,
            NoopProgressReporter,
            |_e| {},
        );

        // Send stop while the first (slow) fetch is still in flight.
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = stop_tx.send(true);

        let start = std::time::Instant::now();
        handle.await.expect("task joins cleanly");

        assert!(
            start.elapsed() >= Duration::from_millis(45),
            "expected the in-flight slow fetch to be awaited to completion, returned too fast: {:?}",
            start.elapsed()
        );
        assert_eq!(
            handled.load(Ordering::SeqCst),
            1,
            "the in-flight batch's handler must have run exactly once, proving it was drained, not aborted"
        );
    }

    /// Post-review Finding F-01: dropping every `watch::Sender` without ever
    /// sending `true` must stop the loop, not spin it unbounded. Before the
    /// fix, `stop_signal.changed()`'s `Result` was discarded in the
    /// `select!`, so a closed channel (which makes `changed()` resolve
    /// immediately with `Err` forever) bypassed `sleep(interval)` on every
    /// iteration — an unthrottled busy loop that never terminates on its own.
    #[tokio::test]
    async fn run_until_stopped_terminates_when_stop_sender_is_dropped_without_sending() {
        let provider = CountingProvider::default();
        let provider_for_closure = provider.clone();

        let (stop_tx, stop_rx) = watch::channel(false);
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());
        let handled = Arc::new(AtomicUsize::new(0));

        let handle = scheduler.run_until_stopped(
            move || {
                provider_for_closure.calls.fetch_add(1, Ordering::SeqCst);
                Vec::new()
            },
            Duration::from_millis(5),
            stop_rx,
            "proj".to_string(),
            "all-tenants".to_string(),
            CountingHandler { handled: handled.clone() },
            FakeStore::default(),
            FakeDedup,
            FakeOffset,
            NoopProgressReporter,
            |_e| {},
        );

        // Drop the sender WITHOUT sending `true` — the disconnection itself,
        // not an explicit stop value, must be what ends the loop.
        drop(stop_tx);

        tokio::time::timeout(Duration::from_millis(200), handle)
            .await
            .expect("run_until_stopped must terminate when the stop channel is dropped, not spin forever")
            .expect("task joins cleanly");

        // Non-vacuousness guard: if the fix regressed to a busy loop, the
        // task above would never have returned within the timeout at all —
        // this assertion only runs on the success path, but keeps the
        // provider handle alive so a future refactor can't silently drop it
        // and weaken the test to "did the JoinHandle exist".
        assert!(provider.calls() >= 1, "the loop must have run at least once before stopping");
    }
}
