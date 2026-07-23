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
use ego_domain::read_side::progress::{NoopProgressReporter, ProgressReporter};
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
        tags: Vec<(EventTag, String)>,
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
                _active_tags: tags.iter().map(|(tag, _tenant)| tag.clone()).collect(),
                _is_running: true,
            },
        );

        // Process each (tag, tenant) pair in parallel with backpressure
        for (tag, tenant) in tags {
            // Check if we can process this tag (respect concurrency limits)
            if self.backpressure.can_process().await {
                // Create a session for this tag, threading its own tenant
                let session = ego_domain::read_side::session::ReadSideSession::new(
                    projection_id.clone(),
                    tag.clone(),
                    tenant,
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

/// Handle to a projection poll loop spawned via [`TagSchedulerImpl::spawn`].
///
/// Bundles the loop's `JoinHandle` together with the `watch` stop-signal sender
/// that [`TagSchedulerImpl::spawn`] creates on the caller's behalf, so a caller
/// gets one ready-to-use handle instead of wiring the channel itself.
pub struct ReadSideProjectionHandle {
    stop_tx: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl ReadSideProjectionHandle {
    /// Signals the loop to stop, then awaits the in-flight batch to drain
    /// before returning. Surfaces (does not swallow) a `JoinError` — the
    /// explicit callback to CORE-018's Finding F-02, where a spawned
    /// scheduler task's panic used to vanish silently.
    pub async fn stop(self) -> Result<(), tokio::task::JoinError> {
        let _ = self.stop_tx.send(true);
        self.task.await
    }
}

/// Grouped configuration for spawning a projection poll loop via
/// [`TagSchedulerImpl::spawn`]. Three boilerplate knobs are defaulted so the
/// common case only names what it actually cares about:
///
/// - `reporter` defaults to [`NoopProgressReporter`] (the `R` type parameter's
///   default), swappable via the type-changing [`ProjectionSpec::reporter`];
/// - `interval` defaults to one second, overridable via
///   [`ProjectionSpec::interval`];
/// - `on_error` defaults to a no-op, overridable via
///   [`ProjectionSpec::on_error`].
///
/// Build one with [`ProjectionSpec::new`] and hand it to
/// [`TagSchedulerImpl::spawn`] — that is the single supported way to launch a
/// projection.
pub struct ProjectionSpec<F, H, S, D, O, R = NoopProgressReporter> {
    tag_provider: F,
    projection_id: String,
    handler: H,
    read_store: S,
    dedup_store: D,
    offset_store: O,
    reporter: R,
    interval: Duration,
    on_error: Box<dyn Fn(Box<dyn std::error::Error>) + Send + Sync>,
}

impl<F, H, S, D, O> ProjectionSpec<F, H, S, D, O, NoopProgressReporter> {
    /// Builds a spec with the required knobs, leaving the reporter
    /// ([`NoopProgressReporter`]), poll `interval` (one second) and `on_error`
    /// (no-op) at their defaults.
    pub fn new(
        projection_id: impl Into<String>,
        tag_provider: F,
        handler: H,
        read_store: S,
        dedup_store: D,
        offset_store: O,
    ) -> Self {
        Self {
            tag_provider,
            projection_id: projection_id.into(),
            handler,
            read_store,
            dedup_store,
            offset_store,
            reporter: NoopProgressReporter,
            interval: Duration::from_secs(1),
            on_error: Box::new(|_| {}),
        }
    }
}

impl<F, H, S, D, O, R> ProjectionSpec<F, H, S, D, O, R> {
    /// Overrides the poll interval (default: one second).
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Swaps in a caller-supplied progress reporter, changing the spec's `R`
    /// type parameter away from the default [`NoopProgressReporter`].
    pub fn reporter<R2>(self, reporter: R2) -> ProjectionSpec<F, H, S, D, O, R2> {
        ProjectionSpec {
            tag_provider: self.tag_provider,
            projection_id: self.projection_id,
            handler: self.handler,
            read_store: self.read_store,
            dedup_store: self.dedup_store,
            offset_store: self.offset_store,
            reporter,
            interval: self.interval,
            on_error: self.on_error,
        }
    }

    /// Overrides the poll-failure callback (default: no-op). Receives every
    /// error the underlying `start_projection` returns.
    pub fn on_error(
        mut self,
        on_error: impl Fn(Box<dyn std::error::Error>) + Send + Sync + 'static,
    ) -> Self {
        self.on_error = Box::new(on_error);
        self
    }
}

impl<E> TagSchedulerImpl<E>
where
    E: Clone + Send + Sync + 'static,
{
    /// Spawns a projection poll loop from a [`ProjectionSpec`] and returns a
    /// single [`ReadSideProjectionHandle`]. This is the only supported entry
    /// point for launching a projection: the spec groups the arguments and
    /// defaults the boilerplate, and `spawn` creates the `watch` stop channel
    /// internally so the caller never has to wire it.
    ///
    /// `tag_provider` is called fresh on every iteration rather than being
    /// captured as a fixed `Vec<(EventTag, String)>` once — required for
    /// dynamic per-tenant tag discovery, where a tenant's tag only exists
    /// once its first event has been written. Each yielded pair carries the
    /// tag together with its own authoritative tenant, threaded straight into
    /// that tag's session.
    ///
    /// On stop (the handle's [`stop`](ReadSideProjectionHandle::stop) is called,
    /// or the handle is dropped so the internal sender disconnects) the loop
    /// finishes draining whatever batch is currently in flight before the task
    /// resolves — the stop check only happens between iterations, never during
    /// an in-progress `start_projection` call.
    ///
    /// `on_error` (from the spec) receives every error `start_projection`
    /// returns, so callers can plug in their own logging/observability (this
    /// crate has no logger dependency of its own).
    pub fn spawn<F, H, S, D, O, R>(
        self,
        spec: ProjectionSpec<F, H, S, D, O, R>,
    ) -> ReadSideProjectionHandle
    where
        F: Fn() -> Vec<(EventTag, String)> + Send + Sync + 'static,
        H: Handler<E> + Clone + Send + Sync + 'static,
        S: ReadSideStore<E> + Send + Sync + Clone + 'static,
        D: DedupStore + Send + Sync + Clone + 'static,
        O: OffsetStore + Send + Sync + Clone + 'static,
        R: ProgressReporter + Clone + Send + Sync + 'static,
    {
        let ProjectionSpec {
            tag_provider,
            projection_id,
            handler,
            read_store,
            dedup_store,
            offset_store,
            reporter,
            interval,
            on_error,
        } = spec;

        let (stop_tx, mut stop_signal) = watch::channel(false);
        let mut scheduler = self;
        let task = tokio::spawn(async move {
            loop {
                if *stop_signal.borrow() {
                    break;
                }

                let tags = tag_provider();
                if let Err(e) = scheduler
                    .start_projection(
                        projection_id.clone(),
                        tags,
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
        });
        ReadSideProjectionHandle { stop_tx, task }
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
            _tenant: &str,
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

    /// Handler that always panics — used to prove `spawn`'s
    /// [`stop`](ReadSideProjectionHandle::stop) surfaces a `JoinError` instead
    /// of swallowing it (CORE-018 Finding F-02's lesson applied to the
    /// spawn/stop lifecycle).
    #[derive(Clone)]
    struct PanickingHandler;

    #[async_trait]
    impl Handler<serde_json::Value> for PanickingHandler {
        async fn handle(
            &self,
            _events: &[EventStreamElement<serde_json::Value>],
        ) -> Result<(), ego_domain::read_side::error::ProjectionError> {
            panic!("deliberate handler panic for spawn JoinError test");
        }
    }

    /// (a) `tag_provider` is called fresh every iteration — not captured
    /// once — proven by letting the loop run for several poll intervals with
    /// an empty tag list (so no store/handler machinery is exercised) and
    /// observing multiple calls before stop. (b) stops gracefully when the
    /// handle's `stop()` is called. The spec touches none of the defaulted
    /// fields (reporter/on_error left at their defaults), proving the default
    /// path is wired correctly.
    #[tokio::test]
    async fn spawn_with_default_spec_calls_tag_provider_and_stops_gracefully() {
        let provider = CountingProvider::default();
        let provider_for_closure = provider.clone();
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());
        let handled = Arc::new(AtomicUsize::new(0));

        // Only the required args — reporter, on_error and interval are left at
        // their spec defaults, then the interval is narrowed so the loop turns
        // over several times within the test window.
        let spec = ProjectionSpec::new(
            "proj",
            move || {
                provider_for_closure.calls.fetch_add(1, Ordering::SeqCst);
                Vec::new()
            },
            CountingHandler {
                handled: handled.clone(),
            },
            FakeStore::default(),
            FakeDedup,
            FakeOffset,
        )
        .interval(Duration::from_millis(5));

        let handle = scheduler.spawn(spec);

        tokio::time::sleep(Duration::from_millis(30)).await;
        handle.stop().await.expect("task joins cleanly");

        assert!(
            provider.calls() >= 3,
            "expected several fresh tag_provider calls across multiple poll iterations, got {}",
            provider.calls()
        );
    }

    /// The `ProjectionSpec::reporter` type-changing builder swaps the default
    /// `NoopProgressReporter` for a caller-supplied reporter, and `spawn` must
    /// actually drive that reporter through the same session machinery — proven
    /// by a recording reporter observing at least one completed batch for a
    /// real (non-empty) tag stream.
    #[tokio::test]
    async fn spawn_with_custom_reporter_spec_drives_the_reporter() {
        #[derive(Clone, Default)]
        struct RecordingReporter {
            reports: Arc<AtomicUsize>,
        }

        impl ProgressReporter for RecordingReporter {
            fn on_batch_completed(
                &self,
                _projection_id: &str,
                _tag: &EventTag,
                _count: usize,
                _offset: &Offset,
            ) {
                self.reports.fetch_add(1, Ordering::SeqCst);
            }
        }

        let reporter = RecordingReporter::default();
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());
        let handled = Arc::new(AtomicUsize::new(0));

        let spec = ProjectionSpec::new(
            "proj",
            || vec![(EventTag::new("tenant-a"), "tenant-a".to_string())],
            CountingHandler {
                handled: handled.clone(),
            },
            FakeStore::default(),
            FakeDedup,
            FakeOffset,
        )
        .interval(Duration::from_millis(5))
        .reporter(reporter.clone());

        let handle = scheduler.spawn(spec);

        tokio::time::sleep(Duration::from_millis(30)).await;
        handle.stop().await.expect("task joins cleanly");

        assert!(
            reporter.reports.load(Ordering::SeqCst) >= 1,
            "the custom reporter supplied via ProjectionSpec::reporter must have been driven at least once"
        );
    }

    /// The spawn/stop lifecycle drains any in-flight batch before `stop()`
    /// resolves: the stop signal is sent (via `stop()`) while a simulated slow
    /// fetch is still running, and the handler must still have been invoked —
    /// proving the loop awaited the in-progress batch to completion rather than
    /// aborting it — before `stop()` returns.
    #[tokio::test]
    async fn spawn_stop_drains_in_flight_batch_before_returning() {
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());
        let handled = Arc::new(AtomicUsize::new(0));

        let spec = ProjectionSpec::new(
            "proj",
            || vec![(EventTag::new("tenant-a"), "tenant-a".to_string())],
            CountingHandler {
                handled: handled.clone(),
            },
            FakeStore {
                fetch_delay: Duration::from_millis(60),
            },
            FakeDedup,
            FakeOffset,
        )
        .interval(Duration::from_millis(5));

        let handle = scheduler.spawn(spec);

        // Send stop while the first (slow) fetch is still in flight.
        tokio::time::sleep(Duration::from_millis(10)).await;

        let start = std::time::Instant::now();
        handle.stop().await.expect("task joins cleanly");

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

    /// Post-review Finding F-01, exercised through the public `spawn` API:
    /// dropping the [`ReadSideProjectionHandle`] without calling `stop()` drops
    /// the internal `watch::Sender`, and the poll loop must observe that
    /// disconnection and terminate — not spin unbounded. Before the fix,
    /// `stop_signal.changed()`'s `Result` was discarded in the `select!`, so a
    /// closed channel (which makes `changed()` resolve immediately with `Err`
    /// forever) bypassed `sleep(interval)` on every iteration — an unthrottled
    /// busy loop that never terminates on its own.
    ///
    /// The task's `JoinHandle` is bundled inside the dropped handle, so we can't
    /// await it directly; termination is proven observationally — once the loop
    /// stops, `tag_provider` stops being called, so its call count must plateau
    /// across two windows (a regressed busy loop would keep incrementing it).
    #[tokio::test]
    async fn spawn_terminates_when_handle_is_dropped_without_stopping() {
        let provider = CountingProvider::default();
        let provider_for_closure = provider.clone();
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());
        let handled = Arc::new(AtomicUsize::new(0));

        let spec = ProjectionSpec::new(
            "proj",
            move || {
                provider_for_closure.calls.fetch_add(1, Ordering::SeqCst);
                Vec::new()
            },
            CountingHandler {
                handled: handled.clone(),
            },
            FakeStore::default(),
            FakeDedup,
            FakeOffset,
        )
        .interval(Duration::from_millis(5));

        let handle = scheduler.spawn(spec);

        // Let the loop turn over a few times, then drop the handle WITHOUT
        // stopping — the dropped sender's disconnection itself must end the
        // loop.
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(handle);

        // Give the loop ample time to notice the drop and break, then sample
        // the call count across two windows.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let after_drop = provider.calls();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let later = provider.calls();

        assert!(
            after_drop >= 1,
            "the loop must have run at least once before the handle was dropped"
        );
        assert_eq!(
            after_drop, later,
            "after the handle (and its stop sender) was dropped the loop must have \
             terminated; a still-growing tag_provider count means it kept spinning \
             (F-01 busy-loop regression): {after_drop} then {later}"
        );
    }

    /// Finding F-02 through the public `spawn` API: a handler panic surfaces as
    /// the spawned task's `JoinError` and must come back out of `stop()`'s
    /// `Result` rather than being silently discarded.
    #[tokio::test]
    async fn spawn_stop_surfaces_join_error_instead_of_swallowing_it() {
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());

        let spec = ProjectionSpec::new(
            "proj",
            || vec![(EventTag::new("tenant-a"), "tenant-a".to_string())],
            PanickingHandler,
            FakeStore::default(),
            FakeDedup,
            FakeOffset,
        )
        .interval(Duration::from_millis(5));

        let handle = scheduler.spawn(spec);

        // Give the loop time to actually hit the panic before we stop it.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let result = handle.stop().await;
        assert!(
            result.as_ref().is_err_and(|e| e.is_panic()),
            "expected the handler panic to surface as a JoinError from stop(), got {result:?}"
        );
    }

    /// Store that records the `(tenant, tag)` every `fetch` receives, so a
    /// test can prove which tenant the scheduler threads for each tag.
    #[derive(Clone, Default)]
    struct RecordingStore {
        seen: Arc<StdMutex<Vec<(String, String)>>>,
    }

    #[async_trait]
    impl ReadSideStore<serde_json::Value> for RecordingStore {
        async fn fetch(
            &self,
            tenant: &str,
            tag: &EventTag,
            _offset: Option<&Offset>,
            _batch_size: usize,
        ) -> Result<Vec<EventStreamElement<serde_json::Value>>, ReadSideStoreError> {
            self.seen
                .lock()
                .unwrap()
                .push((tenant.to_string(), tag.value().to_string()));
            Ok(Vec::new())
        }
    }

    /// Each `(tag, tenant)` pair threads ITS OWN tenant into
    /// `ReadSideStore::fetch` — the scheduler no longer collapses every tag
    /// onto one shared tenant. Two different tenants in a single
    /// `start_projection` batch must each reach the store paired with their
    /// own tag.
    #[tokio::test]
    async fn start_projection_threads_each_pairs_tenant_into_fetch() {
        let store = RecordingStore::default();
        let mut scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());

        scheduler
            .start_projection(
                "proj".to_string(),
                vec![
                    (
                        EventTag::new("users-by-tenant:tenant-a"),
                        "tenant-a".to_string(),
                    ),
                    (
                        EventTag::new("users-by-tenant:tenant-b"),
                        "tenant-b".to_string(),
                    ),
                ],
                CountingHandler {
                    handled: Arc::new(AtomicUsize::new(0)),
                },
                store.clone(),
                FakeDedup,
                FakeOffset,
                NoopProgressReporter,
            )
            .await
            .expect("start_projection succeeds");

        let seen = store.seen.lock().unwrap().clone();
        assert!(
            seen.contains(&(
                "tenant-a".to_string(),
                "users-by-tenant:tenant-a".to_string()
            )),
            "tenant-a's tag must reach fetch paired with tenant-a, got: {seen:?}"
        );
        assert!(
            seen.contains(&(
                "tenant-b".to_string(),
                "users-by-tenant:tenant-b".to_string()
            )),
            "tenant-b's tag must reach fetch paired with tenant-b, got: {seen:?}"
        );
    }
}
