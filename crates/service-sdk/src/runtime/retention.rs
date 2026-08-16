//! Retention of completed reservations: the policy, and the worker that applies it.
//!
//! # Off unless asked for
//!
//! [`RetentionPolicy`] is optional and there is no default. A runtime built
//! without one never starts a worker and never deletes anything, because the
//! alternative is that upgrading the SDK silently begins removing a service's
//! data on a schedule nobody chose.
//!
//! # Validated in the constructor, like the reservation config
//!
//! All three values must be greater than zero, and that is checked where the
//! policy is built rather than where it is used. A zero interval is a busy loop
//! against the database; a zero batch removes nothing while looking configured; a
//! zero retention purges an answer the instant it completes, which is the one
//! thing the reservation exists to prevent. None of those is a state a caller
//! should be able to assemble.
//!
//! # The worker is runtime-owned
//!
//! It starts explicitly — the same shape as the effects subsystem, so nothing
//! begins deleting rows as a side effect of `build()` — and its shutdown is a
//! registered teardown hook, running in the same ordered, panic-isolated
//! mechanism every other hook uses.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ego_domain::operation::{OldestCompleted, OperationReservationStore};
use ego_domain::{Clock, Observability, SpanAttributes, SpanOutcome, TraceContext, Tracer};

use super::runtime_builder::OpenSpan;
use futures::FutureExt;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Why a [`RetentionPolicy`] could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RetentionPolicyError {
    /// The retention window was zero.
    ///
    /// Every completed reservation would be eligible the instant it completed, so
    /// a retry arriving a moment later would re-execute instead of replaying —
    /// the exact duplicate the reservation exists to prevent.
    #[error("the retention window must be greater than zero")]
    ZeroRetention,
    /// The tick interval was zero.
    ///
    /// The worker would purge in a tight loop, which is a denial of service
    /// against the database rather than a retention schedule.
    #[error("the retention tick interval must be greater than zero")]
    ZeroInterval,
    /// The batch size was zero.
    ///
    /// `purge_completed_before` removes at most `batch` rows, so a zero batch
    /// removes nothing on every tick — a worker that runs forever and retains
    /// everything, while the configuration says retention is enabled.
    #[error("the retention batch size must be greater than zero")]
    ZeroBatch,
}

/// How long a completed reservation is kept, how often to look, and how much to
/// remove per look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    retention: Duration,
    interval: Duration,
    batch_size: usize,
}

impl RetentionPolicy {
    /// Builds a policy, or refuses to.
    ///
    /// Validating here means there is one place a degenerate value is rejected,
    /// and no way for a later caller to hold an unvalidated policy — the same
    /// reasoning [`super::ReservationConfig`] is built on.
    pub fn new(
        retention: Duration,
        interval: Duration,
        batch_size: usize,
    ) -> Result<Self, RetentionPolicyError> {
        if retention.is_zero() {
            return Err(RetentionPolicyError::ZeroRetention);
        }
        if interval.is_zero() {
            return Err(RetentionPolicyError::ZeroInterval);
        }
        if batch_size == 0 {
            return Err(RetentionPolicyError::ZeroBatch);
        }
        Ok(Self {
            retention,
            interval,
            batch_size,
        })
    }

    /// How long a completed reservation is kept.
    pub fn retention(&self) -> Duration {
        self.retention
    }

    /// How often the worker looks for eligible reservations.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// The most rows one tick removes.
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }
}

/// A started retention worker, and the handle that stops it.
pub(crate) struct RetentionWorker {
    cancel: Arc<Notify>,
    task: JoinHandle<()>,
}

