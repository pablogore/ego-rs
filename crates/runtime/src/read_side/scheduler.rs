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
    /// Signals the loop to stop and waits, bounded by `deadline`, for the
    /// in-flight batch to drain. On timeout the task is **aborted and then
    /// awaited**, not dropped — dropping a `JoinHandle` detaches the task in
    /// Tokio rather than cancelling it, which would leave the loop polling
    /// past a shutdown that already reported `TimedOut`.
    ///
    /// A panic still surfaces via [`ReadSideStopOutcome::Panicked`] rather
    /// than being swallowed — the explicit callback to CORE-018's Finding
    /// F-02, where a spawned scheduler task's panic used to vanish silently.
    /// This is the one deliberate difference from the sibling retention
    /// workers' `stop`, which isolate a panic instead of propagating it.
    pub async fn stop(self, deadline: Duration) -> ReadSideStopOutcome {
        let _ = self.stop_tx.send(true);

        let mut task = self.task;
        match tokio::time::timeout(deadline, &mut task).await {
            Ok(Ok(())) => ReadSideStopOutcome::Stopped,
            Ok(Err(joined)) => ReadSideStopOutcome::Panicked(joined),
            Err(_) => {
                task.abort();
                let _ = task.await;
                ReadSideStopOutcome::TimedOut
            }
        }
    }
}

/// What a [`ReadSideProjectionHandle::stop`] observed.
#[derive(Debug)]
pub enum ReadSideStopOutcome {
    /// The loop acknowledged the stop signal and exited within the deadline.
    Stopped,
    /// The loop's task panicked — surfaced, not swallowed (CORE-018 F-02).
    Panicked(tokio::task::JoinError),
    /// The loop did not exit within the deadline, so it was **aborted and
    /// then awaited** — reported only once the task is genuinely gone.
    TimedOut,
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
    /// point for launching a persistent projection poll loop: the spec groups
    /// the arguments and defaults the boilerplate, and `spawn` creates the
    /// `watch` stop channel internally so the caller never has to wire it.
    /// (The [`TagScheduler::start_projection`] trait method remains public but
    /// processes a single batch per call — it is the primitive `spawn` loops
    /// over, not a lifecycle entry point.)
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
    // These tests are deterministic and event-driven: the loop's collaborators
    // (tag_provider, handler, reporter, store) signal over `mpsc` channels, and
    // each test awaits those signals under a generous, load-immune timeout. They
    // deliberately avoid "sleep a fixed window, then count iterations / assert an
    // elapsed budget" — that pattern conflates loop liveness with machine load
    // and flakes under `cargo test --workspace` (see issue #224).
    use super::*;
    use ego_domain::read_side::event_stream::EventStreamElement;
    use ego_domain::read_side::offset::Offset;
    use ego_domain::read_side::progress::NoopProgressReporter;
    use ego_domain::read_side::store::ReadSideStoreError;
    use std::sync::Mutex as StdMutex;
    use tokio::sync::mpsc;

    /// A per-recv ceiling on how long we wait for an expected signal. A healthy
    /// loop (millisecond interval) produces its next signal near-instantly; this
    /// is only a backstop so a genuinely stuck/regressed loop fails as a timeout
    /// instead of hanging forever.
    const SIGNAL_TIMEOUT: Duration = Duration::from_secs(5);

    /// Narrow poll interval so the loop turns over quickly within each test.
    const FAST_INTERVAL: Duration = Duration::from_millis(1);

    /// Fake store that returns no events for empty tag lists (fast path used by
    /// the "fresh tag_provider per iteration" / termination tests) and one real
    /// event when given a tag (used by the reporter test).
    #[derive(Clone, Default)]
    struct FakeStore;

