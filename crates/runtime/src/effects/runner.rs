//! Delivery runner (CORE-019 Phases 6-7): drains the internal [`EffectQueue`],
//! consults the `EffectStateStore`/`EffectDedupStore` ports, and drives one
//! attempt per effect through the `effect_type`-keyed [`ExecutorRegistry`].
//!
//! **AD-8 single-consumer invariant**: `EffectStateStore::claim_due` is
//! deliberately non-atomic (design.md AD-8) — it returns due effects without
//! transitioning their state. That is safe only when a single consumer drains
//! a given store/queue pair. Nothing in these types enforces this: a caller
//! that constructs and spawns a second [`DeliveryRunner`] against the same
//! store is responsible for not doing so; the type system does not prevent
//! it (see the `two_runners_can_share_the_same_store_the_type_system_does_not_prevent_it`
//! test below, which states this honestly rather than pretending otherwise).
//! The periodic reclaim loop added below (fix 4, PR2 review) runs on this
//! same single consumer task, not a second one — see [`DeliveryRunner::run_inner`].
//!
//! **AD-6 revision (PR2 round 2)**: an earlier cut of this file gave every
//! retryable effect a *second*, in-process redispatch path — a
//! `tokio::time::sleep`-then-`queue.send` task spawned per retry, racing the
//! very same `claim_due`-driven reclaim loop for the same effect once its
//! `next_at` passed (a real double-dispatch race), and whose own need to
//! re-register itself in the shared, shutdown-drained [`tokio::task::JoinSet`]
//! was deadlock-prone against a shutdown already waiting on that same task.
//! That timer is now gone entirely: [`Self::retry_or_give_up`] and
//! [`Self::finish_success`]'s exhausted-bookkeeping path only ever call
//! `mark_retryable(next_at)` — the periodic reclaim loop (below) is the SOLE
//! way a retryable/pending effect re-enters the queue. See design.md's AD-6
//! revision for the full rationale.
//!
//! **PR2 round 4 review follow-up.** [`Self::reclaim_due`] now transitions a
//! claimed effect to `InFlight` (`mark_in_flight`) *before* enqueueing it —
//! see [`QueuedEffect`] — closing a gap where the same effect could be
//! claimed and re-enqueued on multiple reclaim ticks before its first queue
//! entry ever reached `drain_one` (F-01). Dedup (`EffectDedupStore::reserve`)
//! is now consulted on *every* attempt, fresh or redispatched, keyed by
//! ownership (`EffectId`) and status (has this owner already succeeded) —
//! see [`Self::dispatch_in_flight`] — instead of being gated by
//! `effect.attempt == 0`, which used to misclassify a crash-recovered
//! re-attempt of an effect's own still-held reservation as `Duplicate` and
//! silently mark it `Succeeded` without ever re-executing (F-02). With that
//! gate gone, [`Self::requeue_without_charging_attempt`] no longer needs to
//! inflate `attempt` to skip re-reservation, so a shutdown cancellation no
//! longer silently eats into the effect's real retry budget (F-04). The
//! main drain loop's backpressure-permit wait is now raced against shutdown
//! too, so a hung executor holding every concurrency permit can no longer
//! prevent shutdown from ever reaching [`Self::drain_tasks`]'s
//! abort-on-deadline logic (F-03). Full rationale in design.md's "PR2 round
//! 4 review follow-up" note.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{watch, Mutex as AsyncMutex};
use tokio::task::JoinSet;

use crate::read_side::backpressure::Backpressure;

use super::executor::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
use super::policy::RetryPolicies;
use super::queue::{EffectQueue, EffectQueueReceiver, QueuedEffect};
use super::registry::ExecutorRegistry;
use super::store::{
    AcceptedEffect, DedupOutcome, DedupScope, EffectDedupStore, EffectFingerprint, EffectId,
    EffectStateStore, EffectStoreError, TerminalReason, Timestamp,
};

/// AD-7: bounded number of times a flat, un-backed-off bookkeeping write
/// (`commit_success`/`mark_succeeded`/`mark_retryable`, and `mark_in_flight`,
/// fix 4) is retried before giving up on this pass.
const BOOKKEEPING_RETRY_ATTEMPTS: u32 = 3;

/// Fix 4 (PR2 review): how often the periodic reclaim loop calls
/// `claim_due` to re-feed `Pending`/`RetryableFailed` effects that nothing
/// else would ever re-drive (e.g. `mark_in_flight` failed at accept-time,
/// or a crash left them behind before `recover_in_flight` next runs).
/// Chosen as a middle ground: frequent enough that a stuck effect isn't
/// abandoned for long, infrequent enough not to hammer the state store with
/// polling `claim_due` calls under normal operation where this path is
/// rarely needed at all.
const RECLAIM_INTERVAL: Duration = Duration::from_secs(5);

/// Fix 4: how many effects one reclaim tick claims at once — a small,
/// bounded batch so one tick can't monopolize the queue.
const RECLAIM_BATCH_LIMIT: usize = 32;

/// Fix 5 (PR2 review): bounded wait, once shutdown is signalled, for
/// already-spawned per-effect and backoff-redispatch tasks to finish before
/// `run`/`run_inner` returns.
///
/// ponytail: a fixed local constant, not yet threaded through from a caller
/// — PR4's `RuntimeEffectAcceptor::drain(deadline, ..)` (unmerged as of this
/// PR) already owns exactly this "bounded shutdown deadline" concept one
/// layer up; once PR4 lands, its `deadline` should flow down into this
/// runner instead of this constant duplicating the idea. Not required by
/// this PR's scope.
const SHUTDOWN_DRAIN_DEADLINE: Duration = Duration::from_secs(5);

/// HIGH-4: a safe fallback for [`timestamp_after`] when `duration` cannot be
/// represented as a [`chrono::Duration`] (`chrono::Duration::from_std` fails
/// for values approaching `std::time::Duration::MAX`, which a pathological
/// `RetryPolicy::max_backoff` could in principle set). Falling back to
/// [`chrono::Duration::zero`] — as this used to — would silently turn "wait
/// the maximum backoff" into "retry immediately", causing a retry storm
/// instead of backoff. 100 years is far longer than any real backoff value
/// while staying safely inside `chrono::DateTime<Utc>`'s representable range
/// from "now", so the later `Utc::now() + chrono_duration` addition itself
/// cannot overflow and panic.
fn saturated_backoff_fallback() -> chrono::Duration {
    chrono::Duration::days(365 * 100)
}

fn timestamp_after(duration: Duration) -> Timestamp {
    let chrono_duration =
        chrono::Duration::from_std(duration).unwrap_or_else(|_| saturated_backoff_fallback());
    Timestamp::from_utc(Utc::now() + chrono_duration)
}

/// Drains accepted effects through the one delivery pipeline: dedup reserve →
/// `mark_in_flight` → execute → bookkeeping. [`DeliveryRunner::drain_one`] is
/// the single code path both `Deferred`'s spawned drain loop
/// ([`DeliveryRunner::run`]) and `Inline` mode (design.md §7) call — the only
/// difference between the two profiles is *where* that call runs.
pub(crate) struct DeliveryRunner {
    state: Arc<dyn EffectStateStore>,
    dedup: Arc<dyn EffectDedupStore>,
    registry: Arc<ExecutorRegistry>,
    /// F-02: per-`effect_type` retry policy override, consulted via
    /// [`RetryPolicies::policy_for`] everywhere a single shared policy used
    /// to be read directly.
    retry: RetryPolicies,
    queue: EffectQueue,
    /// Fix 5 (PR2 review): every per-effect dispatch task is tracked here
    /// (instead of a bare `tokio::spawn` with a discarded handle) so shutdown
    /// can actually wait for it to drain within [`SHUTDOWN_DRAIN_DEADLINE`].
    /// Since the AD-6 revision removed the separate backoff-redispatch timer
    /// task, this set now only ever holds per-effect dispatch tasks spawned
    /// by [`Self::run_inner`]'s receiver branch — never a task that itself
    /// recursively spawns another tracked task, which is what made the old
    /// timer path deadlock-prone against shutdown (F-01).
    tasks: AsyncMutex<JoinSet<()>>,
    /// Gap 1 (PR2 residual fix): the [`tokio::task::AbortHandle`] of every
    /// currently in-flight *executor* attempt (the inner `tokio::spawn`
    /// inside [`execute_catching_panics`]), so [`Self::drain_tasks`] can
    /// abort a straggling attempt on drain-deadline expiry. Aborting here
    /// (rather than the outer `tasks` `JoinSet`) is what gives
    /// `classify_join_result`'s `is_cancelled()` branch a real production
    /// caller: the owning per-effect dispatch task observes the resulting
    /// `Cancelled` join error through its own existing `CancelledForShutdown`
    /// handling and finishes normally (fast — just bookkeeping calls), so it
    /// still drains cleanly out of `tasks` instead of being force-aborted
    /// mid-bookkeeping.
    executor_aborts: AsyncMutex<Vec<tokio::task::AbortHandle>>,
}

impl DeliveryRunner {
    pub(crate) fn new(
        state: Arc<dyn EffectStateStore>,
        dedup: Arc<dyn EffectDedupStore>,
        registry: Arc<ExecutorRegistry>,
        retry: impl Into<RetryPolicies>,
        queue: EffectQueue,
    ) -> Self {
        Self {
            state,
            dedup,
            registry,
            retry: retry.into(),
            queue,
            tasks: AsyncMutex::new(JoinSet::new()),
            executor_aborts: AsyncMutex::new(Vec::new()),
        }
    }

    /// Fix 5: tracks `fut` in the shared [`JoinSet`] instead of a detached
    /// `tokio::spawn`.
    async fn spawn_tracked<F>(&self, fut: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.tasks.lock().await.spawn(fut);
    }

    /// Fix 5: once shutdown has stopped the main loop from accepting new
    /// work, waits (bounded by `deadline`) for every outstanding tracked
    /// task to finish.
    ///
    /// Gap 1 (PR2 residual fix): a task still running once `deadline`
    /// elapses is no longer left running, untracked, in the background
    /// forever. It is aborted, and the resulting cancellation is drained out
    /// of `tasks` before this returns — see [`Self::executor_aborts`] for why
    /// the abort targets the inner executor attempt rather than the outer
    /// dispatch task directly.
    ///
    /// ponytail: holds the `tasks` lock for the whole drain window. A brand
    /// new task spawned by a straggling in-flight attempt during this exact
    /// window (e.g. a retry redispatch scheduled the instant shutdown
    /// began) queues behind this lock and, if the deadline elapses first,
    /// becomes an untracked background task once this returns — an edge
    /// case already covered by the same accepted at-least-once/duplicate-
    /// delivery tradeoff AD-6/AD-7 document elsewhere. Revisit only if this
    /// proves surprising in practice.
    async fn drain_tasks(&self, deadline: Duration) {
        let mut tasks = self.tasks.lock().await;
        let timed_out = tokio::time::timeout(deadline, async {
            while tasks.join_next().await.is_some() {}
        })
        .await
        .is_err();

        if !timed_out {
            return;
        }

        tracing::warn!(
            "shutdown drain deadline elapsed with dispatch tasks still outstanding; \
             aborting the underlying executor attempts to guarantee no leaked background work"
        );

        // Abort every still-running executor attempt. Each owning per-effect
        // dispatch task observes the resulting `Cancelled` join error through
        // its already-existing `CancelledForShutdown` handling
        // (`requeue_without_charging_attempt`) — a couple of immediate,
        // non-blocking bookkeeping calls — so it should finish and drain out
        // of `tasks` on its own right after.
        for handle in self.executor_aborts.lock().await.drain(..) {
            handle.abort();
        }

        let still_stuck = tokio::time::timeout(deadline, async {
            while tasks.join_next().await.is_some() {}
        })
        .await
        .is_err();

        if still_stuck {
            // Last resort: a dispatch task stuck somewhere other than the
            // executor await (e.g. a hung store call) — force it out so
            // shutdown provably leaves zero background tasks. The effect it
            // was processing may be left wherever it currently stood — the
            // same accepted stuck-until-crash-recovery tradeoff already
            // documented for a permanently failing bookkeeping write
            // elsewhere in this file.
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        }
    }

