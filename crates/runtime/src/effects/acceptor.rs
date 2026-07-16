//! `RuntimeEffectAcceptor` (CORE-019 Phase 5): the `ego-runtime` implementation
//! of persistent-entity's [`EffectAcceptor`] port (AD-3). Mints the effect
//! id, attaches the established tenant, records the effect in the configured
//! [`EffectStateStore`] under a bounded retry policy (AD-9's classification —
//! only `TemporarilyUnavailable` is retried), and hands it to the one
//! delivery pipeline: `Deferred` enqueues for a separately-spawned
//! [`DeliveryRunner::run`] loop; `Inline` drains the same `drain_one` step
//! synchronously on the caller's task (design.md §7).

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
use persistent_entity::effect_acceptor::{EffectAcceptanceError, EffectAcceptor};
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;

use super::observability::log_accepted;
use super::policy::{DeliveryConfig, RetryPolicy, RunnerMode};
use super::queue::{EffectQueue, EffectQueueReceiver};
use super::registry::ExecutorRegistry;
use super::runner::DeliveryRunner;
use super::store::{AcceptedEffect, EffectDedupStore, EffectId, EffectStateStore, EffectStoreError};

/// **F-02 (PR3 round 3 review, BLOCKER fix):** admission-gating lifecycle
/// state, shared (via `Arc`) between [`RuntimeEffectAcceptor`] and
/// [`EffectRuntimeHandle`] — the single source of truth for whether a NEW
/// `accept()` call is admitted at all, and how many are currently in flight.
/// Closes two gaps the F-01 unified drain does not: (a) a new `accept()`
/// call starting AFTER shutdown has begun used to have nothing rejecting it
/// outright; (b) `shutdown_and_wait` used to return without ever waiting for
/// an `accept()` call that was already in flight when shutdown began
/// (reproducible even in `Inline` mode, where there is no runner task at all
/// to await).
struct LifecycleGate {
    state: StdMutex<LifecycleState>,
    /// **F-03 (PR3 round 4 review, BLOCKER fix):** replaces the former
    /// `AtomicU64` + `Notify` pair. `Notify::notify_waiters()` only wakes
    /// waiters already registered at the exact moment it is called — unlike
    /// a state-carrying primitive, it leaves nothing behind for a later
    /// `.notified()` call. [`InFlightGuard::drop`] decrementing to zero and
    /// calling `notify_waiters()` in the narrow window before
    /// [`Self::wait_until_drained`]'s own `.notified()` future was
    /// constructed/polled would lose that wakeup entirely, burning the whole
    /// deadline despite the count already being genuinely zero.
    /// `watch::Receiver::borrow()`/`changed()` always reflect the latest
    /// sent value regardless of exactly when they are called relative to the
    /// send, so there is no equivalent lost-wakeup window.
    in_flight: watch::Sender<u64>,
}

enum LifecycleState {
    Running,
    Draining { deadline: tokio::time::Instant },
    Closed,
}

/// RAII guard: decrements [`LifecycleGate::in_flight`] on drop (success,
/// failure, or panic-unwind alike) so an early `?` return inside `accept()`
/// can never leak an in-flight count. Notifies [`LifecycleGate::drained`]
/// once the count reaches zero.
struct InFlightGuard {
    gate: Arc<LifecycleGate>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.gate.in_flight.send_modify(|n| *n -= 1);
    }
}

impl LifecycleGate {
    fn new() -> Arc<Self> {
        let (in_flight, _) = watch::channel(0u64);
        Arc::new(Self {
            state: StdMutex::new(LifecycleState::Running),
            in_flight,
        })
    }

    /// Admits a new `accept()` call: `Err(())` if `Draining`/`Closed` — the
    /// caller must reject immediately, before minting anything. `Ok` holds
    /// the `state` lock for its whole duration, so this can never race
    /// [`Self::begin_draining`]: either the increment happens strictly
    /// before the transition to `Draining` (and is therefore counted by
    /// [`Self::wait_until_drained`]), or strictly after (and is rejected).
    fn enter(self: &Arc<Self>) -> Result<InFlightGuard, ()> {
        let state = self.state.lock().unwrap();
        match &*state {
            LifecycleState::Running => {
                self.in_flight.send_modify(|n| *n += 1);
                Ok(InFlightGuard { gate: self.clone() })
            }
            LifecycleState::Draining { .. } | LifecycleState::Closed => Err(()),
        }
    }

    fn begin_draining(&self, deadline: tokio::time::Instant) {
        *self.state.lock().unwrap() = LifecycleState::Draining { deadline };
    }

    fn close(&self) {
        *self.state.lock().unwrap() = LifecycleState::Closed;
    }

    /// Waits for every `accept()` call already in flight when draining began
    /// to finish, bounded by the deadline instant [`Self::begin_draining`]
    /// recorded — the same instant already published on `deadline_rx`, which
    /// is exactly what causes those in-flight calls to actually finish
    /// (naturally, or cut short by `wait_for_deadline`). The `sleep_until`
    /// race here is a defensive backstop, not the primary mechanism. A no-op
    /// if `begin_draining` was never called.
    async fn wait_until_drained(&self) {
        let Some(deadline_instant) = (match &*self.state.lock().unwrap() {
            LifecycleState::Draining { deadline } => Some(*deadline),
            LifecycleState::Running | LifecycleState::Closed => None,
        }) else {
            return;
        };
        // F-03: a `watch::Receiver` always reflects the latest sent value
        // regardless of exactly when `borrow()`/`changed()` are called
        // relative to the corresponding `send`/`send_modify` — no lost
        // wakeup, unlike the former `Notify`-based implementation.
        let mut in_flight_rx = self.in_flight.subscribe();
        loop {
            if *in_flight_rx.borrow() == 0 {
                return;
            }
            tokio::select! {
                result = in_flight_rx.changed() => {
                    if result.is_err() {
                        // Sender dropped (the `LifecycleGate` itself is
                        // gone) — nothing further will ever change.
                        return;
                    }
                }
                _ = tokio::time::sleep_until(deadline_instant) => return,
            }
        }
    }
}

/// **F-02 (PR3 round 3 review):** the message `accept()` reports when it
/// rejects a call outright because the acceptor is already `Draining`/
/// `Closed` — no minting, no store call, no enqueue ever happens for it.
const SHUTDOWN_REJECTED_MSG: &str = "effect acceptor is shutting down; new acceptance calls are rejected";

/// Owns both the shutdown signal AND the spawned `Deferred`-mode runner
/// task's `JoinHandle` (F-01, PR3 review). Returned by
/// [`RuntimeEffectAcceptor::start`] — the only way to actually await the
/// spawned drain loop's real completion (as opposed to merely sending the
/// shutdown signal and hoping). PR4's lifecycle wiring holds this, not a
/// bare `watch::Sender<bool>`.
pub struct EffectRuntimeHandle {
    shutdown: watch::Sender<bool>,
    /// F-02 (PR3 round 2 review): the shared *deadline instant* signal —
    /// see [`RuntimeEffectAcceptor`]'s `deadline_tx`/`deadline_rx` fields for
    /// the full rationale. `None` until [`Self::shutdown_and_wait`] sets it.
    deadline: watch::Sender<Option<tokio::time::Instant>>,
    /// `None` in `Inline` mode — no loop is ever spawned there, so there is
    /// nothing to await.
    runner_task: Option<JoinHandle<()>>,
    /// **F-01 (PR3 round 3 review, BLOCKER fix):** shared with
    /// [`RuntimeEffectAcceptor`] — the authoritative
    /// [`DeliveryRunner::shutdown_and_drain`] call in
    /// [`Self::shutdown_and_wait`] runs directly against this `Arc`,
    /// regardless of whether `runner_task` even exists (`Inline` mode) or
    /// has already been aborted.
    runner: Arc<DeliveryRunner>,
    /// **F-02 (PR3 round 3 review, BLOCKER fix):** shared with
    /// [`RuntimeEffectAcceptor::accept`] — see [`LifecycleGate`].
    lifecycle: Arc<LifecycleGate>,
}

/// Why [`EffectRuntimeHandle::shutdown_and_wait`] did not observe a clean,
/// on-time finish of the spawned `Deferred`-mode runner task.
#[derive(Debug, Error)]
pub enum EffectRuntimeShutdownError {
    /// **PR3 round 6 review follow-up:** returned not only when `deadline`
    /// elapsed before the outer runner task finished, but also whenever
    /// [`DeliveryRunner::shutdown_and_drain`] reports it could not drain
    /// naturally — the deadline was already exhausted going into the drain,
    /// or a dispatch task/executor attempt had to be forcibly aborted. Both
    /// cases mean the shutdown contract was not fully honored within budget,
    /// in either `Inline` or `Deferred` mode.
    #[error("effect runtime shutdown deadline elapsed before the runner task finished")]
    Timeout,
    /// The runner task panicked.
    #[error("effect runtime runner task panicked during shutdown")]
    RunnerPanicked,
    /// The runner task was cancelled/aborted before it finished.
    #[error("effect runtime runner task was cancelled before finishing shutdown")]
    RunnerCancelled,
}

