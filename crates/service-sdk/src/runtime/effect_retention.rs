//! Retention of settled external effects (PROD-002 G12): the policy, and the
//! worker that applies it.
//!
//! Sibling of [`super::retention`], which does the identical job for
//! [`ego_domain::operation::OperationReservationStore`]. The two are kept
//! separate on purpose — [`super::retention::RetentionWorker`] purges
//! completed *operation reservations*, this one purges settled *external
//! effects* — different domain concept, different runtime-owned capability
//! ([`ego_runtime::effects::RetentionMaintenance`] vs
//! [`ego_domain::operation::OperationReservationStore::purge_completed_before`]),
//! and a runtime may configure either, both, or neither independently. This
//! is also why the entry point on [`super::builder::Runtime`] is
//! [`super::builder::Runtime::start_retention_effects`] rather than reusing
//! the existing `start_retention` name (PROD-012's reservation retention) —
//! two schedules that can coexist need two names, not one overloaded one.
//!
//! # Off unless asked for
//!
//! [`EffectRetentionPolicy`] is optional and there is no default, for the
//! exact reason [`super::retention::RetentionPolicy`] has none: a runtime
//! built without one never starts a worker and never deletes an effect,
//! because the alternative is an SDK upgrade that silently begins removing a
//! service's data on a schedule nobody chose.
//!
//! # Lifecycle shape reused verbatim
//!
//! `Notify`-based cooperative cancellation, `abort()`+`await` with a
//! shutdown deadline, panic isolation via [`super::retention::isolate_panics`]
//! (not reimplemented — the exact same free function) — this worker is
//! structurally the same loop as [`super::retention::RetentionWorker`], just
//! purging through [`ego_runtime::effects::RetentionMaintenance`] instead of
//! [`ego_domain::operation::OperationReservationStore`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use ego_domain::{Clock, Observability, SpanAttributes, SpanOutcome, TraceContext, Tracer};
use ego_runtime::effects::store::Timestamp;
use ego_runtime::effects::RetentionMaintenance;

use super::runtime_builder::OpenSpan;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Why an [`EffectRetentionPolicy`] could not be built.
///
/// Mirrors [`super::retention::RetentionPolicyError`] field-for-field — same
/// three degenerate configurations are refused for the same reasons, just
/// against the effects subsystem instead of reservations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EffectRetentionPolicyError {
    /// The retention window was zero.
    #[error("the effect retention window must be greater than zero")]
    ZeroRetention,
    /// The tick interval was zero.
    #[error("the effect retention tick interval must be greater than zero")]
    ZeroInterval,
    /// The batch size was zero.
    #[error("the effect retention batch size must be greater than zero")]
    ZeroBatch,
}

/// How long a settled effect is kept, how often to look, and how much to
/// remove per look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectRetentionPolicy {
    retention: Duration,
    interval: Duration,
    batch_size: usize,
}

impl EffectRetentionPolicy {
    /// Builds a policy, or refuses to. See
    /// [`super::retention::RetentionPolicy::new`] for the identical
    /// reasoning: validating here means there is exactly one place a
    /// degenerate value is rejected.
    pub fn new(
        retention: Duration,
        interval: Duration,
        batch_size: usize,
    ) -> Result<Self, EffectRetentionPolicyError> {
        if retention.is_zero() {
            return Err(EffectRetentionPolicyError::ZeroRetention);
        }
        if interval.is_zero() {
            return Err(EffectRetentionPolicyError::ZeroInterval);
        }
        if batch_size == 0 {
            return Err(EffectRetentionPolicyError::ZeroBatch);
        }
        Ok(Self {
            retention,
            interval,
            batch_size,
        })
    }

    /// How long a settled effect is kept.
    pub fn retention(&self) -> Duration {
        self.retention
    }

    /// How often the worker looks for eligible effects.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// The most rows one tick removes.
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }
}

/// A started effect-retention worker, and the handle that stops it.
pub(crate) struct EffectRetentionWorker {
    cancel: Arc<Notify>,
    task: JoinHandle<()>,
}