    #[async_trait]
    impl ReadSideStore<serde_json::Value> for FakeStore {
        async fn fetch(
            &self,
            _tenant: &str,
            tag: &EventTag,
            _offset: Option<&Offset>,
            _batch_size: usize,
        ) -> Result<Vec<EventStreamElement<serde_json::Value>>, ReadSideStoreError> {
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

    /// Store that signals the instant a fetch begins, then blocks on a
    /// caller-controlled delay, so a test can deterministically observe a batch
    /// being *in flight* and request stop while it still is.
    #[derive(Clone)]
    struct InFlightSignalingStore {
        started_tx: mpsc::UnboundedSender<()>,
        delay: Duration,
    }

    #[async_trait]
    impl ReadSideStore<serde_json::Value> for InFlightSignalingStore {
        async fn fetch(
            &self,
            _tenant: &str,
            tag: &EventTag,
            _offset: Option<&Offset>,
            _batch_size: usize,
        ) -> Result<Vec<EventStreamElement<serde_json::Value>>, ReadSideStoreError> {
            let _ = self.started_tx.send(());
            tokio::time::sleep(self.delay).await;
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

    /// Handler that succeeds silently — used where the tag stream is empty (so
    /// it is never actually invoked) or where the handler is not the subject.
    #[derive(Clone)]
    struct NoopHandler;

    #[async_trait]
    impl Handler<serde_json::Value> for NoopHandler {
        async fn handle(
            &self,
            _events: &[EventStreamElement<serde_json::Value>],
        ) -> Result<(), ego_domain::read_side::error::ProjectionError> {
            Ok(())
        }
    }

    /// Handler that signals each time it runs, so a test can prove a batch's
    /// handler actually executed (e.g. that an in-flight batch was drained).
    #[derive(Clone)]
    struct SignalHandler {
        ran_tx: mpsc::UnboundedSender<()>,
    }

    #[async_trait]
    impl Handler<serde_json::Value> for SignalHandler {
        async fn handle(
            &self,
            _events: &[EventStreamElement<serde_json::Value>],
        ) -> Result<(), ego_domain::read_side::error::ProjectionError> {
            let _ = self.ran_tx.send(());
            Ok(())
        }
    }

    /// Handler that signals it was entered and then panics — used to prove
    /// `stop()` surfaces the resulting `JoinError` instead of swallowing it
    /// (CORE-018 Finding F-02). The signal removes any need to "sleep to let the
    /// panic happen": the test waits for the signal, so the panic is guaranteed
    /// to have been reached before it stops.
    #[derive(Clone)]
    struct PanickingHandler {
        entered_tx: mpsc::UnboundedSender<()>,
    }

    #[async_trait]
    impl Handler<serde_json::Value> for PanickingHandler {
        async fn handle(
            &self,
            _events: &[EventStreamElement<serde_json::Value>],
        ) -> Result<(), ego_domain::read_side::error::ProjectionError> {
            let _ = self.entered_tx.send(());
            panic!("deliberate handler panic for spawn JoinError test");
        }
    }

    /// Progress reporter that signals every completed batch, so a test can prove
    /// a custom reporter was actually driven through the session machinery.
    #[derive(Clone)]
    struct ChannelReporter {
        reported_tx: mpsc::UnboundedSender<()>,
    }

    impl ProgressReporter for ChannelReporter {
        fn on_batch_completed(
            &self,
            _projection_id: &str,
            _tag: &EventTag,
            _count: usize,
            _offset: &Offset,
        ) {
            let _ = self.reported_tx.send(());
        }
    }

    /// Awaits one signal from `rx` under [`SIGNAL_TIMEOUT`], panicking with
    /// `context` on timeout (loop never produced the signal) or on channel close
    /// (the loop terminated unexpectedly early).
    async fn expect_signal(rx: &mut mpsc::UnboundedReceiver<()>, context: &str) {
        tokio::time::timeout(SIGNAL_TIMEOUT, rx.recv())
            .await
            .unwrap_or_else(|_| panic!("{context}: no signal within {SIGNAL_TIMEOUT:?}"))
            .unwrap_or_else(|| panic!("{context}: channel closed before the signal arrived"));
    }

    /// (a) `tag_provider` is called fresh every iteration — proven by receiving
    /// several distinct invocation signals rather than sleeping a window and
    /// counting. (b) stops gracefully: `stop()` joins the loop task without a
    /// panic. The spec touches none of the defaulted fields (reporter/on_error),
    /// proving the default path is wired correctly.
    #[tokio::test]
    async fn spawn_with_default_spec_calls_tag_provider_fresh_each_iteration_and_stops_gracefully()
    {
        let (calls_tx, mut calls_rx) = mpsc::unbounded_channel::<()>();
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());

        let spec = ProjectionSpec::new(
            "proj",
            move || {
                let _ = calls_tx.send(());
                Vec::new() // empty tags: start_projection returns instantly
            },
            NoopHandler,
            FakeStore,
            FakeDedup,
            FakeOffset,
        )
        .interval(FAST_INTERVAL);

        let handle = scheduler.spawn(spec);

        // Fresh call per iteration: require several distinct invocations.
        for i in 0..3 {
            expect_signal(&mut calls_rx, &format!("tag_provider iteration {i}")).await;
        }

        assert!(matches!(
            handle.stop(SIGNAL_TIMEOUT).await,
            ReadSideStopOutcome::Stopped
        ));
    }

    /// The `ProjectionSpec::reporter` type-changing builder swaps the default
    /// `NoopProgressReporter` for a caller-supplied reporter, and `spawn` must
    /// actually drive it through the session machinery — proven by the reporter
    /// signalling at least one completed batch for a real (non-empty) tag stream.
    #[tokio::test]
    async fn spawn_with_custom_reporter_spec_drives_the_reporter() {
        let (reported_tx, mut reported_rx) = mpsc::unbounded_channel::<()>();
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());

        let spec = ProjectionSpec::new(
            "proj",
            || vec![(EventTag::new("tenant-a"), "tenant-a".to_string())],
            NoopHandler,
            FakeStore,
            FakeDedup,
            FakeOffset,
        )
        .interval(FAST_INTERVAL)
        .reporter(ChannelReporter { reported_tx });

        let handle = scheduler.spawn(spec);

        expect_signal(&mut reported_rx, "custom reporter driven").await;

        assert!(matches!(
            handle.stop(SIGNAL_TIMEOUT).await,
            ReadSideStopOutcome::Stopped
        ));
    }

    /// `stop()` drains any in-flight batch before returning: stop is requested
    /// while a fetch is provably still in flight, and the batch's handler must
    /// still run to completion — proving the loop awaited the in-progress batch
    /// (the stop check only happens between iterations) rather than aborting it.
    #[tokio::test]
    async fn spawn_stop_drains_in_flight_batch_before_returning() {
        let (started_tx, mut started_rx) = mpsc::unbounded_channel::<()>();
        let (ran_tx, mut ran_rx) = mpsc::unbounded_channel::<()>();
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());

        let spec = ProjectionSpec::new(
            "proj",
            || vec![(EventTag::new("tenant-a"), "tenant-a".to_string())],
            SignalHandler { ran_tx },
            InFlightSignalingStore {
                started_tx,
                // Simulated fetch latency: only needs to keep the batch in flight
                // across the stop request. Not a timed assertion — its exact value
                // never gates the test.
                delay: Duration::from_millis(50),
            },
            FakeDedup,
            FakeOffset,
        )
        .interval(FAST_INTERVAL);

        let handle = scheduler.spawn(spec);

        // The batch is now provably in flight (fetch started, mid-delay).
        expect_signal(&mut started_rx, "fetch in flight").await;

        // Request stop mid-batch and await the loop's task. If the in-flight
        // batch was drained (not aborted), its handler ran — proven by the signal.
        assert!(matches!(
            handle.stop(SIGNAL_TIMEOUT).await,
            ReadSideStopOutcome::Stopped
        ));
        expect_signal(
            &mut ran_rx,
            "in-flight batch handler must run (drained, not aborted)",
        )
        .await;
    }