impl EffectRuntimeHandle {
    /// Signals shutdown, then awaits the spawned runner task's *actual*
    /// completion — never just that the signal was sent — bounded by
    /// `deadline`. Resolves immediately once the signal is sent in `Inline`
    /// mode, since no task was ever spawned there.
    ///
    /// F-02 (PR3 round 2 review): also publishes the deadline *instant*
    /// (`Instant::now() + deadline`) on the shared `deadline` signal — this
    /// is what [`RuntimeEffectAcceptor::accept_into_store`]/`send_to_queue`
    /// race in-flight acceptance work against, so that work is only
    /// cancelled once this instant is actually reached, not the moment
    /// shutdown merely begins.
    ///
    /// F-01 (PR3 round 2 review, BLOCKER fix): on timeout, the still-running
    /// `runner_task` is explicitly `abort()`-ed and drained rather than
    /// simply dropped — dropping a `JoinHandle` only detaches from the task,
    /// it does NOT cancel it, which used to leave the runner running forever
    /// in the background even after this function returned `Timeout`.
    ///
    /// **F-01/F-02 (PR3 round 3 review, BLOCKERs, unified):** this is the one
    /// coherent shutdown sequence, not several independently-bolted-on
    /// mechanisms:
    /// 1. Close admission (`lifecycle.begin_draining`) — any `accept()` call
    ///    starting from this point on is rejected outright.
    /// 2. Wait for `accept()` calls already in flight to finish naturally,
    ///    bounded by `deadline` ([`LifecycleGate::wait_until_drained`]) — this
    ///    is what used to be missing entirely, provably so even in `Inline`
    ///    mode (no `runner_task` to await at all).
    /// 3. Tell the runner to stop consuming (`shutdown.send(true)`).
    /// 4. Await/abort the outer `runner_task` — **PR3 round 5:** now runs
    ///    quickly, since `DeliveryRunner::run_inner` no longer drains
    ///    anything of its own on the way out (see below).
    /// 5. Run the runner's own drain ([`DeliveryRunner::shutdown_and_drain`])
    ///    directly against the shared `Arc<DeliveryRunner>` — the ONE
    ///    guarantee that every runner-owned child task (per-effect dispatch
    ///    tasks, in-flight executor calls) is gone. **PR3 round 6:** its
    ///    returned `bool` (drained naturally vs. forced/timed out) is now
    ///    folded into the final `Result` — see below.
    /// 6. Close the lifecycle (`lifecycle.close`) regardless of outcome.
    ///
    /// **F-01 (PR3 round 6 review, BLOCKER fix):** the final `Result` used to
    /// come only from step 4's `runner_task_result`, discarding step 5's
    /// outcome entirely — `Ok(())` even when the drain step had to
    /// force-abort a hung executor attempt or itself exhausted the deadline
    /// (reproducible in BOTH `Inline` mode, where `runner_task_result` is
    /// unconditionally `Ok(())`, and `Deferred` mode, where the outer runner
    /// task can finish `Ok` quickly while its own spawned child tasks still
    /// have to be forced out during step 5). Fixed: `Ok(())` now requires
    /// BOTH the runner task finishing cleanly AND step 5 reporting a natural,
    /// on-time drain; otherwise the result is `Err(Timeout)`.
    ///
    /// **F-01 (PR3 round 4 review, BLOCKER fix — reordered):** step 1 above
    /// used to run AFTER `shutdown.send(true)` had already told
    /// `DeliveryRunner::run_inner` to stop consuming its receive loop
    /// immediately — so an `accept()` call already mid-persistence/backoff
    /// when shutdown began could reach `queue.send` only once nothing was
    /// left consuming, durably accepting and successfully enqueueing an
    /// effect that then sat stranded, never dispatched. `shutdown.send(true)`
    /// now fires ONLY after step 2 (`wait_until_drained`) returns — by
    /// construction, every acceptance admitted before draining began has, by
    /// then, either finished enqueueing (while the runner was still
    /// guaranteed to be consuming) or been cut short by the same deadline.
    ///
    /// **F-01 (PR3 round 5 review, BLOCKER fix — leader/follower removed,
    /// steps 4/5 reordered):** round 4 had this function call
    /// `runner.shutdown_and_drain` BEFORE awaiting/aborting `runner_task`,
    /// coordinated via a leader/follower election inside `DeliveryRunner`
    /// (whichever of `run_inner`'s own end-of-loop drain or this call reached
    /// it first became the leader). That was unsafe: `run_inner` runs INSIDE
    /// `runner_task`, so if it ever won the leader race, THIS function's own
    /// later `runner_task.abort()` (on timeout) could kill the leader
    /// mid-drain, abandoning its cleanup before it ever aborted the hung
    /// executor attempt it was working on. Fixed ("Option A"): `run_inner`
    /// no longer drains anything itself at all — it only stops consuming and
    /// returns — so awaiting it here (step 4) is now cheap and safe, and
    /// `DeliveryRunner::shutdown_and_drain` (step 5), called ONLY from here,
    /// is the SOLE cleanup authority; there is no leader/follower ambiguity
    /// left for a timeout-triggered abort to race against. The caller's
    /// deadline is honestly split between the two phases — awaiting
    /// `runner_task` is bounded by at most half of whatever time remains, so
    /// a slow-to-return `runner_task` can't silently consume the entire
    /// budget and leave nothing for the actual drain; the drain step then
    /// gets whatever remains of the ORIGINAL deadline, never a further-
    /// reduced one. RED test (`acceptor.rs`):
    /// `shutdown_and_wait_stops_a_hung_executor_task_even_when_run_inner_would_have_raced_it_for_drain_leadership`.
    pub async fn shutdown_and_wait(
        self,
        deadline: Duration,
    ) -> Result<(), EffectRuntimeShutdownError> {
        let elapses_at = tokio::time::Instant::now() + deadline;

        // **F-01 (PR3 round 4 review, BLOCKER fix):** admission must close
        // and every acceptance already in flight must finish enqueueing
        // BEFORE the runner is ever told to stop consuming. The previous
        // ordering sent `shutdown.send(true)` FIRST — causing
        // `DeliveryRunner::run_inner` to abandon its receive loop
        // immediately — while an `accept()` call already in flight could
        // still be mid-persistence/backoff; by the time it finally reached
        // `queue.send`, the runner had already stopped consuming, so the
        // effect was durably accepted and successfully enqueued yet never
        // actually dispatched. Publishing the deadline and closing admission
        // first, then waiting for every already-admitted acceptance to
        // finish (bounded by that same deadline), guarantees the runner is
        // still consuming for as long as any admitted acceptance could still
        // be enqueueing.
        let _ = self.deadline.send(Some(elapses_at));
        self.lifecycle.begin_draining(elapses_at);
        self.lifecycle.wait_until_drained().await;

        // Only now may the runner stop consuming — every acceptance
        // admitted before draining began has, by this point, either
        // finished enqueueing or been cut short by the deadline above.
        let _ = self.shutdown.send(true);

        // **PR3 round 5:** split the remaining budget so awaiting
        // `run_inner`'s own (now drain-free, should-return-quickly) loop
        // exit can never silently consume the caller's entire deadline and
        // leave nothing for the real cleanup below. `run_inner` no longer
        // does any draining of its own, so half of whatever remains is
        // generous; the actual drain step still gets the full remainder of
        // the ORIGINAL `elapses_at`, not a further-reduced budget.
        let now = tokio::time::Instant::now();
        let remaining = elapses_at.saturating_duration_since(now);
        let runner_task_deadline = now + remaining / 2;

        let runner_task_result = match self.runner_task {
            None => Ok(()),
            Some(mut runner_task) => {
                match tokio::time::timeout_at(runner_task_deadline, &mut runner_task).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(join_err)) if join_err.is_panic() => {
                        Err(EffectRuntimeShutdownError::RunnerPanicked)
                    }
                    Ok(Err(_)) => Err(EffectRuntimeShutdownError::RunnerCancelled),
                    Err(_) => {
                        runner_task.abort();
                        let _ = runner_task.await;
                        Err(EffectRuntimeShutdownError::Timeout)
                    }
                }
            }
        };

        // The SOLE cleanup authority: aborts any still-running executor
        // attempts and drains the tracked `JoinSet`, bounded by whatever
        // remains of the caller's ORIGINAL deadline.
        //
        // **F-01 (PR3 round 6 review, BLOCKER fix):** `shutdown_and_drain`'s
        // return value used to be discarded entirely, so this function's
        // final `Result` came ONLY from `runner_task_result` above — which is
        // unconditionally `Ok(())` in `Inline` mode (no `runner_task` exists
        // at all) and can finish `Ok` quickly in `Deferred` mode even while
        // the drain step below still has to force-abort a hung executor
        // attempt or exhaust the deadline. Both cases used to report a false,
        // clean `Ok(())` despite work having been forcibly cancelled. Now
        // folded honestly into the final `Result`: only `Ok(())` when the
        // runner task itself finished cleanly AND the drain reports nothing
        // had to be forced/timed out; otherwise `Err(Timeout)` — reusing the
        // existing variant, since a pre-existing `runner_task_result` error
        // (panicked/cancelled) already carries its own, more specific cause
        // and is left untouched.
        let drained_cleanly = self.runner.shutdown_and_drain(elapses_at).await;

        self.lifecycle.close();

        match runner_task_result {
            Ok(()) if drained_cleanly => Ok(()),
            Ok(()) => Err(EffectRuntimeShutdownError::Timeout),
            Err(err) => Err(err),
        }
    }
}

/// F-02 (PR3 round 2 review, BLOCKER fix): resolves once `deadline_rx`'s
/// value is `Some(instant)` **and** `instant` has actually elapsed — never
/// merely the moment the value transitions from `None` to `Some`. That
/// distinction is exactly what separates "shutdown has started" (safe for
/// in-flight acceptance work to ignore) from "the drain deadline is up"
/// (the only point in-flight acceptance work must actually be cancelled at).
///
/// Deliberately does NOT use `watch::Receiver::wait_for` here: its `Ref`
/// guard type is not `Send`, which — once awaited inside `tokio::select!` —
/// would poison the whole enclosing `async fn`'s generated future as
/// `!Send`, breaking `#[async_trait]`'s required
/// `Pin<Box<dyn Future<Output = _> + Send>>` for [`EffectAcceptor::accept`].
/// `borrow()`'s guard is only ever read from inside a single expression,
/// never held across an `.await`, so it never taints anything.
async fn wait_for_deadline(deadline_rx: &mut watch::Receiver<Option<tokio::time::Instant>>) {
    loop {
        let current = *deadline_rx.borrow();
        if let Some(instant) = current {
            tokio::time::sleep_until(instant).await;
            return;
        }
        if deadline_rx.changed().await.is_err() {
            // Sender dropped without a deadline ever being set — nothing
            // further will ever change; there's nothing productive left to
            // wait for.
            return;
        }
    }
}