    /// Spawned drain loop (`Deferred` profile, AD-6): receives from the
    /// queue, bounds concurrency with [`Backpressure`] (fix 7, reusing
    /// `read_side::backpressure`), periodically reclaims due effects (fix
    /// 4), and stops on shutdown signal via `watch`, waiting for
    /// outstanding tasks to drain (fix 5) before returning.
    pub(crate) async fn run(
        self: Arc<Self>,
        receiver: EffectQueueReceiver,
        concurrency: usize,
        shutdown: watch::Receiver<bool>,
    ) {
        self.run_inner(
            receiver,
            concurrency,
            shutdown,
            RECLAIM_INTERVAL,
            SHUTDOWN_DRAIN_DEADLINE,
        )
        .await;
    }

    /// Test-observable variant of [`Self::run`] with a configurable reclaim
    /// interval and shutdown-drain deadline, so tests don't have to wait out
    /// the real [`RECLAIM_INTERVAL`]/[`SHUTDOWN_DRAIN_DEADLINE`].
    async fn run_inner(
        self: Arc<Self>,
        mut receiver: EffectQueueReceiver,
        concurrency: usize,
        mut shutdown: watch::Receiver<bool>,
        reclaim_interval: Duration,
        drain_deadline: Duration,
    ) {
        let backpressure = Arc::new(Backpressure::new(concurrency.max(1)));
        let mut reclaim_tick = tokio::time::interval(reclaim_interval);
        reclaim_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick fires immediately; skip it so a fresh runner
        // doesn't reclaim on startup before anything could possibly be due
        // to reclaim.
        reclaim_tick.reset();

        'drain: loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break 'drain;
                    }
                }
                _ = reclaim_tick.tick() => {
                    self.reclaim_due().await;
                }
                maybe_effect = receiver.recv() => {
                    let Some(queued) = maybe_effect else { break 'drain; };

                    // F-03 (PR2 round 4): race backpressure-permit
                    // acquisition against shutdown too. Before this fix,
                    // `backpressure.acquire()` was a bare `.await` outside
                    // any `select!` watching shutdown — a hung/slow
                    // executor holding every concurrency permit could block
                    // this loop from ever reaching `drain_tasks`'s
                    // abort-on-deadline logic at all.
                    let permit = loop {
                        tokio::select! {
                            permit = backpressure.acquire() => {
                                break permit.expect("backpressure semaphore not closed");
                            }
                            changed = shutdown.changed() => {
                                if changed.is_err() || *shutdown.borrow() {
                                    // This one queued effect is dropped
                                    // un-attempted rather than processed —
                                    // it is still safely `Pending`/
                                    // `RetryableFailed` (`Fresh`) or already
                                    // `InFlight` (`Reclaimed`, recoverable
                                    // via a future `recover_in_flight`) in
                                    // the store, never silently lost.
                                    break 'drain;
                                }
                                // Spurious wakeup: the shared `watch` value
                                // didn't actually change to `true`. Keep
                                // waiting for a permit.
                            }
                        }
                    };

                    let this = self.clone();
                    match queued {
                        QueuedEffect::Fresh(effect) => {
                            self.spawn_tracked(async move {
                                this.drain_one(effect).await;
                                drop(permit);
                            })
                            .await;
                        }
                        QueuedEffect::Reclaimed(effect) => {
                            self.spawn_tracked(async move {
                                this.drain_reclaimed(effect).await;
                                drop(permit);
                            })
                            .await;
                        }
                    }
                }
            }
        }

        // Fix 5: stop accepting new work above, then wait for every
        // outstanding dispatch/redispatch task to actually finish.
        self.drain_tasks(drain_deadline).await;
    }

    /// Fix 4 (PR2 review): re-feeds effects that are `Pending`/
    /// `RetryableFailed` and due back into the queue for another dispatch
    /// attempt. Runs on the same single-consumer loop as the main drain step
    /// (AD-8) — not a second consumer, just another branch of the same
    /// `select!`.
    ///
    /// **F-01 (PR2 round 4)**: `claim_due` itself doesn't transition state —
    /// before this fix, an effect stayed `Pending`/`RetryableFailed` until
    /// `drain_one` eventually called `mark_in_flight` on it (after being
    /// dequeued), so the SAME effect could be claimed and re-enqueued on
    /// every successive reclaim tick while the queue had backlog, inflating
    /// it with duplicate entries for one effect. Fixed: claim, THEN
    /// transition to `InFlight` right here, THEN enqueue — only effects that
    /// actually transition are ever enqueued (as [`QueuedEffect::Reclaimed`],
    /// so `drain_reclaimed` — not `drain_one` — handles them and doesn't
    /// attempt its own `mark_in_flight`, which would immediately fail with
    /// `InvalidTransition` since the effect is no longer `Pending`/
    /// `RetryableFailed`).
    async fn reclaim_due(&self) {
        let due = match self
            .state
            .claim_due(Timestamp::now(), RECLAIM_BATCH_LIMIT)
            .await
        {
            Ok(due) => due,
            // ponytail: a transient `claim_due` error just waits for the
            // next tick rather than retrying inline — the next tick is at
            // most `RECLAIM_INTERVAL` away and this path is best-effort by
            // nature (crash-recovery/orphan-reclaim, not the primary path).
            Err(_) => return,
        };
        for stored in due {
            if let Err(err) = self.state.mark_in_flight(stored.id).await {
                // Expected, harmless race: something else (e.g. the direct
                // accept-path queue entry for this same effect) already
                // transitioned it first — not a bug, just skip it.
                tracing::debug!(
                    effect_id = %stored.id,
                    error = %err,
                    "reclaim: mark_in_flight failed, skipping (likely already claimed elsewhere)"
                );
                continue;
            }
            let effect = AcceptedEffect {
                id: stored.id,
                tenant: stored.tenant,
                attempt: stored.attempt,
                description: stored.description,
            };
            if let Err(err) = self.queue.send_reclaimed(effect).await {
                tracing::warn!(
                    effect_id = %stored.id,
                    error = %err,
                    "reclaim: queue.send failed"
                );
            }
        }
    }

    /// One full attempt of one freshly-accepted effect (design.md §5 data
    /// flow) — needs `mark_in_flight` first. See [`Self::drain_reclaimed`]
    /// for the entry point used by [`Self::reclaim_due`]-fed effects (F-01),
    /// which are already `InFlight` by the time they get here.
    pub(crate) async fn drain_one(&self, effect: AcceptedEffect) {
        // `EffectStateStore::mark_terminal` only accepts `InFlight` or
        // `RetryableFailed` as its `from` state (store.rs, already shipped in
        // PR1) — so every short-circuit in `dispatch_in_flight` that needs
        // to record a terminal outcome must go through `mark_in_flight`
        // first. That means `mark_in_flight` runs ahead of the dedup
        // `reserve` call, one step earlier than design.md §5's informal
        // sketch, to stay within the real, already-shipped state machine.
        //
        // Fix 4: bounded-retry this write instead of giving up on the very
        // first transient error — a `TemporarilyUnavailable` failure here
        // used to silently orphan the effect in `Pending` forever (nothing
        // else ever re-fed it into the queue). If it still fails after the
        // bound (or the error is permanent), no side effect has happened
        // yet and no dedup was reserved, so the effect is safely still
        // `Pending` — the periodic reclaim loop above is what eventually
        // re-drives it.
        if self.mark_in_flight_with_retry(effect.id).await.is_err() {
            return;
        }
        self.dispatch_in_flight(effect).await;
    }

    /// F-01 (PR2 round 4): entry point for an effect [`Self::reclaim_due`]
    /// already claimed and transitioned to `InFlight` before ever enqueueing
    /// it (see [`QueuedEffect::Reclaimed`]). Must NOT call `mark_in_flight`
    /// again here — the effect is no longer `Pending`/`RetryableFailed`, so
    /// that would immediately fail with `InvalidTransition`.
    pub(crate) async fn drain_reclaimed(&self, effect: AcceptedEffect) {
        self.dispatch_in_flight(effect).await;
    }

    /// The shared dedup-reserve → execute → bookkeeping pipeline for an
    /// effect that is already `InFlight` — called by both [`Self::drain_one`]
    /// (after its own `mark_in_flight`) and [`Self::drain_reclaimed`] (whose
    /// caller already transitioned it).
    async fn dispatch_in_flight(&self, effect: AcceptedEffect) {
        let scope = DedupScope {
            tenant: effect.tenant.clone(),
            effect_type: effect.description.effect_type.clone(),
            key: effect.description.idempotency_key.clone(),
        };

        // F-02/F-04 (PR2 round 4 unified redesign): dedup is now consulted
        // on EVERY attempt — fresh or redispatched, including a post-crash
        // re-attempt — never gated by `effect.attempt == 0`. That gate used
        // to be the only way `drain_one` distinguished "never reserved yet"
        // from "already reserved by myself", by relying on every redispatch
        // path bumping `attempt` to skip re-reservation. But a crash mid the
        // very first attempt (before `mark_in_flight`'s caller's executor
        // call ever completed) leaves `attempt` at 0 even after
        // `recover_in_flight` resets the effect back to `Pending` — so the
        // re-attempt still had `attempt == 0`, called `reserve` again,
        // collided with its OWN still-held `Fresh` reservation, got back a
        // plain `Duplicate`, and the old code treated any `Duplicate` as
        // "already satisfied elsewhere" — silently marking the effect
        // `Succeeded` without ever actually re-executing it (F-02, a
        // silent-data-loss BLOCKER).
        //
        // The dedup store now tracks ownership (`EffectId`) and status (has
        // the owner already succeeded) — see `store.rs`'s `DedupOutcome` —
        // so the runner can tell these cases apart directly instead of
        // faking it via the attempt counter:
        let fingerprint = EffectFingerprint::compute(
            &effect.description.payload,
            &effect.description.destination,
        );
        match self.reserve_with_retry(&scope, effect.id, fingerprint).await {
            // No prior reservation, or this effect's own reservation that
            // hasn't succeeded yet (a fresh submission, or a legitimate
            // crash-recovery/retry re-attempt of itself) — proceed to
            // (re-)execute.
            Ok(DedupOutcome::Fresh) | Ok(DedupOutcome::OwnedInProgress) => {}
            // HIGH-1 (unchanged) + F-02 (PR2 round 4): either a genuinely
            // *different* submission already owns/completed this scope
            // (`Duplicate`), or this effect's OWN reservation was already
            // durably recorded `Succeeded` (`OwnedSucceeded` — e.g.
            // `finish_success`'s bookkeeping-exhausted path already got as
            // far as `dedup.commit_success` before `state.mark_succeeded`
            // kept failing). Both are "already handled, nothing to do" —
            // `OwnedSucceeded` is the ONLY case allowed to short-circuit to
            // success without re-executing, precisely because the dedup
            // store itself already durably recorded delivery for THIS
            // effect. Neither ever needs a release: `Duplicate` never owned
            // the reservation it collided with, and `OwnedSucceeded`'s
            // reservation is meant to stay held.
            Ok(DedupOutcome::Duplicate) | Ok(DedupOutcome::OwnedSucceeded) => {
                self.finish_already_satisfied(effect.id).await;
                return;
            }
            Ok(DedupOutcome::Conflict) => {
                self.abandon(
                    effect.id,
                    TerminalReason::InvalidEffect("dedup scope conflict".into()),
                )
                .await;
                return;
            }
            Err(()) => {
                self.abandon(
                    effect.id,
                    TerminalReason::Other("dedup store unavailable".into()),
                )
                .await;
                return;
            }
        }

        let Some(executor) = self.registry.get(&effect.description.effect_type) else {
            self.abandon_and_release(effect.id, TerminalReason::ExecutorMissing, scope)
                .await;
            return;
        };

        let ctx = EffectContext {
            effect_id: effect.id,
            tenant: effect.tenant.clone(),
            attempt: effect.attempt + 1,
            idempotency_key: effect.description.idempotency_key.clone(),
        };

        let outcome = execute_catching_panics(
            executor,
            effect.description.clone(),
            ctx,
            &self.executor_aborts,
        )
        .await;

        match outcome {
            ExecutionOutcome::Outcome(AttemptOutcome::Success) => {
                self.finish_success(effect, scope).await
            }
            ExecutionOutcome::Outcome(AttemptOutcome::RetryableFailure(_)) => {
                self.retry_or_give_up(effect, scope).await
            }
            ExecutionOutcome::Outcome(AttemptOutcome::TerminalFailure(reason)) => {
                self.abandon_and_release(effect.id, TerminalReason::Other(reason), scope)
                    .await;
            }
            ExecutionOutcome::CancelledForShutdown => {
                // HIGH-2: an aborted/cancelled executor task is external to
                // the effect's own behavior (see `classify_join_result`) —
                // requeue it without charging a retry attempt.
                self.requeue_without_charging_attempt(effect).await;
            }
        }
    }

    /// Fix 4: bounded retry for `mark_in_flight`, mirroring the flat
    /// (no-backoff) retry shape `finish_success` already uses for its own
    /// bookkeeping write, but classified per AD-9: only
    /// `TemporarilyUnavailable` is retried — everything else is permanent.
    async fn mark_in_flight_with_retry(&self, id: EffectId) -> Result<(), ()> {
        for _ in 0..BOOKKEEPING_RETRY_ATTEMPTS {
            match self.state.mark_in_flight(id).await {
                Ok(()) => return Ok(()),
                Err(EffectStoreError::TemporarilyUnavailable(_)) => continue,
                Err(_) => return Err(()),
            }
        }
        tracing::warn!(
            effect_id = %id,
            "mark_in_flight bookkeeping exhausted retries; leaving effect Pending for the reclaim loop"
        );
        Err(())
    }

    /// Fix 6 (PR2 review): classifies a `dedup.reserve` error the same way
    /// AD-9 already classifies `EffectStateStore::accept`'s errors —
    /// `TemporarilyUnavailable` gets a bounded, backed-off retry under the
    /// effect's own `effect_type` policy (F-02); everything else is
    /// immediately terminal, same as before this fix.
    async fn reserve_with_retry(
        &self,
        scope: &DedupScope,
        effect_id: EffectId,
        fingerprint: EffectFingerprint,
    ) -> Result<DedupOutcome, ()> {
        let policy = self.retry.policy_for(&scope.effect_type);
        let mut attempt: u32 = 0;
        loop {
            match self.dedup.reserve(scope, effect_id, fingerprint).await {
                Ok(outcome) => return Ok(outcome),
                Err(EffectStoreError::TemporarilyUnavailable(_)) => {
                    if !policy.allows_retry(attempt) {
                        return Err(());
                    }
                    let backoff = policy.backoff(attempt + 1);
                    if !backoff.is_zero() {
                        tokio::time::sleep(backoff).await;
                    }
                    attempt += 1;
                }
                Err(_) => return Err(()),
            }
        }
    }

    /// Fix 8 (PR2 review): `mark_terminal` only, no dedup release — the
    /// shape every call site that never reserved (or must not release) a
    /// dedup scope shares. HIGH-3: logs a bookkeeping failure instead of
    /// silently discarding it.
    async fn abandon(&self, id: EffectId, reason: TerminalReason) {
        if let Err(err) = self.state.mark_terminal(id, reason).await {
            tracing::warn!(effect_id = %id, error = %err, "mark_terminal failed while abandoning effect");
        }
    }

    /// Fix 8: `mark_terminal` then release the dedup reservation — the
    /// shape every genuinely-terminal-after-reserving call site shares.
    /// HIGH-3: logs each bookkeeping failure instead of silently discarding it.
    async fn abandon_and_release(&self, id: EffectId, reason: TerminalReason, scope: DedupScope) {
        if let Err(err) = self.state.mark_terminal(id, reason).await {
            tracing::warn!(effect_id = %id, error = %err, "mark_terminal failed while abandoning effect");
        }
        if let Err(err) = self.dedup.release(&scope).await {
            tracing::warn!(effect_id = %id, error = %err, "dedup release failed after abandoning effect");
        }
    }

    /// HIGH-1: a `DedupOutcome::Duplicate` on a fresh submission means a
    /// *different* submission collided with an already in-progress/completed
    /// effect under the same idempotency scope — "already handled, nothing
    /// to do", not a failure. `mark_in_flight` already ran for this effect,
    /// so `Succeeded` (`from: InFlight`) is a legal, honest terminal-happy
    /// transition; this effect never held the reservation it collided with,
    /// so there is nothing to release.
    async fn finish_already_satisfied(&self, id: EffectId) {
        if let Err(err) = self.state.mark_succeeded(id).await {
            tracing::warn!(effect_id = %id, error = %err, "mark_succeeded failed for an already-satisfied duplicate");
        }
    }

    /// Bounded retry for `mark_retryable`, mirroring
    /// [`Self::mark_in_flight_with_retry`]'s shape and AD-9 classification.
    /// AD-6 revision: this write is no longer merely for durability/
    /// observability — since the in-process redispatch timer is gone,
    /// `mark_retryable`'s success is what makes an effect reclaim-eligible
    /// at all. If it's still failing after the bound, the effect is left
    /// wherever it currently is (typically still `InFlight`) — a known,
    /// documented edge (mirrors the existing `mark_in_flight`
    /// permanent-failure tradeoff) resolved only by a future crash-recovery
    /// cycle (`recover_in_flight`), not by this round's scope.
    async fn mark_retryable_with_retry(
        &self,
        id: EffectId,
        attempt: u32,
        next_at: Timestamp,
    ) -> Result<(), ()> {
        for _ in 0..BOOKKEEPING_RETRY_ATTEMPTS {
            match self.state.mark_retryable(id, attempt, next_at).await {
                Ok(()) => return Ok(()),
                Err(EffectStoreError::TemporarilyUnavailable(_)) => continue,
                Err(_) => return Err(()),
            }
        }
        Err(())
    }

    /// AD-7 + F-03: on a successful attempt, bounded-retry the idempotent
    /// bookkeeping write; if it still fails, make the effect reclaim-eligible
    /// again instead of leaving it stranded `InFlight` forever. The effect
    /// already succeeded at the destination — only the bookkeeping write
    /// failed — so per AD-7's accepted tradeoff, redispatching it may
    /// re-execute against the destination; a cooperating destination's
    /// mandatory idempotency-key handling (design.md §6.5) absorbs the
    /// resulting duplicate delivery.
    ///
    /// F-03 fix: the effect's stored state is still `InFlight` at this
    /// point. Neither `drain_one`'s own `mark_in_flight` (`allowed_from:
    /// [Pending, RetryableFailed]`) nor the reclaim loop's `claim_due`
    /// (which explicitly excludes `InFlight`) can ever pick it back up while
    /// it stays there. `mark_retryable`'s `allowed_from` is exactly
    /// `[InFlight]` (store.rs) — it fits this "succeeded but bookkeeping
    /// didn't confirm" case perfectly, no new store operation needed.
    async fn finish_success(&self, effect: AcceptedEffect, scope: DedupScope) {
        for _ in 0..BOOKKEEPING_RETRY_ATTEMPTS {
            if self.dedup.commit_success(&scope).await.is_ok()
                && self.state.mark_succeeded(effect.id).await.is_ok()
            {
                return;
            }
        }
        tracing::warn!(
            effect_id = %effect.id,
            "post-success bookkeeping exhausted retries; making effect reclaim-eligible per AD-7/F-03"
        );
        let next_attempt = effect.attempt + 1;
        let policy = self.retry.policy_for(&scope.effect_type);
        let next_at = timestamp_after(policy.backoff(next_attempt.max(1)));
        // Dedup reservation stays held (dedup-lifetime redesign, `drain_one`
        // doc comment): the effect already succeeded, so its own reservation
        // must still be there when the reclaim loop re-enters `drain_one`
        // for it — `next_attempt >= 1` makes that re-entry skip `reserve`
        // entirely, never risking its own `Duplicate`.
        if self
            .mark_retryable_with_retry(effect.id, next_attempt, next_at)
            .await
            .is_err()
        {
            tracing::warn!(
                effect_id = %effect.id,
                "mark_retryable failed while making a successful-but-unconfirmed effect \
                 reclaim-eligible; it may remain stuck InFlight until crash recovery"
            );
        }
    }

    /// AD-6 (revised): on a retryable delivery failure, `mark_retryable`
    /// alone makes this effect reclaim-eligible — the periodic reclaim
    /// loop's next `claim_due` tick is the SOLE way it re-enters the queue
    /// once `next_at` passes. There is no more separate in-process
    /// sleep-then-`queue.send` timer (removed: it raced the reclaim loop for
    /// the same effect, and its need to self-register in the shared
    /// `JoinSet` was deadlock-prone against shutdown — F-01).
    async fn retry_or_give_up(&self, effect: AcceptedEffect, scope: DedupScope) {
        let policy = self.retry.policy_for(&scope.effect_type);
        if !policy.allows_retry(effect.attempt) {
            self.abandon_and_release(
                effect.id,
                TerminalReason::Other("attempt cap exceeded".into()),
                scope,
            )
            .await;
            return;
        }

        let dispatched_attempt = effect.attempt + 1;
        let backoff = policy.backoff(dispatched_attempt);
        let next_at = timestamp_after(backoff);

        if self
            .mark_retryable_with_retry(effect.id, dispatched_attempt, next_at)
            .await
            .is_err()
        {
            // Gap 2 (PR2 residual fix): `mark_retryable`'s own bounded retry
            // (above) already tried `BOOKKEEPING_RETRY_ATTEMPTS` times. If
            // it's STILL failing, this is a genuine store outage, not a
            // one-off blip — leaving the effect silently `InFlight` forever
            // (as before this fix) is the same "stuck and undiscoverable"
            // class of bug F-03 already fixed for `finish_success`'s
            // exhausted path. Abandon it properly instead: mark terminal and
            // release the dedup reservation, so an operator sees a real
            // signal instead of a silent, permanent stall.
            tracing::warn!(
                effect_id = %effect.id,
                "mark_retryable bookkeeping exhausted retries; abandoning the effect instead \
                 of leaving it stuck InFlight"
            );
            self.abandon_and_release(
                effect.id,
                TerminalReason::Other("retry bookkeeping exhausted: store unavailable".into()),
                scope,
            )
            .await;
        }

        // Dedup reservation stays held for the effect's entire lifetime
        // (dedup-lifetime redesign, `drain_one` doc comment) — released only
        // on a genuinely terminal outcome, never here. (No-op here when the
        // branch above already abandoned-and-released it.)
    }

    /// HIGH-2: re-queues an executor attempt that was aborted/cancelled
    /// (shutdown-triggered — see `classify_join_result`) for the next
    /// reclaim cycle, immediately due. Bypasses the attempt-cap check
    /// entirely — unlike a real `RetryableFailure`, this event can never
    /// exhaust the effect's retry budget, since the effect did not fail on
    /// its own merits.
    ///
    /// **F-04 (PR2 round 4)**: this used to bump `effect.attempt` purely so
    /// a redispatch would skip re-reserving dedup under the (now-removed)
    /// `effect.attempt == 0` gate — but `RetryPolicy::allows_retry`/
    /// `policy_for` consult that very same counter for the retry cap, so a
    /// cancellation silently ate into the effect's real retry budget despite
    /// being documented as never able to exhaust it. Dedup reservation is
    /// now identity-based (F-02/F-04 above), not attempt-gated, so this no
    /// longer needs to touch `attempt` at all — the stored attempt count is
    /// left exactly as it was before the cancellation, provably free with
    /// respect to the retry cap.
    async fn requeue_without_charging_attempt(&self, effect: AcceptedEffect) {
        if self
            .mark_retryable_with_retry(effect.id, effect.attempt, Timestamp::now())
            .await
            .is_err()
        {
            tracing::warn!(
                effect_id = %effect.id,
                "mark_retryable failed while requeueing a shutdown-cancelled attempt; it may \
                 remain stuck InFlight until crash recovery"
            );
        }
    }
}