    /// Post-review Finding F-01, through the public `spawn` API: dropping the
    /// [`ReadSideProjectionHandle`] without calling `stop()` drops the internal
    /// `watch::Sender`, and the loop must observe that disconnection and
    /// terminate — not spin unbounded (before the fix, the discarded `Err` from
    /// `changed()` on a closed channel bypassed the interval into a busy loop).
    ///
    /// Deterministic proof via channel closure: the `tag_provider` closure holds
    /// the only sender, so once the loop terminates and drops the closure the
    /// channel closes and `recv()` yields `None`. A busy-loop regression would
    /// instead keep sending unboundedly; the drain is bounded so that case fails
    /// fast instead of hanging.
    #[tokio::test]
    async fn spawn_terminates_when_handle_is_dropped_without_stopping() {
        let (calls_tx, mut calls_rx) = mpsc::unbounded_channel::<()>();
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());

        let spec = ProjectionSpec::new(
            "proj",
            move || {
                let _ = calls_tx.send(());
                Vec::new()
            },
            NoopHandler,
            FakeStore,
            FakeDedup,
            FakeOffset,
        )
        .interval(FAST_INTERVAL);

        let handle = scheduler.spawn(spec);

        // Confirm the loop is actively iterating before dropping the handle.
        for _ in 0..2 {
            expect_signal(&mut calls_rx, "loop iterating before drop").await;
        }