/// `ego-runtime`'s [`EffectAcceptor`] implementation (design.md §2:
/// "Internal runtime" — not a public extension point third parties
/// re-implement, but constructible from any crate depending on
/// `ego-runtime`).
pub struct RuntimeEffectAcceptor {
    state: Arc<dyn EffectStateStore>,
    accept_retry: RetryPolicy,
    queue: EffectQueue,
    runner: Arc<DeliveryRunner>,
    runner_mode: RunnerMode,
    /// Present only in `Inline` mode: this acceptor is the sole consumer of
    /// its own queue, since no separate drain loop is spawned for it.
    inline_receiver: Option<Mutex<EffectQueueReceiver>>,
    /// `Deferred` mode only: holds the queue's receiver half until
    /// [`Self::start`] spawns the runner task that consumes it (observation
    /// 2, PR3 review). `new` must never call `tokio::spawn` itself, so it
    /// can be constructed outside a Tokio runtime context; a plain
    /// `std::sync::Mutex` is enough since `start` only ever takes the
    /// receiver out synchronously, once.
    deferred_receiver: Option<StdMutex<Option<EffectQueueReceiver>>>,
    /// Shared with the spawned `Deferred` runner's own drain loop
    /// ([`Self::start`]) — flips to `true` the moment
    /// [`EffectRuntimeHandle::shutdown_and_wait`] begins, telling that loop
    /// to stop admitting NEW work immediately. **F-02 (PR3 round 2 review):**
    /// this is a genuinely separate concern from cancelling acceptance work
    /// already in flight — admitting no more new effects is safe to do right
    /// away, but an acceptance already in progress should be allowed to
    /// finish naturally during the drain window and only be cancelled once
    /// the deadline is actually hit. See `deadline_rx`/`deadline_tx` below
    /// for that second signal. Present (and observable) in every runner
    /// mode, not just `Deferred` — `Inline` callers can still `start()`
    /// purely to obtain a shutdown-observing [`EffectRuntimeHandle`].
    shutdown_rx: watch::Receiver<bool>,
    shutdown_tx: watch::Sender<bool>,
    /// **F-02 (PR3 round 2 review, BLOCKER fix):** the *deadline instant*,
    /// not merely "has shutdown started" — `None` while no shutdown is in
    /// progress; set to `Some(Instant::now() + deadline)` exactly once, when
    /// [`EffectRuntimeHandle::shutdown_and_wait(deadline)`] begins. Every
    /// place that must cancel in-flight acceptance work
    /// ([`Self::accept_into_store`]'s retry loop and backoff sleep, and
    /// [`Self::send_to_queue`]'s `queue.send`) races against THIS signal via
    /// [`wait_for_deadline`], not `shutdown_rx` — so acceptance already in
    /// progress when shutdown merely *begins* is allowed to complete
    /// normally, and is cancelled only once the deadline instant is actually
    /// reached.
    deadline_rx: watch::Receiver<Option<tokio::time::Instant>>,
    deadline_tx: watch::Sender<Option<tokio::time::Instant>>,
    /// **F-02 (PR3 round 3 review, BLOCKER fix):** shared with the
    /// [`EffectRuntimeHandle`] [`Self::start`] returns — see
    /// [`LifecycleGate`]. `accept` checks this at entry, before minting
    /// anything, and holds an [`InFlightGuard`] for its whole duration.
    lifecycle: Arc<LifecycleGate>,
}

impl RuntimeEffectAcceptor {
    /// Builds a fresh acceptor for `config`. Constructs only — never spawns
    /// a task — so this can be called outside a Tokio runtime context
    /// (observation 2, PR3 review). Call [`Self::start`] to spawn the
    /// `Deferred` profile's drain loop and obtain the shutdown/lifecycle
    /// handle.
    pub fn new(
        state: Arc<dyn EffectStateStore>,
        dedup: Arc<dyn EffectDedupStore>,
        registry: Arc<ExecutorRegistry>,
        config: DeliveryConfig,
    ) -> Self {
        let (queue, receiver) = EffectQueue::bounded(config.queue_capacity);
        let runner = Arc::new(DeliveryRunner::new(state.clone(), dedup, registry, config.retry));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (deadline_tx, deadline_rx) = watch::channel(None);

        let (inline_receiver, deferred_receiver) = match config.runner_mode {
            RunnerMode::Deferred => (None, Some(StdMutex::new(Some(receiver)))),
            RunnerMode::Inline => (Some(Mutex::new(receiver)), None),
        };

        Self {
            state,
            accept_retry: config.retry,
            queue,
            runner,
            runner_mode: config.runner_mode,
            inline_receiver,
            deferred_receiver,
            shutdown_rx,
            shutdown_tx,
            deadline_rx,
            deadline_tx,
            lifecycle: LifecycleGate::new(),
        }
    }

    /// Spawns the `Deferred` profile's one `DeliveryRunner::run` drain loop
    /// (AD-8 single-consumer invariant) and returns the [`EffectRuntimeHandle`]
    /// that owns both the shutdown signal and the spawned task's
    /// `JoinHandle` (F-01, PR3 review). A no-op for `Inline` mode — no loop
    /// is ever spawned there — but still returns a handle sharing the same
    /// shutdown signal [`Self::accept_into_store`] observes, so callers
    /// don't need to special-case runner mode. Panics if called outside a
    /// Tokio runtime context (same as any `tokio::spawn`). Must be called at
    /// most once.
    pub fn start(&self) -> EffectRuntimeHandle {
        let runner_task = match self.runner_mode {
            RunnerMode::Deferred => {
                let receiver = self
                    .deferred_receiver
                    .as_ref()
                    .expect("Deferred runner_mode always constructs a deferred_receiver")
                    .lock()
                    .unwrap()
                    .take()
                    .expect("start() must not be called more than once");
                let runner = self.runner.clone();
                let shutdown_rx = self.shutdown_rx.clone();
                Some(tokio::spawn(runner.run(receiver, 1, shutdown_rx)))
            }
            RunnerMode::Inline => None,
        };

        EffectRuntimeHandle {
            shutdown: self.shutdown_tx.clone(),
            deadline: self.deadline_tx.clone(),
            runner_task,
            runner: self.runner.clone(),
            lifecycle: self.lifecycle.clone(),
        }
    }

    /// AD-9 classification: retries `EffectStateStore::accept` under the
    /// bounded policy only for `TemporarilyUnavailable`; every other variant
    /// is permanent and surfaces immediately without retry.
    ///
    /// F-02 (PR3 round 2 review, BLOCKER fix): both the `accept` call itself
    /// and the backoff sleep are raced against `deadline_rx` — the shared
    /// *deadline instant* signal, NOT `shutdown_rx` — so a retry already in
    /// flight when shutdown merely begins is allowed to run to completion,
    /// and is only cut short once the drain deadline actually elapses (AD-9's
    /// shutdown interaction).
    async fn accept_into_store(
        &self,
        effect: &AcceptedEffect,
        index: usize,
    ) -> Result<(), EffectAcceptanceError> {
        let mut attempt: u32 = 0;
        loop {
            // A fresh clone per iteration: `tokio::select!` holds every
            // branch's future for its whole body, so reusing one `Receiver`
            // across the outer select and the nested backoff-sleep select
            // below would try to borrow it mutably twice at once.
            let mut outer_deadline = self.deadline_rx.clone();
            tokio::select! {
                result = self.state.accept(effect.clone()) => {
                    match result {
                        Ok(()) => return Ok(()),
                        Err(EffectStoreError::TemporarilyUnavailable(msg)) => {
                            if !self.accept_retry.allows_retry(attempt) {
                                return Err(EffectAcceptanceError::RetriesExhausted {
                                    message: msg,
                                    failed_at_index: index,
                                    failed_idempotency_key: effect.description.idempotency_key.clone(),
                                });
                            }
                            let backoff = self.accept_retry.backoff(attempt + 1);
                            if !backoff.is_zero() {
                                let mut backoff_deadline = self.deadline_rx.clone();
                                tokio::select! {
                                    _ = tokio::time::sleep(backoff) => {}
                                    _ = wait_for_deadline(&mut backoff_deadline) => {
                                        return Err(EffectAcceptanceError::RetriesExhausted {
                                            message: DEADLINE_MSG.to_string(),
                                            failed_at_index: index,
                                            failed_idempotency_key:
                                                effect.description.idempotency_key.clone(),
                                        });
                                    }
                                }
                            }
                            attempt += 1;
                        }
                        Err(other) => return Err(EffectAcceptanceError::Permanent {
                            message: other.to_string(),
                            failed_at_index: index,
                            failed_idempotency_key: effect.description.idempotency_key.clone(),
                        }),
                    }
                }
                _ = wait_for_deadline(&mut outer_deadline) => {
                    return Err(EffectAcceptanceError::RetriesExhausted {
                        message: DEADLINE_MSG.to_string(),
                        failed_at_index: index,
                        failed_idempotency_key: effect.description.idempotency_key.clone(),
                    });
                }
            }
        }
    }