/// The outcome of racing one attempt's spawned executor task to completion —
/// distinguishes a normal `AttemptOutcome` from a cancelled/aborted task
/// (HIGH-2), which is never charged as a delivery failure.
enum ExecutionOutcome {
    Outcome(AttemptOutcome),
    /// The spawned executor task was cancelled/aborted rather than
    /// completing or panicking — see `classify_join_result`.
    CancelledForShutdown,
}

/// Gap 1 (PR2 residual fix): `executor_aborts` receives this attempt's
/// [`tokio::task::AbortHandle`] before it is awaited, so
/// [`DeliveryRunner::drain_tasks`] can abort it on drain-deadline expiry.
/// Stale (already-finished) handles are swept out opportunistically on every
/// call rather than tracked precisely by id — this set stays small (bounded
/// by in-flight concurrency) so an occasional linear scan is cheap.
async fn execute_catching_panics(
    executor: Arc<dyn ExternalEffectExecutor>,
    description: Arc<ego_domain::ExternalEffectDescription>,
    ctx: EffectContext,
    executor_aborts: &AsyncMutex<Vec<tokio::task::AbortHandle>>,
) -> ExecutionOutcome {
    let handle = tokio::spawn(async move { executor.execute(&description, &ctx).await });
    {
        let mut aborts = executor_aborts.lock().await;
        aborts.retain(|h| !h.is_finished());
        aborts.push(handle.abort_handle());
    }
    classify_join_result(handle.await)
}