        // Drop WITHOUT stopping — the disconnected stop sender must end the loop.
        drop(handle);

        // A terminated loop drops the tag_provider (last sender) → channel
        // closes → recv() eventually yields None. Bound the drain so a
        // busy-loop regression trips the cap instead of looping here forever.
        const BUSY_LOOP_CAP: usize = 1000;
        let mut drained = 0usize;
        loop {
            match tokio::time::timeout(SIGNAL_TIMEOUT, calls_rx.recv()).await {
                Ok(Some(())) => {
                    drained += 1;
                    assert!(
                        drained < BUSY_LOOP_CAP,
                        "loop kept emitting {BUSY_LOOP_CAP}+ times after the handle was dropped \
                         — it did not terminate (F-01 busy-loop regression)"
                    );
                }
                // Channel closed: the loop terminated and dropped tag_provider.
                Ok(None) => break,
                Err(_) => panic!(
                    "loop neither terminated nor closed its channel within {SIGNAL_TIMEOUT:?} \
                     after the handle was dropped (F-01)"
                ),
            }
        }
    }

    /// Finding F-02 through the public `spawn` API: a handler panic surfaces as
    /// the spawned task's `JoinError` and must come back out of `stop()`'s
    /// `Result` rather than being silently discarded.
    #[tokio::test]
    async fn spawn_stop_surfaces_join_error_instead_of_swallowing_it() {
        let (entered_tx, mut entered_rx) = mpsc::unbounded_channel::<()>();
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());

        let spec = ProjectionSpec::new(
            "proj",
            || vec![(EventTag::new("tenant-a"), "tenant-a".to_string())],
            PanickingHandler { entered_tx },
            FakeStore,
            FakeDedup,
            FakeOffset,
        )
        .interval(FAST_INTERVAL);

        let handle = scheduler.spawn(spec);

        // The handler signals immediately before panicking, so once we observe
        // this the task is guaranteed to panic — no fixed sleep needed.
        expect_signal(&mut entered_rx, "panicking handler reached").await;

        match handle.stop(SIGNAL_TIMEOUT).await {
            ReadSideStopOutcome::Panicked(e) => {
                assert!(e.is_panic(), "expected a panic JoinError, got {e:?}")
            }
            other => panic!("expected Panicked, got {other:?}"),
        }
    }

    /// A loop parked mid-batch in an await that never resolves must not block
    /// `stop()` forever: past a deadline the task is aborted (dropping it at
    /// that await point) and awaited, and the outcome reports `TimedOut`
    /// rather than hanging the caller.
    #[tokio::test]
    async fn spawn_stop_times_out_and_aborts_a_hung_loop() {
        let (started_tx, mut started_rx) = mpsc::unbounded_channel::<()>();
        let scheduler = TagSchedulerImpl::<serde_json::Value>::new(ReadSideConfig::default());

        let spec = ProjectionSpec::new(
            "proj",
            || vec![(EventTag::new("tenant-a"), "tenant-a".to_string())],
            NoopHandler,
            InFlightSignalingStore {
                started_tx,
                // Never resolves within the test's lifetime: proves the
                // timeout path, not the drain path.
                delay: Duration::from_secs(u64::MAX),
            },
            FakeDedup,
            FakeOffset,
        )
        .interval(FAST_INTERVAL);

        let handle = scheduler.spawn(spec);

        // The batch is now provably parked inside the never-resolving fetch.
        expect_signal(&mut started_rx, "fetch in flight").await;

        let outcome = tokio::time::timeout(SIGNAL_TIMEOUT, handle.stop(Duration::from_millis(50)))
            .await
            .expect("stop() must itself return within its own deadline, not hang the caller");

        assert!(
            matches!(outcome, ReadSideStopOutcome::TimedOut),
            "expected TimedOut for a hung loop, got {outcome:?}"
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
                NoopHandler,
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