impl EffectRetentionWorker {
    /// Spawns the loop. Purges **before** waiting, same as
    /// [`super::retention::RetentionWorker::start`] and for the same reason:
    /// a started worker does its first pass promptly, which is also what
    /// makes it observable in a test without depending on wall-time elapsing.
    pub(crate) fn start(
        policy: EffectRetentionPolicy,
        store: Arc<dyn RetentionMaintenance>,
        clock: Arc<dyn Clock>,
        tracer: Option<Arc<dyn Tracer>>,
        observability: Option<Arc<dyn Observability>>,
    ) -> Self {
        let cancel = Arc::new(Notify::new());
        let loop_cancel = cancel.clone();

        let task = tokio::spawn(async move {
            let window = chrono::Duration::from_std(policy.retention())
                .unwrap_or_else(|_| chrono::Duration::seconds(i64::MAX / 1_000));

            loop {
                let cutoff = Timestamp::from_utc(clock.now() - window);

                // A fresh root span per tick, same reasoning as the
                // reservation-retention worker: a background tick has no
                // request boundary to descend from, and a single trace
                // spanning every tick of a long-lived worker would never
                // close.
                let span = tracer.as_ref().map(|tracer| {
                    let ctx = TraceContext::root();
                    tracer.start_span(&ctx, "effect.purge_batch", SpanAttributes::new());
                    OpenSpan::new(tracer.clone(), ctx.span_id())
                });

                let started = Instant::now();

                // A failed purge is not fatal: the next tick tries again. The
                // provider's own `run_retention` already logs
                // `cleanup_deleted` on a successful, non-empty purge (see
                // `crates/effect-store/src/{postgres,stoolap}/mod.rs`) — this
                // worker does not log that again, only the metrics/span below.
                let purged = store
                    .purge_before(cutoff, policy.batch_size())
                    .await;

                // G13's fixed metric names (architecture-reconciliation.md):
                // `effect.cleanup.rows` (counter) and
                // `effect.cleanup.batch_duration` (histogram) — G13 itself
                // (wiring claim/recovery metrics too) is not yet implemented
                // in this worktree, but its names for cleanup are already
                // decided, so this worker uses them rather than inventing a
                // parallel ad hoc scheme.
                if let Some(obs) = observability.as_ref() {
                    obs.histogram(
                        "effect.cleanup.batch_duration",
                        started.elapsed().as_secs_f64(),
                        &[],
                    );
                    if let Ok(rows) = &purged {
                        obs.counter("effect.cleanup.rows", *rows as f64, &[]);
                    }

                    // G13's `effect.cleanup.oldest_terminal_age` gauge —
                    // mirrors `RetentionWorker`'s `idempotency.purge.
                    // oldest_completed_age` line for line (`retention.rs`):
                    // queried *after* the purge (so it describes the backlog
                    // that remains, not rows this batch was about to
                    // delete), computed from this worker's own injected
                    // clock (not the store's `settled_at`, and not
                    // `Instant::now()` — a test positions the logical clock
                    // freely), and clamped at zero for the same reason: two
                    // replicas' clocks can disagree, and a settled_at
                    // slightly ahead of this reader must read as "nothing
                    // older than now", not as negative backlog.
                    //
                    // `RetentionMaintenance::oldest_terminal` returns
                    // `Result<Option<Timestamp>, EffectStoreError>` rather
                    // than `OperationReservationStore::oldest_completed`'s
                    // three-variant `OldestCompleted` — its default already
                    // folds "empty" and "unsupported" into the same `None`,
                    // so both stay silent here exactly as `Empty`/
                    // `Unsupported` do for reservations, and an `Err` adds no
                    // sample of its own, matching that same precedent.
                    if let Ok(Some(settled_at)) = store.oldest_terminal().await {
                        let age = clock.now() - settled_at.into_utc();
                        obs.gauge(
                            "effect.cleanup.oldest_terminal_age",
                            age.num_milliseconds().max(0) as f64 / 1_000.0,
                            &[],
                        );
                    }
                }

                if let Some(span) = span {
                    span.close(match &purged {
                        Ok(_) => SpanOutcome::Ok,
                        Err(_) => SpanOutcome::Error {
                            status_message: "the effect store could not purge".to_string(),
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

    /// Cancels the loop and waits, bounded, for it to leave. Identical
    /// contract to [`super::retention::RetentionWorker::stop`]: on timeout
    /// the task is aborted and then awaited, never merely dropped.
    pub(crate) async fn stop(self, deadline: Duration) -> EffectRetentionShutdown {
        self.cancel.notify_waiters();
        self.cancel.notify_one();

        let mut task = self.task;
        match tokio::time::timeout(deadline, &mut task).await {
            Ok(Ok(())) => EffectRetentionShutdown::Stopped,
            Ok(Err(joined)) if joined.is_panic() => EffectRetentionShutdown::Panicked,
            Ok(Err(_)) => EffectRetentionShutdown::Stopped,
            Err(_) => {
                task.abort();
                let _ = task.await;
                EffectRetentionShutdown::TimedOut
            }
        }
    }
}

/// What an effect-retention shutdown observed. Mirrors
/// [`super::retention::RetentionShutdown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectRetentionShutdown {
    /// The loop acknowledged cancellation and exited.
    Stopped,
    /// The loop panicked. Isolated rather than propagated.
    Panicked,
    /// The loop did not exit within the deadline and was aborted, then
    /// awaited.
    TimedOut,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_needs_all_three_values_above_zero() {
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
        assert!(EffectRetentionPolicy::new(ok, ok, 1).is_ok());
    }
}