    /// F-02 (PR3 round 2 review, BLOCKER fix): `queue.send` used to race
    /// against nothing — a shutdown/deadline could elapse while this call
    /// was blocked awaiting queue capacity and it would simply keep waiting.
    /// Now shared by both `accept_one` branches (Deferred enqueues directly;
    /// Inline enqueues before immediately draining its own receiver), it
    /// races the send against `deadline_rx` the same way
    /// [`Self::accept_into_store`] does, returning `RetriesExhausted` if the
    /// deadline elapses first.
    async fn send_to_queue(
        &self,
        effect: AcceptedEffect,
        index: usize,
    ) -> Result<(), EffectAcceptanceError> {
        let idempotency_key = effect.description.idempotency_key.clone();
        let mut deadline_rx = self.deadline_rx.clone();
        tokio::select! {
            result = self.queue.send(effect) => {
                result.map_err(|SendError(effect)| EffectAcceptanceError::Permanent {
                    message: "effect queue closed".to_string(),
                    failed_at_index: index,
                    failed_idempotency_key: effect.description.idempotency_key.clone(),
                })
            }
            _ = wait_for_deadline(&mut deadline_rx) => {
                Err(EffectAcceptanceError::RetriesExhausted {
                    message: DEADLINE_MSG.to_string(),
                    failed_at_index: index,
                    failed_idempotency_key: idempotency_key,
                })
            }
        }
    }

    async fn accept_one(&self, effect: AcceptedEffect, index: usize) -> Result<(), EffectAcceptanceError> {
        self.accept_into_store(&effect, index).await?;
        log_accepted(&effect);

        match self.runner_mode {
            RunnerMode::Deferred => self.send_to_queue(effect, index).await,
            RunnerMode::Inline => {
                let mut receiver = self
                    .inline_receiver
                    .as_ref()
                    .expect("Inline runner_mode always constructs an inline receiver")
                    .lock()
                    .await;
                self.send_to_queue(effect, index).await?;
                let dispatched = receiver.recv().await.expect(
                    "the effect just sent must be immediately receivable — \
                     no other consumer exists in Inline mode",
                );
                self.runner.drain_one(dispatched).await;
                Ok(())
            }
        }
    }
}

/// F-02 (PR3 round 2 review): the message every acceptance-side cancellation
/// (`accept_into_store`'s retry loop/backoff sleep, `send_to_queue`) reports
/// once `deadline_rx`'s instant is actually reached.
const DEADLINE_MSG: &str = "effect acceptance interrupted by shutdown deadline";

#[async_trait]
impl EffectAcceptor for RuntimeEffectAcceptor {
    /// **F-02 (PR3 round 3 review, BLOCKER fix):** checks [`LifecycleGate`]
    /// at entry, before minting or touching the store at all — a call
    /// starting after shutdown has begun (`Draining`/`Closed`) is rejected
    /// immediately with the same shape as any other post-commit
    /// `EffectAcceptanceError` (F-03's precedent: a missing acceptor maps to
    /// `Permanent` too). The `InFlightGuard` is held for the whole batch —
    /// RAII, so it can never be skipped by the early `?` return inside the
    /// loop below.
    async fn accept(
        &self,
        tenant: &TenantId,
        effects: Vec<ExternalEffectDescription>,
    ) -> Result<(), EffectAcceptanceError> {
        let _in_flight_guard = self.lifecycle.enter().map_err(|()| {
            let failed_idempotency_key = effects
                .first()
                .map(|d| d.idempotency_key.clone())
                .unwrap_or_else(|| {
                    IdempotencyKey::new("shutdown-rejected-empty-batch")
                        .expect("non-empty literal is always a valid IdempotencyKey")
                });
            EffectAcceptanceError::Permanent {
                message: SHUTDOWN_REJECTED_MSG.to_string(),
                failed_at_index: 0,
                failed_idempotency_key,
            }
        })?;

        for (index, description) in effects.into_iter().enumerate() {
            let effect = AcceptedEffect {
                id: EffectId::new(),
                tenant: tenant.clone(),
                attempt: 0,
                description: Arc::new(description),
            };
            self.accept_one(effect, index).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::executor::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
    use crate::effects::store::InMemoryEffectStore;
    use ego_domain::IdempotencyKey;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;
    use tokio::sync::Notify;

    fn description(effect_type: &str, key: &str) -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new(key).unwrap(),
            effect_type: effect_type.to_string(),
            payload: vec![1, 2, 3],
            destination: "https://example.com".to_string(),
        }
    }

    struct AlwaysSucceeds {
        calls: AtomicUsize,
    }

    impl AlwaysSucceeds {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ExternalEffectExecutor for AlwaysSucceeds {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            AttemptOutcome::Success
        }
    }

    /// Blocks inside `execute` until `gate` is notified — used to prove
    /// `accept` awaits queue capacity rather than refusing.
    struct GatedExecutor {
        gate: Arc<Notify>,
    }