impl RetentionWorker {
    /// Spawns the loop.
    ///
    /// It purges **before** waiting rather than after, so a started worker does
    /// its first pass promptly. That is also what makes it observable in a test
    /// without depending on a tick elapsing in wall time.
    pub(crate) fn start(
        policy: RetentionPolicy,
        store: Arc<dyn OperationReservationStore>,
        clock: Arc<dyn Clock>,
        tracer: Option<Arc<dyn Tracer>>,
        observability: Option<Arc<dyn Observability>>,
    ) -> Self {
        let cancel = Arc::new(Notify::new());
        let loop_cancel = cancel.clone();

        let task = tokio::spawn(async move {
            // `from_std` cannot fail for a `Duration` this small, but the policy's
            // own validation is what guarantees it is non-zero — the conversion is
            // not the place that decides.
            let window = chrono::Duration::from_std(policy.retention())
                .unwrap_or_else(|_| chrono::Duration::seconds(i64::MAX / 1_000));

            loop {
                let cutoff = clock.now() - window;

                // AD-10's `idempotency.purge_batch`, and the one span in that table
                // whose parent is **root**: a background tick has no request boundary
                // to descend from, and attaching it to one would bill a single caller
                // for work done on behalf of everybody.
                //
                // A fresh root per tick rather than one for the worker's lifetime.
                // Each batch is its own unit of work, and a single trace spanning
                // every tick of a long-lived worker would never close.
                //
                // No `operation_key_hash`: a purge is about many reservations, so
                // there is no one key it could name.
                //
                // `OpenSpan` is the same guard the reservation span uses, not a second
                // one. The reason applies identically here and is if anything sharper:
                // the `.await` below is a cancellation point, and shutdown *does*
                // cancel it — `RetentionWorker::stop` aborts the task when a purge
                // overruns its deadline, which is exactly a drop mid-`await`. Without
                // the guard every such shutdown would leak an entry in the adapter's
                // bounded table.
                let span = tracer.as_ref().map(|tracer| {
                    let ctx = TraceContext::root();
                    tracer.start_span(&ctx, "idempotency.purge_batch", SpanAttributes::new());
                    OpenSpan::new(tracer.clone(), ctx.span_id())
                });

                // Measured around the store call alone, so it is the time the purge
                // took and not the time the tick took. `Instant`, not the injected
                // clock: that clock is a *logical* one a test positions freely, so a
                // duration read from it would be zero under a frozen clock and
                // nonsense under a jumped one. This is the one place in this file that
                // legitimately wants wall time.
                let started = Instant::now();

                // A failed purge is not fatal: the next tick tries again.
                //
                // A purge failure now closes `idempotency.purge_batch` as `Error`
                // **when tracing is configured**; without a tracer, retention
                // behaviour is unchanged — the `Result` produces no observable signal
                // and the loop carries on exactly as before. That gap is deliberate
                // at this point rather than overlooked: a failure counter is a metric,
                // and metrics are what B7.10/B7.11 still owe.
                let purged = store
                    .purge_completed_before(cutoff, policy.batch_size())
                    .await;

                // AD-10's two purge metrics.
                //
                // The duration goes out whatever the outcome: a batch that failed still
                // consumed time, and omitting it would make failure look instantaneous
                // on a latency panel — the opposite of what an operator needs when the
                // database is the thing that is slow.
                //
                // The row count goes out only on success. A failed purge removed
                // nothing, so counting zero would claim work that did not happen, and
                // would be indistinguishable from a healthy tick with nothing eligible.
                // Those two states call for different actions, so they must not share a
                // number.
                if let Some(obs) = observability.as_ref() {
                    obs.histogram(
                        "idempotency.purge.batch_duration",
                        started.elapsed().as_secs_f64(),
                        &[],
                    );
                    if let Ok(rows) = &purged {
                        obs.counter("idempotency.purge.rows", *rows as f64, &[]);
                    }

                    // AD-10's `idempotency.purge.oldest_completed_age`, a gauge.
                    //
                    // Queried **after** the purge, on purpose: the question it
                    // answers is how old the backlog that *remains* is. Asked
                    // first, it would describe rows this batch was about to
                    // delete, and would keep reporting a stale age forever on a
                    // deployment whose retention is working perfectly.
                    //
                    // The age is computed here rather than by the store: the
                    // store knows `completed_at`, this worker owns the clock, and
                    // the same injected clock that positioned `cutoff` above must
                    // position this — otherwise a test could place a row's age
                    // freely and then read it back against wall time.
                    //
                    // A sample goes out for exactly one of the four answers.
                    // `Empty` and `Unsupported` both stay silent, and are still
                    // kept apart by the port: an absent gauge is honest for both,
                    // where a `0.0` would claim the oldest completed reservation
                    // has just been written. An error is left to the existing
                    // handling below and adds no sample of its own.
                    match store.oldest_completed().await {
                        Ok(OldestCompleted::At(completed_at)) => {
                            let age = clock.now() - completed_at;
                            obs.gauge(
                                "idempotency.purge.oldest_completed_age",
                                age.num_milliseconds() as f64 / 1_000.0,
                                &[],
                            );
                        }
                        Ok(OldestCompleted::Empty) | Ok(OldestCompleted::Unsupported) => {}
                        Err(_) => {}
                    }
                }

                if let Some(span) = span {
                    span.close(match &purged {
                        Ok(_) => SpanOutcome::Ok,
                        // A fixed, redaction-safe message: a store error can quote a
                        // DSN, and `SpanOutcome::Error` requires a safe one.
                        Err(_) => SpanOutcome::Error {
                            status_message: "the reservation store could not purge".to_string(),
                        },
                    });
                }

                tokio::select! {
                    _ = loop_cancel.notified() => break,
                    _ = tokio::time::sleep(policy.interval()) => {}
                }
            }
        });

        Self { cancel, task }
    }