/// HIGH-2: a `tokio::task::JoinError` is only ever one of two kinds — a
/// panic, or a cancellation (the task was aborted, e.g. because the async
/// runtime itself is shutting down mid-attempt). The effect itself did not
/// cause a cancellation; charging it a retry attempt purely because of
/// external shutdown timing would be unfair and could exhaust its retry
/// budget for reasons entirely unrelated to its own behavior. A panic, by
/// contrast, is the executor's own bug and is still charged as a retryable
/// failure, unchanged from before this fix.
fn classify_join_result(
    result: Result<AttemptOutcome, tokio::task::JoinError>,
) -> ExecutionOutcome {
    match result {
        Ok(outcome) => ExecutionOutcome::Outcome(outcome),
        Err(join_err) if join_err.is_panic() => {
            ExecutionOutcome::Outcome(AttemptOutcome::RetryableFailure(
                "executor panicked".to_string(),
            ))
        }
        Err(_) => ExecutionOutcome::CancelledForShutdown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::policy::{DeliveryConfig, RetryPolicy, RunnerMode};
    use crate::effects::store::{EffectId, EffectState, EffectStoreError, InMemoryEffectStore, StoredEffect};
    use async_trait::async_trait;
    use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::time::Duration as StdDuration;

    fn description(effect_type: &str, key: &str) -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new(key).unwrap(),
            effect_type: effect_type.to_string(),
            payload: vec![1, 2, 3],
            destination: "https://example.com".to_string(),
        }
    }

    fn accepted(id: EffectId, effect_type: &str, key: &str) -> AcceptedEffect {
        AcceptedEffect {
            id,
            tenant: TenantId::new("tenant-a").unwrap(),
            attempt: 0,
            description: Arc::new(description(effect_type, key)),
        }
    }

    struct ScriptedExecutor {
        outcomes: std::sync::Mutex<Vec<AttemptOutcome>>,
        calls: AtomicUsize,
    }

    impl ScriptedExecutor {
        fn new(outcomes: Vec<AttemptOutcome>) -> Self {
            Self {
                outcomes: std::sync::Mutex::new(outcomes),
                calls: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ExternalEffectExecutor for ScriptedExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut outcomes = self.outcomes.lock().unwrap();
            if outcomes.is_empty() {
                AttemptOutcome::Success
            } else {
                outcomes.remove(0)
            }
        }
    }

    /// Gap 1 (PR2 residual fix): an executor whose `execute` never returns on
    /// its own — simulates a task still running past the shutdown drain
    /// deadline. Signals `started` once it has actually begun executing, so
    /// the test can wait for that before triggering shutdown.
    struct HangingExecutor {
        started: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl ExternalEffectExecutor for HangingExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            self.started.notify_one();
            std::future::pending::<()>().await;
            unreachable!("this executor never returns on its own");
        }
    }

    struct PanickingOnceExecutor {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ExternalEffectExecutor for PanickingOnceExecutor {
        async fn execute(
            &self,
            _effect: &ExternalEffectDescription,
            _ctx: &EffectContext,
        ) -> AttemptOutcome {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                panic!("simulated executor panic");
            }
            AttemptOutcome::Success
        }
    }

    /// Delegates to an inner [`InMemoryEffectStore`], failing
    /// `mark_succeeded` a configurable number of times first — lets AD-7's
    /// bounded-retry-then-stay-in-flight path be tested without a real
    /// durable backend.
    struct FlakyBookkeepingStore {
        inner: InMemoryEffectStore,
        mark_succeeded_failures_left: AtomicU32,
    }

    impl FlakyBookkeepingStore {
        fn new(failures: u32) -> Self {
            Self {
                inner: InMemoryEffectStore::new(),
                mark_succeeded_failures_left: AtomicU32::new(failures),
            }
        }
    }

    #[async_trait]
    impl EffectStateStore for FlakyBookkeepingStore {
        async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
            self.inner.accept(effect).await
        }
        async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.inner.mark_in_flight(id).await
        }
        async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
            if self.mark_succeeded_failures_left.load(Ordering::SeqCst) > 0 {
                self.mark_succeeded_failures_left
                    .fetch_sub(1, Ordering::SeqCst);
                return Err(EffectStoreError::TemporarilyUnavailable("flaky".into()));
            }
            self.inner.mark_succeeded(id).await
        }
        async fn mark_retryable(
            &self,
            id: EffectId,
            attempt: u32,
            next_at: Timestamp,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_retryable(id, attempt, next_at).await
        }
        async fn mark_terminal(
            &self,
            id: EffectId,
            reason: TerminalReason,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_terminal(id, reason).await
        }
        async fn claim_due(
            &self,
            now: Timestamp,
            limit: usize,
        ) -> Result<Vec<StoredEffect>, EffectStoreError> {
            self.inner.claim_due(now, limit).await
        }
        async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
            self.inner.recover_in_flight(now).await
        }
    }

    /// Delegates to an inner [`InMemoryEffectStore`], failing
    /// `mark_retryable` unconditionally — proves fixes 1/3 without a real
    /// durable backend.
    struct AlwaysFailingMarkRetryableStore {
        inner: InMemoryEffectStore,
    }

    impl AlwaysFailingMarkRetryableStore {
        fn new() -> Self {
            Self {
                inner: InMemoryEffectStore::new(),
            }
        }
    }

    #[async_trait]
    impl EffectStateStore for AlwaysFailingMarkRetryableStore {
        async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
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
            _id: EffectId,
            _attempt: u32,
            _next_at: Timestamp,
        ) -> Result<(), EffectStoreError> {
            Err(EffectStoreError::TemporarilyUnavailable(
                "bookkeeping store down".into(),
            ))
        }
        async fn mark_terminal(
            &self,
            id: EffectId,
            reason: TerminalReason,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_terminal(id, reason).await
        }
        async fn claim_due(
            &self,
            now: Timestamp,
            limit: usize,
        ) -> Result<Vec<StoredEffect>, EffectStoreError> {
            self.inner.claim_due(now, limit).await
        }
        async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
            self.inner.recover_in_flight(now).await
        }
    }

    /// Delegates to an inner [`InMemoryEffectStore`], failing
    /// `mark_retryable` a configurable number of times first — proves the
    /// bounded-retry path added to `mark_retryable` in the PR2 round 2
    /// redesign.
    struct FlakyMarkRetryableStore {
        inner: InMemoryEffectStore,
        failures_left: AtomicU32,
    }

    impl FlakyMarkRetryableStore {
        fn new(failures: u32) -> Self {
            Self {
                inner: InMemoryEffectStore::new(),
                failures_left: AtomicU32::new(failures),
            }
        }
    }

    #[async_trait]
    impl EffectStateStore for FlakyMarkRetryableStore {
        async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
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
            next_at: Timestamp,
        ) -> Result<(), EffectStoreError> {
            if self.failures_left.load(Ordering::SeqCst) > 0 {
                self.failures_left.fetch_sub(1, Ordering::SeqCst);
                return Err(EffectStoreError::TemporarilyUnavailable("flaky".into()));
            }
            self.inner.mark_retryable(id, attempt, next_at).await
        }
        async fn mark_terminal(
            &self,
            id: EffectId,
            reason: TerminalReason,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_terminal(id, reason).await
        }
        async fn claim_due(
            &self,
            now: Timestamp,
            limit: usize,
        ) -> Result<Vec<StoredEffect>, EffectStoreError> {
            self.inner.claim_due(now, limit).await
        }
        async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
            self.inner.recover_in_flight(now).await
        }
    }

    /// Delegates to an inner [`InMemoryEffectStore`], failing
    /// `mark_in_flight` a configurable number of times first — proves fix
    /// 4's bounded-retry path.
    struct FlakyMarkInFlightStore {
        inner: InMemoryEffectStore,
        failures_left: AtomicU32,
    }

    impl FlakyMarkInFlightStore {
        fn new(failures: u32) -> Self {
            Self {
                inner: InMemoryEffectStore::new(),
                failures_left: AtomicU32::new(failures),
            }
        }
    }

    #[async_trait]
    impl EffectStateStore for FlakyMarkInFlightStore {
        async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
            self.inner.accept(effect).await
        }
        async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
            if self.failures_left.load(Ordering::SeqCst) > 0 {
                self.failures_left.fetch_sub(1, Ordering::SeqCst);
                return Err(EffectStoreError::TemporarilyUnavailable("flaky".into()));
            }
            self.inner.mark_in_flight(id).await
        }
        async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.inner.mark_succeeded(id).await
        }
        async fn mark_retryable(
            &self,
            id: EffectId,
            attempt: u32,
            next_at: Timestamp,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_retryable(id, attempt, next_at).await
        }
        async fn mark_terminal(
            &self,
            id: EffectId,
            reason: TerminalReason,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_terminal(id, reason).await
        }
        async fn claim_due(
            &self,
            now: Timestamp,
            limit: usize,
        ) -> Result<Vec<StoredEffect>, EffectStoreError> {
            self.inner.claim_due(now, limit).await
        }
        async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
            self.inner.recover_in_flight(now).await
        }
    }

    /// Delegates to an inner [`InMemoryEffectStore`], failing
    /// `mark_in_flight` unconditionally and permanently — proves fix 4's
    /// "permanent error leaves the effect safely `Pending`" branch.
    struct AlwaysFailingMarkInFlightStore {
        inner: InMemoryEffectStore,
    }

    impl AlwaysFailingMarkInFlightStore {
        fn new() -> Self {
            Self {
                inner: InMemoryEffectStore::new(),
            }
        }
    }

    #[async_trait]
    impl EffectStateStore for AlwaysFailingMarkInFlightStore {
        async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
            self.inner.accept(effect).await
        }
        async fn mark_in_flight(&self, _id: EffectId) -> Result<(), EffectStoreError> {
            Err(EffectStoreError::Backend("permanently down".into()))
        }
        async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
            self.inner.mark_succeeded(id).await
        }
        async fn mark_retryable(
            &self,
            id: EffectId,
            attempt: u32,
            next_at: Timestamp,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_retryable(id, attempt, next_at).await
        }
        async fn mark_terminal(
            &self,
            id: EffectId,
            reason: TerminalReason,
        ) -> Result<(), EffectStoreError> {
            self.inner.mark_terminal(id, reason).await
        }
        async fn claim_due(
            &self,
            now: Timestamp,
            limit: usize,
        ) -> Result<Vec<StoredEffect>, EffectStoreError> {
            self.inner.claim_due(now, limit).await
        }
        async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
            self.inner.recover_in_flight(now).await
        }
    }

    /// Delegates to an inner [`InMemoryEffectStore`], failing `reserve` a
    /// configurable number of times first — proves fix 6's bounded-retry
    /// path for the dedup store.
    struct FlakyReserveDedupStore {
        inner: InMemoryEffectStore,
        failures_left: AtomicU32,
    }

    impl FlakyReserveDedupStore {
        fn new(failures: u32) -> Self {
            Self {
                inner: InMemoryEffectStore::new(),
                failures_left: AtomicU32::new(failures),
            }
        }
    }

    #[async_trait]
    impl EffectDedupStore for FlakyReserveDedupStore {
        async fn reserve(
            &self,
            scope: &DedupScope,
            effect_id: EffectId,
            fingerprint: EffectFingerprint,
        ) -> Result<DedupOutcome, EffectStoreError> {
            if self.failures_left.load(Ordering::SeqCst) > 0 {
                self.failures_left.fetch_sub(1, Ordering::SeqCst);
                return Err(EffectStoreError::TemporarilyUnavailable(
                    "dedup store flaky".into(),
                ));
            }
            self.inner.reserve(scope, effect_id, fingerprint).await
        }
        async fn commit_success(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
            self.inner.commit_success(scope).await
        }
        async fn release(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
            self.inner.release(scope).await
        }
    }

    /// Fails `reserve` unconditionally with a permanent error — proves fix
    /// 6's "everything else stays immediately terminal" branch.
    struct AlwaysFailingReserveDedupStore;

    #[async_trait]
    impl EffectDedupStore for AlwaysFailingReserveDedupStore {
        async fn reserve(
            &self,
            _scope: &DedupScope,
            _effect_id: EffectId,
            _fingerprint: EffectFingerprint,
        ) -> Result<DedupOutcome, EffectStoreError> {
            Err(EffectStoreError::Backend("dedup store corrupted".into()))
        }
        async fn commit_success(&self, _scope: &DedupScope) -> Result<(), EffectStoreError> {
            Ok(())
        }
        async fn release(&self, _scope: &DedupScope) -> Result<(), EffectStoreError> {
            Ok(())
        }
    }

    fn runner_with(
        state: Arc<dyn EffectStateStore>,
        dedup: Arc<dyn EffectDedupStore>,
        registry: ExecutorRegistry,
        retry: impl Into<RetryPolicies>,
    ) -> (Arc<DeliveryRunner>, EffectQueue) {
        let (queue, _receiver) = EffectQueue::bounded(8);
        let runner = Arc::new(DeliveryRunner::new(
            state,
            dedup,
            Arc::new(registry),
            retry,
            queue.clone(),
        ));
        (runner, queue)
    }

    #[tokio::test]
    async fn happy_path_success_marks_succeeded_and_commits_dedup() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(ScriptedExecutor::new(vec![])))
            .unwrap();
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            registry,
            RetryPolicy::default(),
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::Succeeded,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn retryable_failure_is_reclaimed_by_the_reclaim_loop_and_eventually_succeeds() {
        // AD-6 revision: `retry_or_give_up` no longer spawns a
        // sleep-then-`queue.send` timer — `mark_retryable` alone is what
        // makes the effect reclaim-eligible; the periodic reclaim loop's
        // `claim_due` tick is the only way it ever gets a second attempt.
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![AttemptOutcome::RetryableFailure(
            "timeout".into(),
        )]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let fast_retry = RetryPolicy {
            max_attempts: 3,
            base_backoff: StdDuration::from_millis(5),
            max_backoff: StdDuration::from_millis(5),
        };
        let (queue, receiver) = EffectQueue::bounded(8);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            fast_retry,
            queue,
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        // Immediately after the failed attempt: no task/queue entry exists
        // for it yet (no timer to spawn one) — only the reclaim loop below
        // will ever redrive it.
        assert_eq!(
            executor.call_count(),
            1,
            "no second attempt must happen until the reclaim loop actually ticks"
        );

        // Only the periodic reclaim loop redrives it, once `next_at` passes.
        let loop_handle = tokio::spawn(runner.clone().run_inner(
            receiver,
            2,
            shutdown_rx,
            StdDuration::from_millis(10),
            SHUTDOWN_DRAIN_DEADLINE,
        ));

        tokio::time::timeout(StdDuration::from_secs(1), async {
            while executor.call_count() < 2 {
                tokio::time::sleep(StdDuration::from_millis(5)).await;
            }
        })
        .await
        .expect("the reclaim loop redelivers the retryable effect within timeout");

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(StdDuration::from_secs(1), loop_handle)
            .await
            .expect("run_inner returns after shutdown")
            .expect("task did not panic");

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::Succeeded,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn executor_missing_marks_terminal_and_releases_dedup() {
        let store = Arc::new(InMemoryEffectStore::new());
        let registry = ExecutorRegistry::new(); // nothing registered
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            registry,
            RetryPolicy::default(),
        );

        let id = EffectId::new();
        let effect = accepted(id, "unregistered.type", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::TerminalFailed,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn dedup_conflict_marks_invalid_effect_terminal() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(ScriptedExecutor::new(vec![])))
            .unwrap();
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            registry,
            RetryPolicy::default(),
        );

        // Reserve the scope up front with a *different* fingerprint so this
        // effect's own reserve() call reports Conflict.
        let scope = DedupScope {
            tenant: TenantId::new("tenant-a").unwrap(),
            effect_type: "invoice.created".to_string(),
            key: IdempotencyKey::new("uow-1:0").unwrap(),
        };
        store
            .reserve(
                &scope,
                EffectId::new(),
                EffectFingerprint::compute(b"different-payload", "https://different.example.com"),
            )
            .await
            .unwrap();

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::TerminalFailed,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn executor_panic_counts_as_one_retryable_attempt() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(PanickingOnceExecutor {
            calls: AtomicUsize::new(0),
        });
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        // Zero backoff so the redispatched effect is immediately `claim_due`
        // eligible, no reclaim-loop timing needed for this test.
        let immediate_retry = RetryPolicy {
            max_attempts: 3,
            base_backoff: StdDuration::ZERO,
            max_backoff: StdDuration::ZERO,
        };
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            registry,
            immediate_retry,
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;
        // Panic is caught and classified as a retryable failure (via
        // `classify_join_result`'s `is_panic()` arm), not a crash — the
        // effect becomes reclaim-eligible for a second attempt.
        let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempt, 1);
        let redispatched = AcceptedEffect {
            id: due[0].id,
            tenant: due[0].tenant.clone(),
            attempt: due[0].attempt,
            description: due[0].description.clone(),
        };
        runner.drain_one(redispatched).await;

        assert_eq!(executor.calls.load(Ordering::SeqCst), 2);
        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::Succeeded,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn bookkeeping_write_eventually_succeeds_within_bounded_retries() {
        let store = Arc::new(FlakyBookkeepingStore::new(BOOKKEEPING_RETRY_ATTEMPTS - 1));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(ScriptedExecutor::new(vec![])))
            .unwrap();
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            registry,
            RetryPolicy::default(),
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::Succeeded,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn bookkeeping_write_exhausted_makes_effect_reclaim_eligible_not_stuck_in_flight() {
        // F-03: `finish_success`'s bookkeeping-exhausted path used to leave
        // the effect `InFlight` forever — neither `drain_one`'s own
        // `mark_in_flight` (`allowed_from: [Pending, RetryableFailed]`) nor
        // the reclaim loop's `claim_due` (which explicitly excludes
        // `InFlight`) could ever pick it back up. It must transition via
        // `mark_retryable` (`allowed_from: [InFlight]` — fits perfectly) so
        // the reclaim loop can reclaim it normally.
        let store = Arc::new(FlakyBookkeepingStore::new(u32::MAX));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let immediate_retry = RetryPolicy {
            max_attempts: 3,
            base_backoff: StdDuration::ZERO,
            max_backoff: StdDuration::ZERO,
        };
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup.clone() as Arc<dyn EffectDedupStore>,
            registry,
            immediate_retry,
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        // No longer stuck `InFlight` forever — reclaim-eligible instead.
        let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert_eq!(
            due.len(),
            1,
            "the effect must be reclaim-eligible, not stuck InFlight forever"
        );
        assert_eq!(due[0].state, EffectState::RetryableFailed);
        assert_eq!(due[0].attempt, 1);

        // F-03 dedup semantics (unchanged): the effect already succeeded at
        // the destination, so its own reservation (made `Fresh` on its first
        // attempt) must still be held. F-02/F-04 (PR2 round 4): the dedup
        // store's own `commit_success` already durably recorded success on
        // the very first `finish_success` retry iteration (only the state
        // store's `mark_succeeded` kept failing) — `reserve` for this SAME
        // effect id now reports the more precise `OwnedSucceeded`, not the
        // old ambiguous `Duplicate`.
        let scope = DedupScope {
            tenant: TenantId::new("tenant-a").unwrap(),
            effect_type: "invoice.created".to_string(),
            key: IdempotencyKey::new("uow-1:0").unwrap(),
        };
        let fp = EffectFingerprint::compute(&[1, 2, 3], "https://example.com");
        assert_eq!(
            dedup.reserve(&scope, id, fp).await.unwrap(),
            DedupOutcome::OwnedSucceeded,
            "the effect's own reservation must still be held and known-succeeded, not released"
        );

        // F-02/F-04 (PR2 round 4): redispatching this specific effect must
        // now short-circuit to success WITHOUT re-executing — the dedup
        // store already knows this exact effect delivered successfully, so
        // re-running the executor (the old, coarser AD-7 tradeoff that
        // treated every reclaim-eligible-after-success effect the same,
        // whether or not dedup had actually confirmed success) is no longer
        // necessary.
        let redispatched = AcceptedEffect {
            id: due[0].id,
            tenant: due[0].tenant.clone(),
            attempt: due[0].attempt,
            description: due[0].description.clone(),
        };
        runner.drain_one(redispatched).await;

        assert_eq!(
            executor.call_count(),
            1,
            "a known-already-succeeded dedup reservation must short-circuit, never re-execute"
        );
    }

    #[tokio::test]
    async fn two_runners_can_share_the_same_store_the_type_system_does_not_prevent_it() {
        // AD-8 documents this as a caller responsibility, not a type-system
        // invariant — this test proves the type system really does allow it
        // by constructing two runners over one shared store.
        let store = Arc::new(InMemoryEffectStore::new());
        let (_runner_a, _queue_a) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            ExecutorRegistry::new(),
            RetryPolicy::default(),
        );
        let (_runner_b, _queue_b) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            ExecutorRegistry::new(),
            RetryPolicy::default(),
        );

        // Both runners really do hold live references to the same store —
        // this is what makes the non-atomic `claim_due` race possible if
        // both were driven concurrently, which is exactly why AD-8 requires
        // the caller (not the compiler) to keep it to one.
        assert!(Arc::strong_count(&store) >= 3);
    }

    #[tokio::test]
    async fn deferred_drain_loop_processes_queued_effects_and_stops_on_shutdown() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let (queue, receiver) = EffectQueue::bounded(4);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            RetryPolicy::default(),
            queue.clone(),
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();
        queue.send(effect).await.unwrap();

        let loop_handle = tokio::spawn(runner.clone().run(receiver, 2, shutdown_rx));

        // Give the spawned drain loop a moment to actually pull the queued
        // effect through `drain_one` before signalling shutdown. Polling the
        // executor's call count (not the store) avoids racing the drain
        // loop's own `mark_in_flight` transition.
        tokio::time::timeout(StdDuration::from_secs(1), async {
            while executor.call_count() == 0 {
                tokio::time::sleep(StdDuration::from_millis(5)).await;
            }
        })
        .await
        .expect("drain loop processes the queued effect within timeout");

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(StdDuration::from_secs(1), loop_handle)
            .await
            .expect("run() returns promptly after shutdown signal")
            .expect("drain loop task did not panic");

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::Succeeded,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn immediate_delivery_config_runs_the_same_pipeline_and_signals_failure_without_retry() {
        // Phase 7 (design.md §7): `DeliveryConfig::immediate()` is a
        // configuration of the one pipeline, not a bypass. Proof: this test
        // calls the exact same `drain_one` the Deferred drain loop calls,
        // just synchronously and with the `immediate()` retry policy — no
        // separate accept()->execute() shortcut exists anywhere in this file.
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![AttemptOutcome::RetryableFailure(
            "boom".into(),
        )]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let config = DeliveryConfig::immediate();
        assert_eq!(config.runner_mode, RunnerMode::Inline);
        let (queue, _receiver) = EffectQueue::bounded(config.queue_capacity);
        let runner = DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            config.retry,
            queue,
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        assert_eq!(
            executor.call_count(),
            1,
            "immediate() is zero-retry: exactly one attempt, then a signaled failure"
        );
        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::TerminalFailed,
                ..
            }
        ));
    }

    // --- Fix 1 & 3 (PR2 round 2 redesign): `mark_retryable` bookkeeping
    // failure no longer abandons the effect, and the dedup reservation is
    // never early-released — it stays held for the effect's entire lifetime,
    // not just "until redispatch" (there is no more redispatch task to
    // release it before). --------------------------------------------------

    #[tokio::test]
    async fn mark_retryable_transient_failure_retries_then_becomes_reclaim_eligible() {
        let state = Arc::new(FlakyMarkRetryableStore::new(BOOKKEEPING_RETRY_ATTEMPTS - 1));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![AttemptOutcome::RetryableFailure(
            "timeout".into(),
        )]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        // Zero backoff so the effect is immediately `claim_due`-eligible —
        // no reclaim-loop timing needed for this test.
        let immediate_retry = RetryPolicy {
            max_attempts: 3,
            base_backoff: StdDuration::ZERO,
            max_backoff: StdDuration::ZERO,
        };
        let (runner, _queue) = runner_with(
            state.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            registry,
            immediate_retry,
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        state.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        let due = state.claim_due(Timestamp::now(), 10).await.unwrap();
        assert_eq!(due.len(), 1, "mark_retryable's bounded retry must eventually succeed");
        assert_eq!(due[0].state, EffectState::RetryableFailed);
    }

    #[tokio::test]
    async fn mark_retryable_permanent_failure_abandons_effect_instead_of_leaving_it_stuck_in_flight()
    {
        // Gap 2 (PR2 residual fix): `mark_retryable`'s own bounded retry
        // (already exhausted here — `AlwaysFailingMarkRetryableStore` always
        // fails) used to just log a warning and leave the effect silently
        // stuck `InFlight` forever — invisible until a future crash-recovery
        // pass that isn't wired anywhere. `retry_or_give_up` must now fall
        // back to `abandon_and_release` instead, the same "stuck and
        // undiscoverable" class of bug F-03 already fixed for
        // `finish_success`'s exhausted path: the effect ends up
        // `TerminalFailed`, not stuck, and its dedup reservation is released
        // like any other genuinely terminal outcome.
        let state = Arc::new(AlwaysFailingMarkRetryableStore::new());
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![AttemptOutcome::RetryableFailure(
            "timeout".into(),
        )]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let (runner, _queue) = runner_with(
            state.clone() as Arc<dyn EffectStateStore>,
            dedup.clone() as Arc<dyn EffectDedupStore>,
            registry,
            RetryPolicy::default(),
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        state.accept(effect.clone()).await.unwrap();

        let scope = DedupScope {
            tenant: effect.tenant.clone(),
            effect_type: effect.description.effect_type.clone(),
            key: effect.description.idempotency_key.clone(),
        };
        let fp =
            EffectFingerprint::compute(&effect.description.payload, &effect.description.destination);

        runner.drain_one(effect).await;

        // Not stuck — never reclaim-eligible either, but genuinely
        // `TerminalFailed`, discoverable via ordinary bookkeeping.
        let due = state.claim_due(Timestamp::now(), 10).await.unwrap();
        assert!(due.is_empty());
        let err = state.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::TerminalFailed,
                ..
            }
        ));

        // Dedup reservation released, like every other genuinely terminal
        // outcome — proven by a fresh reservation succeeding for the same
        // scope.
        assert_eq!(
            dedup.reserve(&scope, EffectId::new(), fp).await.unwrap(),
            DedupOutcome::Fresh
        );
    }

    // --- Fix 4: bounded retry for `mark_in_flight`, plus the periodic
    // reclaim loop ---------------------------------------------------------

    #[tokio::test]
    async fn mark_in_flight_transient_failure_retries_then_succeeds() {
        let store = Arc::new(FlakyMarkInFlightStore::new(BOOKKEEPING_RETRY_ATTEMPTS - 1));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(ScriptedExecutor::new(vec![])))
            .unwrap();
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            registry,
            RetryPolicy::default(),
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::Succeeded,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn mark_in_flight_permanent_failure_leaves_effect_pending_for_the_reclaim_loop() {
        let store = Arc::new(AlwaysFailingMarkInFlightStore::new());
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            registry,
            RetryPolicy::default(),
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        assert_eq!(
            executor.call_count(),
            0,
            "no side effect must run once mark_in_flight is permanently failing"
        );
        // Still Pending and reclaimable — never silently orphaned forever.
        let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].state, EffectState::Pending);
    }

    #[tokio::test]
    async fn reclaim_loop_redelivers_a_pending_effect_once_the_interval_ticks() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let (queue, receiver) = EffectQueue::bounded(4);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            RetryPolicy::default(),
            queue,
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Accepted directly into the store as `Pending` but never sent
        // through the queue — simulates fix 4's target scenario:
        // `mark_in_flight` failed at accept-time (or the process crashed)
        // and nothing else would ever re-feed it.
        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect).await.unwrap();

        let loop_handle = tokio::spawn(runner.clone().run_inner(
            receiver,
            1,
            shutdown_rx,
            StdDuration::from_millis(20),
            SHUTDOWN_DRAIN_DEADLINE,
        ));

        tokio::time::timeout(StdDuration::from_secs(1), async {
            while executor.call_count() == 0 {
                tokio::time::sleep(StdDuration::from_millis(5)).await;
            }
        })
        .await
        .expect("the periodic reclaim loop redelivers the pending effect within timeout");

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(StdDuration::from_secs(1), loop_handle)
            .await
            .expect("run_inner returns after shutdown")
            .expect("task did not panic");
    }

    #[tokio::test]
    async fn reclaim_loop_does_not_redeliver_an_effect_whose_next_at_is_still_in_the_future() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let (queue, receiver) = EffectQueue::bounded(4);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            RetryPolicy::default(),
            queue,
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect).await.unwrap();
        store.mark_in_flight(id).await.unwrap();
        let far_future = Timestamp::from_utc(Utc::now() + chrono::Duration::hours(1));
        store.mark_retryable(id, 1, far_future).await.unwrap();

        let loop_handle = tokio::spawn(runner.clone().run_inner(
            receiver,
            1,
            shutdown_rx,
            StdDuration::from_millis(10),
            SHUTDOWN_DRAIN_DEADLINE,
        ));

        // Give several reclaim ticks a chance to fire.
        tokio::time::sleep(StdDuration::from_millis(80)).await;
        assert_eq!(
            executor.call_count(),
            0,
            "an effect not yet due must not be reclaimed early"
        );

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(StdDuration::from_secs(1), loop_handle)
            .await
            .expect("run_inner returns after shutdown")
            .expect("task did not panic");
    }

    #[tokio::test]
    async fn reclaim_loop_stops_ticking_once_shutdown_signal_fires() {
        let store = Arc::new(InMemoryEffectStore::new());
        let registry = ExecutorRegistry::new();
        let (queue, receiver) = EffectQueue::bounded(4);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            RetryPolicy::default(),
            queue,
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let loop_handle = tokio::spawn(runner.run_inner(
            receiver,
            1,
            shutdown_rx,
            StdDuration::from_millis(10),
            SHUTDOWN_DRAIN_DEADLINE,
        ));

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(StdDuration::from_millis(500), loop_handle)
            .await
            .expect(
                "the reclaim loop shares the main drain loop's shutdown signal and stops promptly",
            )
            .expect("task did not panic");
    }

    // --- F-01: no more JoinSet self-deadlock; shutdown no longer waits on a
    // retry's backoff at all (there is no more backoff-sleep task tracked in
    // the JoinSet — only currently-executing dispatch tasks are) ----------

    #[tokio::test]
    async fn shutdown_completes_promptly_even_while_a_retryable_effect_is_mid_backoff() {
        // Before the AD-6 revision, a retryable failure spawned a
        // sleep-then-`queue.send` task tracked in the same shared `JoinSet`
        // shutdown drains — with a long backoff (as configured below),
        // shutdown had to wait out most of that backoff (or hit the drain
        // deadline) before `run_inner` could return, and that task's own
        // need to self-register in the `JoinSet` was deadlock-prone against
        // a shutdown already waiting on it (F-01).
        //
        // After the revision, `retry_or_give_up` only calls `mark_retryable`
        // and returns — no task is ever spawned for the backoff itself, so
        // there is nothing self-referential in the `JoinSet` and nothing for
        // shutdown to wait out. This test proves shutdown returns almost
        // immediately despite a backoff (3s) that vastly exceeds both the
        // shutdown-drain deadline and this test's own timeout.
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![AttemptOutcome::RetryableFailure(
            "timeout".into(),
        )]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let long_backoff_retry = RetryPolicy {
            max_attempts: 3,
            base_backoff: StdDuration::from_secs(3),
            max_backoff: StdDuration::from_secs(3),
        };
        let (queue, receiver) = EffectQueue::bounded(8);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            long_backoff_retry,
            queue.clone(),
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();
        queue.send(effect).await.unwrap();

        let loop_handle = tokio::spawn(runner.clone().run_inner(
            receiver,
            2,
            shutdown_rx,
            RECLAIM_INTERVAL,
            SHUTDOWN_DRAIN_DEADLINE,
        ));

        tokio::time::timeout(StdDuration::from_secs(1), async {
            while executor.call_count() == 0 {
                tokio::time::sleep(StdDuration::from_millis(5)).await;
            }
        })
        .await
        .expect("first attempt runs");

        // The effect is now `RetryableFailed`, mid a 3s backoff — but no
        // task is tracking that backoff at all.
        let started_shutdown = std::time::Instant::now();
        shutdown_tx.send(true).unwrap();

        tokio::time::timeout(StdDuration::from_millis(500), loop_handle)
            .await
            .expect("run_inner returns promptly, not deadlocked or waiting on the backoff")
            .expect("drain loop task did not panic");

        let elapsed = started_shutdown.elapsed();
        assert!(
            elapsed < StdDuration::from_millis(300),
            "shutdown must return almost immediately — nothing tracks the retry's backoff \
             anymore (elapsed: {elapsed:?})"
        );
    }

    // --- Gap 1 (PR2 residual fix): drain-deadline expiry actually aborts
    // outstanding tasks instead of leaving them as untracked background work
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn shutdown_aborts_a_task_still_running_past_the_drain_deadline_and_leaves_no_leaked_background_task(
    ) {
        // Before this fix, `drain_tasks` just gave up once its deadline
        // elapsed, returning with the still-running dispatch task (and its
        // inner executor attempt) left running, untracked, in the
        // background forever. This proves: (a) the still-running attempt
        // gets aborted, (b) `run_inner` returns with the tracked `JoinSet`
        // fully empty, and (c) the abort is classified as cancelled — not
        // charged as an ordinary retryable failure.
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let started = Arc::new(tokio::sync::Notify::new());
        let executor = Arc::new(HangingExecutor {
            started: started.clone(),
        });
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        // Zero-retry policy: if the abort were (wrongly) classified as an
        // ordinary `RetryableFailure`, `allows_retry(0)` is false and the
        // effect would be immediately `TerminalFailed` ("attempt cap
        // exceeded") — the same proof shape HIGH-2's own test already uses.
        let no_retries = RetryPolicy {
            max_attempts: 0,
            base_backoff: StdDuration::ZERO,
            max_backoff: StdDuration::ZERO,
        };
        let (queue, receiver) = EffectQueue::bounded(4);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            no_retries,
            queue.clone(),
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();
        queue.send(effect).await.unwrap();

        let drain_deadline = StdDuration::from_millis(20);
        let loop_handle = tokio::spawn(runner.clone().run_inner(
            receiver,
            2,
            shutdown_rx,
            RECLAIM_INTERVAL,
            drain_deadline,
        ));

        tokio::time::timeout(StdDuration::from_secs(1), started.notified())
            .await
            .expect("the hanging executor starts running within timeout");

        shutdown_tx.send(true).unwrap();

        tokio::time::timeout(StdDuration::from_secs(1), loop_handle)
            .await
            .expect(
                "run_inner must return once the drain deadline aborts the hanging task, \
                 not block forever",
            )
            .expect("task did not panic");

        assert!(
            runner.tasks.lock().await.is_empty(),
            "the tracked JoinSet must be fully drained — no leaked background task past \
             the deadline"
        );

        let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert_eq!(
            due.len(),
            1,
            "the aborted attempt must be reclaim-eligible (RetryableFailed), not stuck or \
             silently discarded"
        );
        assert_eq!(due[0].state, EffectState::RetryableFailed);
        // F-04 (PR2 round 4): a cancellation no longer bumps `attempt` at
        // all — it stays exactly as it was before the cancellation (0
        // here), provably untouched by the zero-retry attempt cap.
        assert_eq!(
            due[0].attempt, 0,
            "cancellation must not be charged against the zero-retry attempt cap"
        );
    }

    // --- Fix 6: dedup-store error classification (AD-9-style) -------------

    #[tokio::test]
    async fn dedup_reserve_transient_failure_retries_then_succeeds() {
        let state = Arc::new(InMemoryEffectStore::new());
        let dedup = Arc::new(FlakyReserveDedupStore::new(2));
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(ScriptedExecutor::new(vec![])))
            .unwrap();
        let fast_retry = RetryPolicy {
            max_attempts: 5,
            base_backoff: StdDuration::from_millis(1),
            max_backoff: StdDuration::from_millis(1),
        };
        let (runner, _queue) = runner_with(
            state.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            registry,
            fast_retry,
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        state.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        let err = state.mark_in_flight(id).await.unwrap_err();
        assert!(
            matches!(
                err,
                EffectStoreError::InvalidTransition {
                    from: EffectState::Succeeded,
                    ..
                }
            ),
            "the effect should succeed once the transient dedup error clears within the bound"
        );
    }

    #[tokio::test]
    async fn dedup_reserve_permanent_error_is_immediately_terminal_without_retry() {
        let state = Arc::new(InMemoryEffectStore::new());
        let dedup = Arc::new(AlwaysFailingReserveDedupStore);
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let (runner, _queue) = runner_with(
            state.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            registry,
            RetryPolicy::default(),
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        state.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        assert_eq!(
            executor.call_count(),
            0,
            "a permanent dedup-store error must never be retried nor reach the executor"
        );
        let err = state.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::TerminalFailed,
                ..
            }
        ));
    }

    // --- F-02: per-`effect_type` retry policy override -------------------

    #[tokio::test]
    async fn runner_selects_a_different_retry_policy_per_effect_type() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register(
                "fast-fail",
                Arc::new(ScriptedExecutor::new(vec![AttemptOutcome::RetryableFailure(
                    "x".into(),
                )])),
            )
            .unwrap();
        registry
            .register(
                "lenient-fail",
                Arc::new(ScriptedExecutor::new(vec![AttemptOutcome::RetryableFailure(
                    "x".into(),
                )])),
            )
            .unwrap();

        let no_retries = RetryPolicy {
            max_attempts: 0,
            base_backoff: StdDuration::ZERO,
            max_backoff: StdDuration::ZERO,
        };
        let lenient = RetryPolicy {
            max_attempts: 5,
            base_backoff: StdDuration::ZERO,
            max_backoff: StdDuration::ZERO,
        };
        let policies = RetryPolicies::new(lenient).with_override("fast-fail", no_retries);
        let (queue, _receiver) = EffectQueue::bounded(8);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            policies,
            queue,
        ));

        let fast_id = EffectId::new();
        let fast_effect = accepted(fast_id, "fast-fail", "uow-1:0");
        store.accept(fast_effect.clone()).await.unwrap();
        runner.drain_one(fast_effect).await;
        // The `fast-fail` override (0 retries) exhausts on the first
        // failure: `TerminalFailed`, not `RetryableFailed`.
        let err = store.mark_in_flight(fast_id).await.unwrap_err();
        assert!(
            matches!(
                err,
                EffectStoreError::InvalidTransition {
                    from: EffectState::TerminalFailed,
                    ..
                }
            ),
            "the runner must apply fast-fail's own override, not the lenient default"
        );

        let lenient_id = EffectId::new();
        let lenient_effect = accepted(lenient_id, "lenient-fail", "uow-1:0");
        store.accept(lenient_effect.clone()).await.unwrap();
        runner.drain_one(lenient_effect).await;
        // No override registered for `lenient-fail` — falls back to the
        // lenient default (5 retries), so it's merely `RetryableFailed`
        // (reclaim-eligible), not `TerminalFailed`. `mark_in_flight` is a
        // legal transition FROM `RetryableFailed` too, so inspect via
        // `claim_due` instead, which doesn't mutate state.
        let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
        let lenient_due = due
            .iter()
            .find(|e| e.id == lenient_id)
            .expect("an effect_type without an override must still use the shared default policy");
        assert_eq!(lenient_due.state, EffectState::RetryableFailed);
    }

    // --- HIGH-1: DedupOutcome::Duplicate is a benign no-op, not a failure -

    #[tokio::test]
    async fn dedup_duplicate_on_a_fresh_submission_is_marked_succeeded_not_terminal_failed() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            registry,
            RetryPolicy::default(),
        );

        // Pre-reserve the scope, owned by a DIFFERENT effect id, with the
        // SAME fingerprint this fresh effect's own `drain_one` call will
        // compute — simulating a different, already in-progress/completed
        // submission under the same idempotency scope.
        let scope = DedupScope {
            tenant: TenantId::new("tenant-a").unwrap(),
            effect_type: "invoice.created".to_string(),
            key: IdempotencyKey::new("uow-1:0").unwrap(),
        };
        let fp = EffectFingerprint::compute(&[1, 2, 3], "https://example.com");
        store.reserve(&scope, EffectId::new(), fp).await.unwrap();

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        assert_eq!(
            executor.call_count(),
            0,
            "a deduplicated fresh submission must never reach the executor"
        );
        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(
            matches!(
                err,
                EffectStoreError::InvalidTransition {
                    from: EffectState::Succeeded,
                    ..
                }
            ),
            "Duplicate must be a benign already-satisfied outcome (Succeeded), not TerminalFailed"
        );
    }

    // --- HIGH-2: shutdown/abort-triggered cancellation is not charged as a
    // retryable delivery failure -------------------------------------------

    #[tokio::test]
    async fn cancelled_join_result_is_not_charged_as_a_retryable_failure() {
        let handle = tokio::spawn(async { std::future::pending::<AttemptOutcome>().await });
        handle.abort();
        let result = handle.await;
        assert!(result.as_ref().unwrap_err().is_cancelled());

        let outcome = classify_join_result(result);

        assert!(matches!(outcome, ExecutionOutcome::CancelledForShutdown));
    }

    #[tokio::test]
    async fn panicking_join_result_is_still_charged_as_a_retryable_failure() {
        let handle = tokio::spawn(async { panic!("simulated executor panic") });
        let result: Result<AttemptOutcome, _> = handle.await;
        assert!(result.as_ref().unwrap_err().is_panic());

        let outcome = classify_join_result(result);

        assert!(matches!(
            outcome,
            ExecutionOutcome::Outcome(AttemptOutcome::RetryableFailure(_))
        ));
    }

    #[tokio::test]
    async fn shutdown_cancelled_attempt_is_requeued_without_charging_the_retry_attempt_cap() {
        // Wire a zero-retry policy: if the cancellation were (wrongly)
        // classified as a `RetryableFailure`, `allows_retry(0)` would be
        // false and the effect would be immediately `TerminalFailed`
        // ("attempt cap exceeded"). Proving it instead becomes
        // `RetryableFailed` (reclaim-eligible) demonstrates the cap was
        // never even consulted for this path.
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(ScriptedExecutor::new(vec![])))
            .unwrap();
        let no_retries = RetryPolicy {
            max_attempts: 0,
            base_backoff: StdDuration::ZERO,
            max_backoff: StdDuration::ZERO,
        };
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            registry,
            no_retries,
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();
        store.mark_in_flight(id).await.unwrap();

        runner.requeue_without_charging_attempt(effect).await;

        let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert_eq!(due.len(), 1, "must be reclaim-eligible despite a zero-retry policy");
        assert_eq!(due[0].state, EffectState::RetryableFailed);
        // F-04 (PR2 round 4): no longer bumped — dedup reservation is
        // identity-based now, not attempt-gated, so this counter is left
        // exactly as it was before the cancellation, provably free of any
        // retry-cap cost.
        assert_eq!(due[0].attempt, 0);
    }

    // --- HIGH-4: backoff/timestamp arithmetic saturates, never silently
    // degrades to zero on overflow ------------------------------------------

    #[test]
    fn timestamp_after_saturates_instead_of_degrading_to_zero_on_overflow() {
        let before_plus_one_year = Utc::now() + chrono::Duration::days(365);

        let ts = timestamp_after(StdDuration::MAX);

        assert!(
            ts.into_utc() > before_plus_one_year,
            "an unrepresentable duration must saturate to a large fallback, never `now` \
             (which a `zero()` fallback would silently produce, causing a retry storm)"
        );
    }

    // --- PR2 round 4: F-01 through F-04 (this round's new findings) -------

    #[tokio::test]
    async fn crash_recovered_first_attempt_actually_re_executes_not_falsely_marked_succeeded() {
        // F-02 (BLOCKER, silent data loss): before this fix, `reserve` was
        // only ever called when `effect.attempt == 0`. A crash mid the very
        // first attempt (after `mark_in_flight`/dedup `reserve`, before the
        // executor call ever completed) left `recover_in_flight` resetting
        // the effect back to `Pending` with `attempt` still 0. The
        // re-attempt, still `attempt == 0`, called `reserve` again, collided
        // with its OWN still-held `Fresh` reservation, got back a plain
        // `Duplicate`, and the old code treated any `Duplicate` as "already
        // satisfied elsewhere" — calling `mark_succeeded` WITHOUT ever
        // actually re-executing. This proves the recovered re-attempt now
        // genuinely re-executes instead.
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            registry,
            RetryPolicy::default(),
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        // Simulate the first attempt starting to dispatch — `mark_in_flight`
        // and the dedup `reserve` it would make — then the process crashing
        // before the executor call ever completed.
        store.mark_in_flight(id).await.unwrap();
        let scope = DedupScope {
            tenant: effect.tenant.clone(),
            effect_type: effect.description.effect_type.clone(),
            key: effect.description.idempotency_key.clone(),
        };
        let fp = EffectFingerprint::compute(
            &effect.description.payload,
            &effect.description.destination,
        );
        assert_eq!(
            store.reserve(&scope, id, fp).await.unwrap(),
            DedupOutcome::Fresh
        );
        let recovered = store.recover_in_flight(Timestamp::now()).await.unwrap();
        assert_eq!(recovered, 1);

        // Re-attempt: `attempt` is still 0 (`recover_in_flight` never
        // touches it) — the exact condition the old `attempt == 0` gate
        // misclassified.
        runner.drain_one(effect).await;

        assert_eq!(
            executor.call_count(),
            1,
            "the recovered attempt must genuinely re-execute, not bounce off its own \
             still-held reservation"
        );
        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(
            matches!(
                err,
                EffectStoreError::InvalidTransition {
                    from: EffectState::Succeeded,
                    ..
                }
            ),
            "must reach Succeeded only via a real execution, never a false short-circuit"
        );
    }

    #[tokio::test]
    async fn two_shutdown_cancellations_then_a_genuine_failure_still_has_the_full_retry_budget() {
        // F-04 (BLOCKER, retry budget corruption): `requeue_without_
        // charging_attempt` used to bump `effect.attempt` purely to skip
        // re-reserving dedup under the (now-removed) `attempt == 0` gate —
        // but that same counter is what `RetryPolicy::allows_retry` checks
        // against the retry cap, so 2 shutdown cancellations under
        // `max_attempts: 1` used to silently leave zero real retries, not
        // the documented "cancellation can never exhaust the retry budget".
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![
            AttemptOutcome::RetryableFailure("genuine failure 1".into()),
            AttemptOutcome::RetryableFailure("genuine failure 2".into()),
        ]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let one_retry = RetryPolicy {
            max_attempts: 1,
            base_backoff: StdDuration::ZERO,
            max_backoff: StdDuration::ZERO,
        };
        let (runner, _queue) = runner_with(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            registry,
            one_retry,
        );

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        // Two shutdown-triggered cancellations in a row — neither is a real
        // delivery failure, and neither may cost any retry budget.
        store.mark_in_flight(id).await.unwrap();
        runner.requeue_without_charging_attempt(effect.clone()).await;
        let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert_eq!(due[0].attempt, 0, "first cancellation must not bump attempt");

        store.mark_in_flight(id).await.unwrap();
        runner.requeue_without_charging_attempt(effect.clone()).await;
        let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert_eq!(due[0].attempt, 0, "second cancellation must not bump attempt either");

        // Now the full `max_attempts: 1` real budget must still be
        // available: the first genuine failure is retried once, the second
        // genuine failure exhausts the (still-full) cap.
        runner.drain_one(effect).await;
        let due = store.claim_due(Timestamp::now(), 10).await.unwrap();
        assert_eq!(due[0].state, EffectState::RetryableFailed);
        assert_eq!(due[0].attempt, 1, "the first genuine failure consumes the one real retry");

        let redispatched = AcceptedEffect {
            id: due[0].id,
            tenant: due[0].tenant.clone(),
            attempt: due[0].attempt,
            description: due[0].description.clone(),
        };
        runner.drain_one(redispatched).await;

        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(
            matches!(
                err,
                EffectStoreError::InvalidTransition {
                    from: EffectState::TerminalFailed,
                    ..
                }
            ),
            "the second genuine failure must exhaust the full (uncorrupted) one-retry budget"
        );
        assert_eq!(executor.call_count(), 2, "both genuine attempts must actually run");
    }

    #[tokio::test]
    async fn reclaim_loop_does_not_enqueue_the_same_effect_twice_across_multiple_ticks() {
        // F-01 (BLOCKER): `claim_due` doesn't itself transition state.
        // Before this fix, an effect stayed `Pending`/`RetryableFailed`
        // until something later (`drain_one`) called `mark_in_flight` on
        // it, so the SAME effect could be claimed and re-enqueued on every
        // successive reclaim tick while its first queue entry was still
        // waiting to be dequeued. Fixed: `reclaim_due` claims THEN
        // transitions to `InFlight` THEN enqueues — a second/third tick
        // over the same still-`InFlight` effect now sees `mark_in_flight`
        // fail and skips it.
        let store = Arc::new(InMemoryEffectStore::new());
        let registry = ExecutorRegistry::new();
        let (queue, mut receiver) = EffectQueue::bounded(8);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            RetryPolicy::default(),
            queue,
        ));

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect).await.unwrap();

        // Several reclaim ticks fire before anything ever dequeues the
        // effect (simulating backlog/saturated concurrency).
        runner.reclaim_due().await;
        runner.reclaim_due().await;
        runner.reclaim_due().await;

        let first = receiver.recv().await.expect("one entry must be queued");
        assert!(
            matches!(first, QueuedEffect::Reclaimed(e) if e.id == id),
            "the single queued entry must be the reclaimed effect"
        );

        // No second entry ever gets enqueued for this same effect.
        let second = tokio::time::timeout(StdDuration::from_millis(50), receiver.recv()).await;
        assert!(
            second.is_err(),
            "no second queue entry must exist for the same effect across multiple reclaim ticks"
        );
    }

    #[tokio::test]
    async fn shutdown_reaches_drain_deadline_despite_a_hung_backpressure_permit_wait() {
        // F-03 (BLOCKER): before this fix, acquiring a backpressure permit
        // for a newly-received effect was a bare `.await` outside any
        // `select!` watching shutdown. With every concurrency slot held by
        // a hung executor, a second queued effect would block the main
        // loop on `backpressure.acquire()` forever — never reaching
        // `drain_tasks`'s abort-on-deadline logic at all.
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let started = Arc::new(tokio::sync::Notify::new());
        let executor = Arc::new(HangingExecutor {
            started: started.clone(),
        });
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let (queue, receiver) = EffectQueue::bounded(4);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            RetryPolicy::default(),
            queue.clone(),
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Exactly one concurrency permit — the first (hanging) effect
        // consumes it entirely.
        let first_id = EffectId::new();
        let first_effect = accepted(first_id, "invoice.created", "uow-1:0");
        store.accept(first_effect.clone()).await.unwrap();
        queue.send(first_effect).await.unwrap();

        // A second effect, queued behind the first — the pre-fix code would
        // dequeue it and then block forever trying to acquire the
        // already-exhausted permit.
        let second_id = EffectId::new();
        let second_effect = accepted(second_id, "invoice.created", "uow-2:0");
        store.accept(second_effect.clone()).await.unwrap();
        queue.send(second_effect).await.unwrap();

        let drain_deadline = StdDuration::from_millis(30);
        let loop_handle = tokio::spawn(runner.clone().run_inner(
            receiver,
            1, // exactly one concurrency permit
            shutdown_rx,
            RECLAIM_INTERVAL,
            drain_deadline,
        ));

        tokio::time::timeout(StdDuration::from_secs(1), started.notified())
            .await
            .expect("the hanging executor starts running within timeout");

        // Give the loop a moment to dequeue the second effect and get stuck
        // trying to acquire the already-exhausted permit.
        tokio::time::sleep(StdDuration::from_millis(20)).await;

        shutdown_tx.send(true).unwrap();

        tokio::time::timeout(StdDuration::from_secs(1), loop_handle)
            .await
            .expect(
                "run_inner must return once shutdown is signalled, even with a second effect \
                 stuck waiting on a saturated backpressure permit",
            )
            .expect("task did not panic");
    }
}