    #[async_trait]
    impl ExternalEffectExecutor for GatedExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            self.gate.notified().await;
            AttemptOutcome::Success
        }
    }

    /// Records every `AcceptedEffect` passed to `accept`, then delegates to a
    /// real `InMemoryEffectStore` — proves the id/tenant were attached
    /// *before* the store ever sees the effect.
    struct RecordingStore {
        inner: InMemoryEffectStore,
        accepted: StdMutex<Vec<AcceptedEffect>>,
    }

    impl RecordingStore {
        fn new() -> Self {
            Self {
                inner: InMemoryEffectStore::new(),
                accepted: StdMutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl EffectStateStore for RecordingStore {
        async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
            self.accepted.lock().unwrap().push(effect.clone());
            self.inner.accept(effect).await
        }
        async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.inner.mark_in_flight(id).await
        }
        async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.inner.mark_succeeded(id).await
        }
        async fn mark_retryable(
            &self,
            id: EffectId,
            attempt: u32,
            next_at: super::super::store::Timestamp,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_retryable(id, attempt, next_at).await
        }
        async fn mark_terminal(
            &self,
            id: EffectId,
            reason: super::super::store::TerminalReason,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_terminal(id, reason).await
        }
        async fn claim_due(
            &self,
            now: super::super::store::Timestamp,
            limit: usize,
        ) -> Result<Vec<super::super::store::StoredEffect>, EffectStoreError> {
            self.inner.claim_due(now, limit).await
        }
        async fn recover_in_flight(&self, now: super::super::store::Timestamp) -> Result<u64, EffectStoreError> {
            self.inner.recover_in_flight(now).await
        }
    }

    /// Returns a scripted sequence of `accept` results before falling
    /// through to a real, delegate `InMemoryEffectStore` — lets AD-9's
    /// retry-classification be tested without a real durable backend.
    struct ScriptedAcceptStore {
        inner: InMemoryEffectStore,
        script: StdMutex<Vec<Result<(), EffectStoreError>>>,
        accept_calls: AtomicU32,
    }

    impl ScriptedAcceptStore {
        fn new(script: Vec<Result<(), EffectStoreError>>) -> Self {
            Self {
                inner: InMemoryEffectStore::new(),
                script: StdMutex::new(script),
                accept_calls: AtomicU32::new(0),
            }
        }

        fn accept_calls(&self) -> u32 {
            self.accept_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EffectStateStore for ScriptedAcceptStore {
        async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
            self.accept_calls.fetch_add(1, Ordering::SeqCst);
            let next = self.script.lock().unwrap().pop();
            match next {
                Some(scripted) => scripted,
                None => self.inner.accept(effect).await,
            }
        }
        async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.inner.mark_in_flight(id).await
        }
        async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.inner.mark_succeeded(id).await
        }
        async fn mark_retryable(
            &self,
            id: EffectId,
            attempt: u32,
            next_at: super::super::store::Timestamp,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_retryable(id, attempt, next_at).await
        }
        async fn mark_terminal(
            &self,
            id: EffectId,
            reason: super::super::store::TerminalReason,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_terminal(id, reason).await
        }
        async fn claim_due(
            &self,
            now: super::super::store::Timestamp,
            limit: usize,
        ) -> Result<Vec<super::super::store::StoredEffect>, EffectStoreError> {
            self.inner.claim_due(now, limit).await
        }
        async fn recover_in_flight(&self, now: super::super::store::Timestamp) -> Result<u64, EffectStoreError> {
            self.inner.recover_in_flight(now).await
        }
    }

    fn tenant() -> TenantId {
        TenantId::new("tenant-a").unwrap()
    }

    #[tokio::test]
    async fn accept_mints_distinct_ids_and_attaches_tenant_before_store_interaction() {
        let recording = Arc::new(RecordingStore::new());
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(AlwaysSucceeds::new()))
            .unwrap();
        let acceptor = RuntimeEffectAcceptor::new(
            recording.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            DeliveryConfig::immediate(),
        );
        let tenant = tenant();

        acceptor
            .accept(&tenant, vec![description("invoice.created", "uow-1:0")])
            .await
            .unwrap();
        acceptor
            .accept(&tenant, vec![description("invoice.created", "uow-1:1")])
            .await
            .unwrap();

        let recorded = recording.accepted.lock().unwrap();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].tenant, tenant);
        assert_eq!(recorded[0].attempt, 0);
        assert_ne!(
            recorded[0].id, recorded[1].id,
            "each accepted effect must get its own freshly-minted id"
        );
    }

    #[tokio::test]
    async fn accept_retries_temporarily_unavailable_then_succeeds_within_bound() {
        let store = Arc::new(ScriptedAcceptStore::new(vec![
            Err(EffectStoreError::TemporarilyUnavailable("pool exhausted".into())),
            Err(EffectStoreError::TemporarilyUnavailable("pool exhausted".into())),
        ]));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(AlwaysSucceeds::new()))
            .unwrap();
        let fast_retry = RetryPolicy {
            max_attempts: 5,
            base_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
        };
        let config = DeliveryConfig {
            queue_capacity: 4,
            retry: fast_retry,
            runner_mode: RunnerMode::Inline,
        };
        let acceptor = RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            config,
        );

        acceptor
            .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
            .await
            .unwrap();

        assert_eq!(store.accept_calls(), 3, "2 failures + 1 eventual success");
    }

    #[tokio::test]
    async fn accept_returns_retries_exhausted_when_temporarily_unavailable_persists() {
        let store = Arc::new(ScriptedAcceptStore::new(vec![
            Err(EffectStoreError::TemporarilyUnavailable("pool exhausted".into())),
            Err(EffectStoreError::TemporarilyUnavailable("pool exhausted".into())),
            Err(EffectStoreError::TemporarilyUnavailable("pool exhausted".into())),
        ]));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let registry = ExecutorRegistry::new();
        let policy = RetryPolicy {
            max_attempts: 2,
            base_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(1),
        };
        let config = DeliveryConfig {
            queue_capacity: 4,
            retry: policy,
            runner_mode: RunnerMode::Inline,
        };
        let acceptor = RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            config,
        );

        let err = acceptor
            .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
            .await
            .unwrap_err();

        assert!(matches!(err, EffectAcceptanceError::RetriesExhausted { .. }));
        assert_eq!(store.accept_calls(), 3, "1 initial attempt + 2 retries, then give up");
    }

    #[tokio::test]
    async fn accept_surfaces_backend_error_immediately_without_retry() {
        let store = Arc::new(ScriptedAcceptStore::new(vec![Err(EffectStoreError::Backend(
            "corrupt record".into(),
        ))]));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let registry = ExecutorRegistry::new();
        let acceptor = RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            DeliveryConfig::immediate(),
        );

        let err = acceptor
            .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
            .await
            .unwrap_err();

        assert!(matches!(err, EffectAcceptanceError::Permanent { .. }));
        assert_eq!(store.accept_calls(), 1, "a permanent error must never be retried");
    }

    #[tokio::test]
    async fn accept_surfaces_conflict_from_accept_as_permanent_without_retry() {
        // The exact classification this test guards: `Conflict` FROM
        // `accept` specifically is permanent (an id-collision/invariant
        // conflict), never paired with the retry loop that only
        // `TemporarilyUnavailable` gets (AD-9's classification table).
        let store = Arc::new(ScriptedAcceptStore::new(vec![Err(EffectStoreError::Conflict(
            "effect id collision".into(),
        ))]));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let registry = ExecutorRegistry::new();
        let acceptor = RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            DeliveryConfig::immediate(),
        );

        let err = acceptor
            .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
            .await
            .unwrap_err();

        assert!(matches!(err, EffectAcceptanceError::Permanent { .. }));
        assert_eq!(store.accept_calls(), 1);
    }

    #[tokio::test]
    async fn accept_awaits_capacity_never_refusing_in_inline_mode() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let gate = Arc::new(Notify::new());
        registry
            .register(
                "invoice.created",
                Arc::new(GatedExecutor { gate: gate.clone() }),
            )
            .unwrap();
        let acceptor = RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            DeliveryConfig::immediate(),
        );
        let acceptor = Arc::new(acceptor);

        let first = {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                acceptor
                    .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !first.is_finished(),
            "first accept should still be blocked inside the gated executor"
        );

        let second = {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                acceptor
                    .accept(&tenant(), vec![description("invoice.created", "uow-2:0")])
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !second.is_finished(),
            "second accept must block awaiting capacity, never refuse or drop the effect"
        );

        gate.notify_waiters();
        first
            .await
            .expect("task joins")
            .expect("first accept eventually completes");
        gate.notify_waiters();
        second
            .await
            .expect("task joins")
            .expect("second accept eventually completes once capacity frees up");
    }

    #[tokio::test]
    async fn inline_mode_accept_drives_the_pipeline_synchronously_before_returning() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(AlwaysSucceeds::new());
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let acceptor = RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            DeliveryConfig::immediate(),
        );

        acceptor
            .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
            .await
            .unwrap();

        assert_eq!(
            executor.call_count(),
            1,
            "Inline mode must have already executed the effect by the time accept() returns"
        );
    }

    #[tokio::test]
    async fn deferred_mode_accept_enqueues_and_the_spawned_runner_eventually_processes_it() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(AlwaysSucceeds::new());
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let acceptor = RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            DeliveryConfig::default(),
        );
        let _handle = acceptor.start();

        acceptor
            .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            while executor.call_count() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("spawned deferred runner processes the enqueued effect within timeout");
    }

    // --- PR3 review: F-01 (lifecycle handle), F-02 (shutdown-aware
    // acceptance retry), observation 2 (new/start split) ---

    /// Observation 2 (PR3 review): `new` must only ever construct — never
    /// `tokio::spawn` — so it can be called outside a Tokio runtime context
    /// without panicking. This test intentionally runs on a plain OS thread
    /// (no `#[tokio::test]`) to prove exactly that: if `new` still spawned
    /// internally, this test would panic with "no reactor running".
    #[test]
    fn new_does_not_require_a_tokio_runtime_context() {
        let store = Arc::new(InMemoryEffectStore::new());
        let registry = ExecutorRegistry::new();

        let _acceptor = RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            DeliveryConfig::default(),
        );
    }

    /// Executor that signals `started` the moment it is invoked, then does
    /// real, observable bounded work before marking `finished` — lets a test
    /// prove `shutdown_and_wait` awaits actual completion, not just that the
    /// shutdown signal was sent.
    struct SlowExecutor {
        started: Arc<Notify>,
        work_duration: Duration,
        finished: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl ExternalEffectExecutor for SlowExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            self.started.notify_one();
            tokio::time::sleep(self.work_duration).await;
            self.finished.store(true, Ordering::SeqCst);
            AttemptOutcome::Success
        }
    }

    /// F-01 (PR3 review, BLOCKER): `shutdown_and_wait` must actually await
    /// the spawned `Deferred` runner task's completion, not merely send the
    /// shutdown signal and return. Proven by racing a real, observable,
    /// bounded unit of executor work against the returned handle: if
    /// `shutdown_and_wait` returned as soon as the signal was sent (the bug
    /// this fixes), `finished` would still be `false` when it returns.
    #[tokio::test]
    async fn shutdown_and_wait_awaits_the_runner_task_to_actually_finish_its_work() {
        use std::sync::atomic::AtomicBool;

        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let started = Arc::new(Notify::new());
        let finished = Arc::new(AtomicBool::new(false));
        registry
            .register(
                "invoice.created",
                Arc::new(SlowExecutor {
                    started: started.clone(),
                    work_duration: Duration::from_millis(150),
                    finished: finished.clone(),
                }),
            )
            .unwrap();
        let acceptor = RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            DeliveryConfig::default(),
        );
        let handle = acceptor.start();

        acceptor
            .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("the runner must have already started dispatching the effect");

        handle
            .shutdown_and_wait(Duration::from_secs(5))
            .await
            .expect("the runner task must finish cleanly within the deadline");

        assert!(
            finished.load(Ordering::SeqCst),
            "shutdown_and_wait must not return until the runner's actual work finished, \
             not merely after the shutdown signal was sent"
        );
    }

    /// F-02 (PR3 round 2 review, BLOCKER): shutdown merely *starting* must
    /// NOT cancel an acceptance retry already in flight — only the actual
    /// deadline elapsing may. Proven by signalling shutdown with a deadline
    /// (5s) far beyond the in-progress backoff sleep (100ms): the retry must
    /// complete normally and succeed, exactly as if shutdown had never been
    /// signalled.
    #[tokio::test]
    async fn acceptance_in_progress_completes_normally_when_shutdown_starts_but_deadline_has_not_elapsed(
    ) {
        let modest_backoff_retry = RetryPolicy {
            max_attempts: 5,
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(100),
        };
        let store = Arc::new(ScriptedAcceptStore::new(vec![Err(
            EffectStoreError::TemporarilyUnavailable("pool exhausted".into()),
        )]));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(AlwaysSucceeds::new()))
            .unwrap();
        let config = DeliveryConfig {
            queue_capacity: 4,
            retry: modest_backoff_retry,
            runner_mode: RunnerMode::Inline,
        };
        let acceptor = Arc::new(RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            config,
        ));
        let handle = acceptor.start();

        let accept_task = {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                acceptor
                    .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
                    .await
            })
        };

        // Let the retry loop fail its first attempt and enter its 100ms
        // backoff sleep, then signal shutdown with a deadline (5s) that is
        // nowhere close to elapsing yet.
        tokio::time::sleep(Duration::from_millis(20)).await;
        handle
            .shutdown_and_wait(Duration::from_secs(5))
            .await
            .expect("Inline mode spawns no runner task, so this resolves immediately");

        let result = tokio::time::timeout(Duration::from_secs(1), accept_task)
            .await
            .expect("accept must not be cancelled just because shutdown started")
            .expect("task joins");

        assert!(
            result.is_ok(),
            "acceptance in progress must complete normally since the 5s deadline hasn't elapsed"
        );
        assert_eq!(
            store.accept_calls(),
            2,
            "1 failure + 1 successful retry, uninterrupted by shutdown merely starting"
        );
    }

    /// F-02 (PR3 round 2 review, BLOCKER): once the deadline instant actually
    /// elapses, an acceptance retry still mid-backoff-sleep at that point
    /// MUST be cancelled with an explicit `EffectAcceptanceError`, rather
    /// than sleeping out the full (here, 500ms) backoff regardless.
    #[tokio::test]
    async fn acceptance_in_progress_is_cancelled_once_the_deadline_instant_actually_elapses() {
        // CORE-027 flaky-triage fix: `RetryPolicy::backoff` applies *full
        // jitter* — a uniform random duration in `[0, capped]`, never a fixed
        // sleep. A 30s backoff cap against a 1s deadline margin still leaves
        // ~3.5% of samples landing under 1.05s (confirmed empirically: this
        // test failed at iteration 28/200 of the flaky-triage tight loop with
        // `store.accept_calls() == 2`, i.e. the jittered backoff finished
        // before the deadline elapsed and a genuine second attempt raced in
        // ahead of cancellation — not a scheduler race in the acceptor
        // itself). Widening the cap to a year makes that collision
        // probability (~1.05s / 31_536_000s ≈ 3e-8 per run) negligible without
        // slowing the test down, since the deadline always cuts the sleep
        // short well before it could ever run to completion.
        let long_backoff_retry = RetryPolicy {
            max_attempts: 10,
            base_backoff: Duration::from_secs(60 * 60 * 24 * 365),
            max_backoff: Duration::from_secs(60 * 60 * 24 * 365),
        };
        let store = Arc::new(ScriptedAcceptStore::new(vec![
            Err(EffectStoreError::TemporarilyUnavailable("pool exhausted".into())),
            Err(EffectStoreError::TemporarilyUnavailable("pool exhausted".into())),
            Err(EffectStoreError::TemporarilyUnavailable("pool exhausted".into())),
        ]));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let registry = ExecutorRegistry::new();
        let config = DeliveryConfig {
            queue_capacity: 4,
            retry: long_backoff_retry,
            runner_mode: RunnerMode::Inline,
        };
        let acceptor = Arc::new(RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            config,
        ));
        let handle = acceptor.start();

        let accept_task = {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                acceptor
                    .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
                    .await
            })
        };

        // Let the retry loop fail its first attempt and enter its (would-be
        // 30s) backoff sleep, then signal shutdown with a deadline (1s) short
        // enough to elapse well before that backoff would ever finish.
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle
            .shutdown_and_wait(Duration::from_secs(1))
            .await
            .expect("Inline mode spawns no runner task, so this resolves immediately");

        let result = tokio::time::timeout(Duration::from_secs(2), accept_task)
            .await
            .expect(
                "accept must resolve once the deadline instant elapses, not keep sleeping \
                 the full 30s backoff",
            )
            .expect("task joins");

        assert!(matches!(
            result.unwrap_err(),
            EffectAcceptanceError::RetriesExhausted { .. }
        ));
        assert_eq!(
            store.accept_calls(),
            1,
            "only the initial failing attempt — the backoff sleep was cut short by the \
             deadline elapsing, no retry attempt ever made"
        );
    }

    /// F-01 (PR3 round 2 review, BLOCKER): `shutdown_and_wait`'s timeout
    /// branch must not merely drop the `JoinHandle` — dropping a
    /// `tokio::task::JoinHandle` only detaches from the underlying task, it
    /// does NOT cancel/abort it, which used to leave the task running in the
    /// background forever even after `shutdown_and_wait` returned `Timeout`.
    /// Proven directly against `EffectRuntimeHandle` (constructed here since
    /// this `tests` module is a child of `acceptor`'s own module and can see
    /// its private fields), bypassing the full acceptor/runner machinery: a
    /// task that increments a shared counter forever is wrapped in a handle
    /// and shut down with a deadline far shorter than the task could ever
    /// finish on its own. Once `shutdown_and_wait` returns `Timeout`, the
    /// counter must stop incrementing — proving the task was actually
    /// aborted, not merely detached.
    #[tokio::test]
    async fn shutdown_and_wait_aborts_the_runner_task_on_timeout_instead_of_merely_detaching() {
        let counter = Arc::new(AtomicUsize::new(0));
        let runner_task = {
            let counter = counter.clone();
            tokio::spawn(async move {
                loop {
                    counter.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
        };
        let (shutdown, _shutdown_rx) = watch::channel(false);
        let (deadline, _deadline_rx) = watch::channel(None);
        let store = Arc::new(InMemoryEffectStore::new());
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store as Arc<dyn EffectDedupStore>,
            Arc::new(ExecutorRegistry::new()),
            RetryPolicy::default(),
        ));
        let handle = EffectRuntimeHandle {
            shutdown,
            deadline,
            runner_task: Some(runner_task),
            runner,
            lifecycle: LifecycleGate::new(),
        };

        let err = handle
            .shutdown_and_wait(Duration::from_millis(50))
            .await
            .expect_err("the runner task never finishes on its own, so this must time out");
        assert!(matches!(err, EffectRuntimeShutdownError::Timeout));

        let count_at_timeout = counter.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            count_at_timeout,
            counter.load(Ordering::SeqCst),
            "the runner task must have been aborted, not merely detached — the counter must \
             stop incrementing once shutdown_and_wait returns Timeout"
        );
    }

    // --- PR3 round 3 review: F-01 (unified `DeliveryRunner::shutdown_and_drain`,
    // proven directly in runner.rs) + F-02 (lifecycle admission gating) ---

    /// **F-02 (PR3 round 3 review, BLOCKER):** a NEW `accept()` call starting
    /// after shutdown has already begun (`LifecycleGate` already `Draining`)
    /// must be rejected immediately — no minting, no store interaction at
    /// all. Drives the shared `lifecycle` field directly (this `tests`
    /// module is a child of `acceptor`'s own module and can see it) rather
    /// than spinning up the whole `EffectRuntimeHandle::shutdown_and_wait`
    /// machinery, to isolate exactly the behavior under test: admission
    /// gating at `accept`'s entry.
    #[tokio::test]
    async fn accept_started_after_draining_is_rejected_immediately_without_touching_the_store() {
        let store = Arc::new(RecordingStore::new());
        let dedup = Arc::new(InMemoryEffectStore::new());
        let registry = ExecutorRegistry::new();
        let acceptor = RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            DeliveryConfig::immediate(),
        );

        acceptor
            .lifecycle
            .begin_draining(tokio::time::Instant::now() + Duration::from_secs(5));

        let err = acceptor
            .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
            .await
            .unwrap_err();

        assert!(matches!(err, EffectAcceptanceError::Permanent { .. }));
        assert_eq!(
            store.accepted.lock().unwrap().len(),
            0,
            "a call starting after shutdown began must never touch the store at all"
        );
    }

    /// Delegates every call to a real [`InMemoryEffectStore`] except
    /// `accept`, which signals `entered` then blocks on `gate` before
    /// delegating — lets a test deterministically observe "an acceptance is
    /// now in flight, inside the store call" and control exactly when it
    /// finishes.
    struct GatedAcceptStore {
        inner: InMemoryEffectStore,
        gate: Arc<Notify>,
        entered: Arc<Notify>,
    }

    #[async_trait]
    impl EffectStateStore for GatedAcceptStore {
        async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
            self.entered.notify_one();
            self.gate.notified().await;
            self.inner.accept(effect).await
        }
        async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.inner.mark_in_flight(id).await
        }
        async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.inner.mark_succeeded(id).await
        }
        async fn mark_retryable(
            &self,
            id: EffectId,
            attempt: u32,
            next_at: super::super::store::Timestamp,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_retryable(id, attempt, next_at).await
        }
        async fn mark_terminal(
            &self,
            id: EffectId,
            reason: super::super::store::TerminalReason,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_terminal(id, reason).await
        }
        async fn claim_due(
            &self,
            now: super::super::store::Timestamp,
            limit: usize,
        ) -> Result<Vec<super::super::store::StoredEffect>, EffectStoreError> {
            self.inner.claim_due(now, limit).await
        }
        async fn recover_in_flight(&self, now: super::super::store::Timestamp) -> Result<u64, EffectStoreError> {
            self.inner.recover_in_flight(now).await
        }
    }

    /// **F-02 (PR3 round 3 review, BLOCKER):** `shutdown_and_wait` must
    /// genuinely not return until an `accept()` call that was ALREADY in
    /// flight when shutdown began has finished — not merely send the
    /// shutdown signal and move on. Shared implementation for both `mode`s
    /// (the bug was previously provable specifically in `Inline` mode, where
    /// there is no runner task at all to await, so `shutdown_and_wait` had
    /// nothing else to even pretend to wait for).
    async fn shutdown_and_wait_awaits_an_already_in_flight_accept_call(mode: RunnerMode) {
        let gate = Arc::new(Notify::new());
        let entered = Arc::new(Notify::new());
        let store = Arc::new(GatedAcceptStore {
            inner: InMemoryEffectStore::new(),
            gate: gate.clone(),
            entered: entered.clone(),
        });
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(AlwaysSucceeds::new()))
            .unwrap();
        let config = DeliveryConfig {
            queue_capacity: 4,
            retry: RetryPolicy::default(),
            runner_mode: mode,
        };
        let acceptor = Arc::new(RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            config,
        ));
        let handle = acceptor.start();

        let accept_task = {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                acceptor
                    .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .expect("the accept call must have already entered the gated store.accept");

        let shutdown_task = tokio::spawn(handle.shutdown_and_wait(Duration::from_secs(5)));

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !shutdown_task.is_finished(),
            "shutdown_and_wait must not return while the already-in-flight accept call is \
             still blocked inside the store"
        );

        gate.notify_waiters();

        // Whether the unblocked `accept()` call ultimately succeeds or fails
        // (in `Deferred` mode it MAY legitimately race the spawned runner
        // task exiting on the same shutdown signal and observe "queue
        // closed" — a pre-existing, unrelated behavior) is not what this
        // test proves. What matters is that the call actually finished
        // running, and that `shutdown_and_wait` did not return before it did.
        let _ = accept_task.await.expect("task joins");

        let _ = tokio::time::timeout(Duration::from_secs(2), shutdown_task)
            .await
            .expect("shutdown_and_wait must finish soon after the in-flight accept finishes")
            .expect("task joins");
    }

    #[tokio::test]
    async fn shutdown_and_wait_awaits_an_already_in_flight_accept_call_in_inline_mode() {
        shutdown_and_wait_awaits_an_already_in_flight_accept_call(RunnerMode::Inline).await;
    }

    #[tokio::test]
    async fn shutdown_and_wait_awaits_an_already_in_flight_accept_call_in_deferred_mode() {
        shutdown_and_wait_awaits_an_already_in_flight_accept_call(RunnerMode::Deferred).await;
    }

    // --- PR3 round 4 review: F-01 (drain acceptances before stopping the
    // runner consumer), F-02 (single drain authority, proven in runner.rs),
    // F-03 (watch-based in-flight tracking, no lost wakeup) ---

    /// **F-01 (PR3 round 4 review, BLOCKER):** `shutdown_and_wait` used to
    /// signal `shutdown.send(true)` — telling the spawned `Deferred` runner
    /// to stop consuming its receive loop — BEFORE waiting for acceptances
    /// already in flight to finish enqueueing. An `accept()` call gated mid
    /// `state.accept()` (via [`GatedAcceptStore`]) that is released only
    /// AFTER `shutdown_and_wait` has begun still reaches `queue.send` — the
    /// runner must still be consuming at that moment, or the effect is
    /// durably accepted and successfully enqueued yet never actually
    /// dispatched. Proven end-to-end: once released, the effect must
    /// actually reach the registered executor.
    #[tokio::test]
    async fn late_enqueue_after_shutdown_begins_is_still_consumed_by_the_deferred_runner() {
        let gate = Arc::new(Notify::new());
        let entered = Arc::new(Notify::new());
        let store = Arc::new(GatedAcceptStore {
            inner: InMemoryEffectStore::new(),
            gate: gate.clone(),
            entered: entered.clone(),
        });
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(AlwaysSucceeds::new());
        registry.register("invoice.created", executor.clone()).unwrap();
        let config = DeliveryConfig {
            queue_capacity: 4,
            retry: RetryPolicy::default(),
            runner_mode: RunnerMode::Deferred,
        };
        let acceptor = Arc::new(RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            config,
        ));
        let handle = acceptor.start();

        let accept_task = {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                acceptor
                    .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .expect("the accept call must have already entered the gated store.accept");

        let shutdown_task = tokio::spawn(handle.shutdown_and_wait(Duration::from_secs(5)));

        // Give shutdown_and_wait a generous head start: under the pre-fix
        // ordering it sends `shutdown=true` (and the runner stops consuming)
        // essentially immediately; under the fix, it is still blocked inside
        // `wait_until_drained` this whole time, since the in-flight accept
        // hasn't finished yet.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !shutdown_task.is_finished(),
            "shutdown_and_wait must still be waiting on the in-flight accept call"
        );

        // Release the gated store call now — well after shutdown began.
        // The accept call proceeds to `send_to_queue` next.
        gate.notify_waiters();

        accept_task
            .await
            .expect("task joins")
            .expect("the late-releasing accept call must still succeed");

        let _ = tokio::time::timeout(Duration::from_secs(1), shutdown_task)
            .await
            .expect("shutdown_and_wait must finish soon after the in-flight accept finishes")
            .expect("task joins");

        tokio::time::timeout(Duration::from_secs(1), async {
            while executor.call_count() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect(
            "the effect enqueued AFTER shutdown began must still be consumed and dispatched by \
             the runner — not left stranded in the queue because the runner had already stopped \
             consuming before the late enqueue happened",
        );
    }

    /// **F-03 (PR3 round 4 review, BLOCKER):** `LifecycleGate::wait_until_drained`
    /// used to pair an `AtomicU64` in-flight counter with `Notify` —
    /// `Notify::notify_waiters()` only wakes waiters already registered at
    /// the exact moment it is called, unlike a stored-permit/state-carrying
    /// primitive. A guard dropping to zero concurrently with
    /// `wait_until_drained`'s own read-then-await sequence could have its
    /// wakeup lost entirely, burning the full deadline for nothing.
    ///
    /// Best-effort integration-level regression check against the REAL,
    /// fixed `LifecycleGate`: many trials racing a concurrent guard-drop
    /// directly against `wait_until_drained` with no artificial delay on
    /// either side. Total elapsed time must stay far below what even a
    /// couple of full per-trial deadlines would cost. Note: on this
    /// hardware/scheduler, the true pre-fix race window (a handful of CPU
    /// instructions between the atomic read and `.notified()` being
    /// polled/registered, with no intervening yield point) proved
    /// empirically unreproducible even across 25,000+ trials using several
    /// synchronization strategies (plain `tokio::spawn`, pre-spun
    /// busy-waiting `std::thread`, and single-`yield_now` alignment on a
    /// current-thread runtime) — a cooperatively-scheduled task's
    /// synchronous check-then-register prefix always completes before any
    /// other task/thread gets a chance to run. This test therefore mainly
    /// guards against a regression that reintroduces contention/deadlocks
    /// under concurrent load; the deterministic RED/GREEN proof for F-03
    /// itself is `lost_wakeup_pattern_is_reproduced_with_a_widened_race_window`
    /// below, which validates the exact same watch-vs-Notify contract
    /// difference using an artificially widened window (confirmed
    /// empirically: 50/50 lost-wakeup failures with `Notify`, 0/50 with
    /// `watch`, per the same construction).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn wait_until_drained_does_not_lose_the_last_guards_wakeup_under_concurrent_drop() {
        const TRIALS: usize = 300;
        const PER_TRIAL_DEADLINE: Duration = Duration::from_millis(50);

        let started = std::time::Instant::now();
        for _ in 0..TRIALS {
            let gate = LifecycleGate::new();
            let guard = gate.enter().expect("gate starts Running");
            gate.begin_draining(tokio::time::Instant::now() + PER_TRIAL_DEADLINE);

            tokio::spawn(async move {
                drop(guard);
            });

            gate.wait_until_drained().await;
        }
        let elapsed = started.elapsed();

        assert!(
            elapsed < PER_TRIAL_DEADLINE * 3,
            "across {TRIALS} trials racing a concurrent guard-drop against \
             wait_until_drained's own read-then-await, total elapsed time ({elapsed:?}) must \
             stay far below even a couple of per-trial deadlines ({PER_TRIAL_DEADLINE:?} each)"
        );
    }

    /// **F-03 (PR3 round 4 review, BLOCKER) — deterministic RED/GREEN proof.**
    /// Reproduces the exact lost-wakeup contract gap `LifecycleGate` used to
    /// be exposed to: a "decrement in-flight count, notify if it reached
    /// zero" writer racing a "check count, then await a notification" reader,
    /// with the writer's decrement+notify deliberately landing inside the
    /// reader's own artificially widened check-then-await window (a
    /// `tokio::time::sleep` inserted purely to make an otherwise
    /// nanosecond-wide, empirically unreproducible race land reliably in a
    /// test — see the note on the test above for the reproduction attempts
    /// that ruled out a realistic timing-only reproduction). Run against
    /// both shapes side by side: the `Notify`-based shape (`LifecycleGate`'s
    /// OLD implementation) must reliably lose the wakeup and burn the full
    /// deadline; the `watch`-based shape (`LifecycleGate`'s fixed
    /// implementation, mirrored here since `watch::Receiver::borrow()`/
    /// `changed()` always reflect the latest sent value regardless of when
    /// they are polled relative to the send) must never lose it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn lost_wakeup_pattern_is_reproduced_with_a_widened_race_window() {
        use std::sync::atomic::AtomicU64;

        const TRIALS: usize = 30;
        const PER_TRIAL_DEADLINE: Duration = Duration::from_millis(200);
        // The artificial widening: enough for the writer's own 2ms delay
        // below to reliably land inside it, but far short of the 200ms
        // per-trial deadline.
        const WIDENED_WINDOW: Duration = Duration::from_millis(5);

        async fn wait_notify_based(
            in_flight: Arc<AtomicU64>,
            drained: Arc<Notify>,
            deadline: tokio::time::Instant,
        ) -> bool {
            loop {
                if in_flight.load(Ordering::SeqCst) == 0 {
                    return true;
                }
                tokio::time::sleep(WIDENED_WINDOW).await;
                tokio::select! {
                    _ = drained.notified() => {}
                    _ = tokio::time::sleep_until(deadline) => return false,
                }
                if in_flight.load(Ordering::SeqCst) == 0 {
                    return true;
                }
            }
        }

        async fn wait_watch_based(
            mut rx: watch::Receiver<u64>,
            deadline: tokio::time::Instant,
        ) -> bool {
            loop {
                if *rx.borrow() == 0 {
                    return true;
                }
                tokio::time::sleep(WIDENED_WINDOW).await;
                tokio::select! {
                    result = rx.changed() => {
                        if result.is_err() {
                            return true;
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => return false,
                }
            }
        }

        let mut notify_based_lost_wakeups = 0usize;
        for _ in 0..TRIALS {
            let in_flight = Arc::new(AtomicU64::new(1));
            let drained = Arc::new(Notify::new());
            let (in_flight2, drained2) = (in_flight.clone(), drained.clone());
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(2)).await;
                if in_flight2.fetch_sub(1, Ordering::SeqCst) == 1 {
                    drained2.notify_waiters();
                }
            });
            let deadline = tokio::time::Instant::now() + PER_TRIAL_DEADLINE;
            if !wait_notify_based(in_flight, drained, deadline).await {
                notify_based_lost_wakeups += 1;
            }
        }
        assert!(
            notify_based_lost_wakeups > 0,
            "sanity check: the Notify-based shape must reproduce the lost-wakeup bug at least \
             once across {TRIALS} widened-window trials, or this test cannot prove anything"
        );

        let mut watch_based_lost_wakeups = 0usize;
        for _ in 0..TRIALS {
            let (tx, rx) = watch::channel(1u64);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(2)).await;
                tx.send_modify(|n| *n -= 1);
            });
            let deadline = tokio::time::Instant::now() + PER_TRIAL_DEADLINE;
            if !wait_watch_based(rx, deadline).await {
                watch_based_lost_wakeups += 1;
            }
        }
        assert_eq!(
            watch_based_lost_wakeups, 0,
            "the watch-based shape (LifecycleGate's fixed implementation) must never lose the \
             wakeup under the exact same widened race window that reliably breaks the \
             Notify-based shape above"
        );
    }

    // --- PR3 round 5 review: F-01 (leader/follower drain coordination
    // removed; external drain via `shutdown_and_drain` is the sole cleanup
    // authority) ---

    /// Executor that signals `started`, then runs forever, incrementing a
    /// shared counter — lets a test observe directly whether the underlying
    /// task is still actually running (not merely that some outer task
    /// returned).
    struct CountingHangingExecutor {
        started: Arc<Notify>,
        counter: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ExternalEffectExecutor for CountingHangingExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            self.started.notify_one();
            loop {
                self.counter.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }

    /// **F-01 (PR3 round 5 review, BLOCKER fix):** the round 4 leader/follower
    /// drain coordination was itself unsafe in the real shutdown path.
    /// `run_inner` runs INSIDE the very task `EffectRuntimeHandle` holds as
    /// `runner_task` — so if `run_inner` observes shutdown and becomes the
    /// drain LEADER (using its own longer internal deadline) before
    /// `EffectRuntimeHandle::shutdown_and_wait`'s external call reaches
    /// `coordinated_drain`, the external call becomes a FOLLOWER bounded by
    /// its own (shorter) deadline. Once that shorter deadline elapses without
    /// `drain_done` firing (because the leader's own longer drain is still
    /// running), `shutdown_and_wait` proceeds to await `runner_task` with an
    /// already-elapsed deadline and aborts it — but that outer task IS the
    /// leader, still mid `drain_tasks_locked`, having not yet aborted the
    /// hung executor attempt it was in the middle of cleaning up. The
    /// leader's own cleanup is abandoned mid-flight, and the hung executor
    /// task (owned by `DeliveryRunner`'s own fields, not the aborted
    /// `runner_task`) is left running forever — the exact "background work
    /// survives shutdown" class of bug the whole `EffectRuntimeHandle`
    /// redesign was meant to eliminate.
    ///
    /// Reproduced end-to-end through the REAL `EffectRuntimeHandle` and the
    /// REAL `shutdown_and_wait`/`shutdown_and_drain`/`run()` (real
    /// `run_inner`, real production `SHUTDOWN_DRAIN_DEADLINE` constant) — not
    /// a synthetic harness calling bare drain functions directly (the gap the
    /// previous round's own test left: it never went through
    /// `EffectRuntimeHandle::shutdown_and_wait`'s own `runner_task.abort()`
    /// path at all). The only orchestration liberty taken is pre-triggering
    /// the shared `shutdown` signal directly and giving the already-spawned
    /// `run()` task a brief, deterministic head start to reach its own drain
    /// step first — the real-world equivalent of `run_inner` simply winning
    /// that race, which is exactly the scenario the reviewer described.
    ///
    /// Fixed (design.md "PR3 round 5"): `run_inner` no longer performs any
    /// drain of its own at all; `DeliveryRunner::shutdown_and_drain` (called
    /// solely by `EffectRuntimeHandle::shutdown_and_wait`) is the ONE
    /// cleanup authority, so there is no leader/follower ambiguity left for
    /// this race to exploit — a hung executor attempt is now provably
    /// aborted before `shutdown_and_wait` returns, whether it returns `Ok` or
    /// a timeout-shaped error.
    #[tokio::test]
    async fn shutdown_and_wait_stops_a_hung_executor_task_even_when_run_inner_would_have_raced_it_for_drain_leadership(
    ) {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let started = Arc::new(Notify::new());
        let counter = Arc::new(AtomicUsize::new(0));
        registry
            .register(
                "invoice.created",
                Arc::new(CountingHangingExecutor {
                    started: started.clone(),
                    counter: counter.clone(),
                }),
            )
            .unwrap();
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            RetryPolicy::default(),
        ));
        let (queue, receiver) = EffectQueue::bounded(4);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (deadline_tx, _deadline_rx) = watch::channel(None);

        let id = EffectId::new();
        let effect = AcceptedEffect {
            id,
            tenant: tenant(),
            attempt: 0,
            description: Arc::new(description("invoice.created", "uow-1:0")),
        };
        store.accept(effect.clone()).await.unwrap();
        queue.send(effect).await.unwrap();

        // Real `run()` — real `run_inner` internally, real production
        // constants — exactly the task `EffectRuntimeHandle` holds as
        // `runner_task` in production.
        let runner_task = tokio::spawn(runner.clone().run(receiver, 2, shutdown_rx));

        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("the hung executor starts running within timeout");

        // Give `run_inner` a deterministic head start to observe shutdown and
        // reach its own drain step first, BEFORE the external
        // `EffectRuntimeHandle` below is even constructed/invoked.
        shutdown_tx.send(true).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;

        let handle = EffectRuntimeHandle {
            shutdown: shutdown_tx,
            deadline: deadline_tx,
            runner_task: Some(runner_task),
            runner: runner.clone(),
            lifecycle: LifecycleGate::new(),
        };

        // A short caller deadline — short enough that a leader running its
        // own much longer internal drain (the round 4 bug's shape) would
        // never finish in time, forcing the old follower logic to abort the
        // leader's own task mid-cleanup.
        let _ = handle.shutdown_and_wait(Duration::from_millis(150)).await;

        let count_after_shutdown = counter.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            count_after_shutdown,
            counter.load(Ordering::SeqCst),
            "the hung executor task must be provably stopped once shutdown_and_wait returns, \
             regardless of whether it returned Ok or a timeout-shaped error"
        );
    }

    // --- PR3 round 6 review: F-01 (honest final `Result` — `Ok(())` must
    // require BOTH the runner task finishing cleanly AND
    // `DeliveryRunner::shutdown_and_drain` reporting a natural, on-time
    // drain, not just the former) ---

    /// **F-01 (PR3 round 6 review, BLOCKER):** `Inline` mode never spawns a
    /// `runner_task` at all, so `shutdown_and_wait`'s result used to come
    /// unconditionally from `Ok(())` regardless of what happened during the
    /// drain step. `Inline` mode's `drain_one` also runs synchronously on the
    /// caller's own task, never through `spawn_tracked` — so `tasks` stays
    /// empty for the whole run, and the pre-fix `drain_tasks_locked` had no
    /// way to know a hung executor (tracked only in `executor_aborts`) even
    /// existed, let alone that it had to force-abort it. Proven by driving a
    /// real inline acceptance into a hanging executor and shutting down with
    /// a short deadline: `shutdown_and_wait` must report `Err(Timeout)`, not
    /// a false, clean `Ok(())`.
    #[tokio::test]
    async fn shutdown_and_wait_returns_timeout_when_an_inline_executor_hangs_past_the_deadline() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let started = Arc::new(Notify::new());
        let counter = Arc::new(AtomicUsize::new(0));
        registry
            .register(
                "invoice.created",
                Arc::new(CountingHangingExecutor {
                    started: started.clone(),
                    counter: counter.clone(),
                }),
            )
            .unwrap();
        let config = DeliveryConfig {
            queue_capacity: 4,
            retry: RetryPolicy::default(),
            runner_mode: RunnerMode::Inline,
        };
        let acceptor = Arc::new(RuntimeEffectAcceptor::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            config,
        ));
        let handle = acceptor.start();

        let _accept_task = {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                acceptor
                    .accept(&tenant(), vec![description("invoice.created", "uow-1:0")])
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("the hung inline executor starts running within timeout");

        let err = handle
            .shutdown_and_wait(Duration::from_millis(100))
            .await
            .expect_err(
                "an inline-executing acceptance blocked on a hung executor must not let \
                 shutdown_and_wait report a clean Ok(())",
            );
        assert!(matches!(err, EffectRuntimeShutdownError::Timeout));
    }

    /// **F-01 (PR3 round 6 review, BLOCKER):** in `Deferred` mode, the outer
    /// `runner_task` (`run_inner`) can finish `Ok` quickly once it observes
    /// shutdown — it no longer drains anything of its own (round 5) — while a
    /// child per-effect dispatch task it already spawned is still hung inside
    /// its own executor call and has to be force-aborted during the
    /// subsequent `DeliveryRunner::shutdown_and_drain` step. The pre-fix
    /// result discarded that step's outcome entirely, reporting `Ok(())`
    /// purely because the outer task returned cleanly. Proven directly
    /// against `EffectRuntimeHandle`/`DeliveryRunner::run` (bypassing the
    /// acceptor) with a short caller deadline.
    #[tokio::test]
    async fn shutdown_and_wait_returns_timeout_when_the_runner_task_exits_cleanly_but_a_child_executor_hangs(
    ) {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let started = Arc::new(Notify::new());
        let counter = Arc::new(AtomicUsize::new(0));
        registry
            .register(
                "invoice.created",
                Arc::new(CountingHangingExecutor {
                    started: started.clone(),
                    counter: counter.clone(),
                }),
            )
            .unwrap();
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            RetryPolicy::default(),
        ));
        let (queue, receiver) = EffectQueue::bounded(4);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (deadline_tx, _deadline_rx) = watch::channel(None);

        let id = EffectId::new();
        let effect = AcceptedEffect {
            id,
            tenant: tenant(),
            attempt: 0,
            description: Arc::new(description("invoice.created", "uow-1:0")),
        };
        store.accept(effect.clone()).await.unwrap();
        queue.send(effect).await.unwrap();

        // Real `run()`/`run_inner` — once it observes `shutdown = true` it
        // stops consuming and returns quickly, WITHOUT draining the child
        // dispatch task it already spawned for the hung effect above.
        let runner_task = tokio::spawn(runner.clone().run(receiver, 2, shutdown_rx));

        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("the hung child executor starts running within timeout");

        let handle = EffectRuntimeHandle {
            shutdown: shutdown_tx,
            deadline: deadline_tx,
            runner_task: Some(runner_task),
            runner: runner.clone(),
            lifecycle: LifecycleGate::new(),
        };

        let err = handle
            .shutdown_and_wait(Duration::from_millis(150))
            .await
            .expect_err(
                "the outer runner_task exits cleanly once it stops consuming, but the child \
                 executor task still had to be force-aborted during the drain step — the \
                 final Result must reflect that, not a false Ok(())",
            );
        assert!(matches!(err, EffectRuntimeShutdownError::Timeout));
    }
}