    /// Cancels the loop and waits, bounded, for it to leave.
    ///
    /// Returns what happened so a caller can report it. Nothing here calls
    /// `abandon`, `complete` or `renew`: a worker that only ever purges completed
    /// rows has no lease of its own, and shutting it down must not touch a
    /// reservation another owner is still holding.
    ///
    /// On timeout the task is **aborted and then awaited**, not dropped. Dropping a
    /// `JoinHandle` detaches the task in Tokio rather than cancelling it, so an
    /// earlier version returned `TimedOut` while the worker carried on purging — the
    /// shutdown reported a failure and the thing it was shutting down survived it.
    pub(crate) async fn stop(self, deadline: Duration) -> RetentionShutdown {
        self.cancel.notify_waiters();
        // `notify_waiters` reaches a task already parked in `select!`; a task
        // between iterations is not waiting yet, so `notify_one` is also issued to
        // leave a permit for it. Without both, cancelling could be missed and the
        // bounded wait below would always spend its full deadline.
        self.cancel.notify_one();

        let mut task = self.task;
        match tokio::time::timeout(deadline, &mut task).await {
            Ok(Ok(())) => RetentionShutdown::Stopped,
            Ok(Err(joined)) if joined.is_panic() => RetentionShutdown::Panicked,
            Ok(Err(_)) => RetentionShutdown::Stopped,
            Err(_) => {
                // It did not leave on its own, so it is cancelled — and awaited, so
                // this returns only once the task is genuinely gone. Awaiting an
                // aborted handle yields `Err(is_cancelled)`, which is the expected
                // outcome here rather than a further failure.
                task.abort();
                let _ = task.await;
                RetentionShutdown::TimedOut
            }
        }
    }
}

/// What a retention shutdown observed.
///
/// Returned rather than logged here, so the caller owns reporting. The
/// instrumentation this feeds is B7.10/B7.11's subject; the point of naming the
/// outcomes now is that a panic or a timeout is a fact the teardown path can
/// surface instead of one it discards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetentionShutdown {
    /// The loop acknowledged cancellation and exited.
    Stopped,
    /// The loop panicked. Isolated rather than propagated: a worker's failure
    /// must not stop the remaining teardown hooks.
    Panicked,
    /// The loop did not exit within the deadline, so it was **aborted and then
    /// awaited** — this outcome is reported only once the task is gone. Not
    /// abandoned: dropping the handle would detach the task and leave it purging
    /// past a shutdown that had already reported failure.
    TimedOut,
}

/// Runs `hook` with panics isolated, matching how provider teardown is guarded.
///
/// A panic inside a teardown hook must not prevent the hooks after it from
/// running, and must not be silently lost either — the boolean is the caller's
/// signal that one happened.
pub(crate) async fn isolate_panics<F>(hook: F) -> bool
where
    F: Future<Output = ()>,
{
    AssertUnwindSafe(hook).catch_unwind().await.is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_needs_all_three_values_above_zero() {
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
        assert!(RetentionPolicy::new(ok, ok, 1).is_ok());
    }
}
