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

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{watch, Mutex as AsyncMutex};
use tokio::task::JoinSet;

use crate::read_side::backpressure::Backpressure;

use super::executor::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
use super::policy::RetryPolicy;
use super::queue::{EffectQueue, EffectQueueReceiver};
use super::registry::ExecutorRegistry;
use super::store::{
    AcceptedEffect, DedupOutcome, DedupScope, EffectDedupStore, EffectId, EffectStateStore,
    EffectStoreError, TerminalReason, Timestamp,
};

/// AD-7: bounded number of times a flat, un-backed-off bookkeeping write
/// (`commit_success`/`mark_succeeded`, and now `mark_in_flight`, fix 4) is
/// retried before giving up on this pass.
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

fn timestamp_after(duration: Duration) -> Timestamp {
    let chrono_duration =
        chrono::Duration::from_std(duration).unwrap_or_else(|_| chrono::Duration::zero());
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
    retry: RetryPolicy,
    queue: EffectQueue,
    /// Fix 5 (PR2 review): every per-effect dispatch task and every
    /// backoff-redispatch task is tracked here (instead of a bare
    /// `tokio::spawn` with a discarded handle) so shutdown can actually wait
    /// for them to drain within [`SHUTDOWN_DRAIN_DEADLINE`].
    tasks: AsyncMutex<JoinSet<()>>,
}

impl DeliveryRunner {
    pub(crate) fn new(
        state: Arc<dyn EffectStateStore>,
        dedup: Arc<dyn EffectDedupStore>,
        registry: Arc<ExecutorRegistry>,
        retry: RetryPolicy,
        queue: EffectQueue,
    ) -> Self {
        Self {
            state,
            dedup,
            registry,
            retry,
            queue,
            tasks: AsyncMutex::new(JoinSet::new()),
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
        let _ = tokio::time::timeout(deadline, async {
            while tasks.join_next().await.is_some() {}
        })
        .await;
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

        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                _ = reclaim_tick.tick() => {
                    self.reclaim_due().await;
                }
                maybe_effect = receiver.recv() => {
                    match maybe_effect {
                        Some(effect) => {
                            let permit = backpressure
                                .acquire()
                                .await
                                .expect("backpressure semaphore not closed");
                            let this = self.clone();
                            self.spawn_tracked(async move {
                                this.drain_one(effect).await;
                                drop(permit);
                            })
                            .await;
                        }
                        None => break,
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
            let effect = AcceptedEffect {
                id: stored.id,
                tenant: stored.tenant,
                attempt: stored.attempt,
                description: stored.description,
            };
            let _ = self.queue.send(effect).await;
        }
    }

    /// One full attempt of one accepted effect (design.md §5 data flow).
    pub(crate) async fn drain_one(&self, effect: AcceptedEffect) {
        let scope = DedupScope {
            tenant: effect.tenant.clone(),
            effect_type: effect.description.effect_type.clone(),
            key: effect.description.idempotency_key.clone(),
        };
        let fingerprint = fingerprint(&effect.description.payload, &effect.description.destination);

        // `EffectStateStore::mark_terminal` only accepts `InFlight` or
        // `RetryableFailed` as its `from` state (store.rs, already shipped in
        // PR1) — so every short-circuit below that needs to record a
        // terminal outcome must go through `mark_in_flight` first. That
        // means `mark_in_flight` runs ahead of the dedup `reserve` call here,
        // one step earlier than design.md §5's informal sketch, to stay
        // within the real, already-shipped state machine.
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

        match self.reserve_with_retry(&scope, fingerprint).await {
            Ok(DedupOutcome::Duplicate) => {
                self.abandon(effect.id, TerminalReason::Other("deduplicated".into()))
                    .await;
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
            Ok(DedupOutcome::Fresh) => {}
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

        let outcome = execute_catching_panics(executor, effect.description.clone(), ctx).await;

        match outcome {
            AttemptOutcome::Success => self.finish_success(effect, scope).await,
            AttemptOutcome::RetryableFailure(_) => self.retry_or_give_up(effect, scope).await,
            AttemptOutcome::TerminalFailure(reason) => {
                self.abandon_and_release(effect.id, TerminalReason::Other(reason), scope)
                    .await;
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
    /// existing delivery `RetryPolicy` (ponytail: one flat retry budget
    /// shared with delivery retries, not a separate counter — revisit if
    /// that proves too coarse); everything else is immediately terminal,
    /// same as before this fix.
    async fn reserve_with_retry(
        &self,
        scope: &DedupScope,
        fingerprint: u64,
    ) -> Result<DedupOutcome, ()> {
        let mut attempt: u32 = 0;
        loop {
            match self.dedup.reserve(scope, fingerprint).await {
                Ok(outcome) => return Ok(outcome),
                Err(EffectStoreError::TemporarilyUnavailable(_)) => {
                    if !self.retry.allows_retry(attempt) {
                        return Err(());
                    }
                    let backoff = self.retry.backoff(attempt + 1);
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
    /// dedup scope shares.
    async fn abandon(&self, id: EffectId, reason: TerminalReason) {
        let _ = self.state.mark_terminal(id, reason).await;
    }

    /// Fix 8: `mark_terminal` then release the dedup reservation — the
    /// shape every genuinely-terminal-after-reserving call site shares.
    async fn abandon_and_release(&self, id: EffectId, reason: TerminalReason, scope: DedupScope) {
        let _ = self.state.mark_terminal(id, reason).await;
        let _ = self.dedup.release(&scope).await;
    }

    /// AD-7: on a successful attempt, bounded-retry the idempotent
    /// bookkeeping write; if it still fails, genuinely re-dispatch the
    /// effect (fix 2, PR2 review) instead of only leaving it `InFlight` with
    /// nothing further scheduled. A cooperating destination's mandatory
    /// idempotency-key handling (design.md §6.5) absorbs the resulting
    /// duplicate delivery — the accepted AD-7 tradeoff.
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
            "post-success bookkeeping exhausted retries; redispatching per AD-7"
        );
        let next_attempt = effect.attempt + 1;
        let backoff = self.retry.backoff(next_attempt.max(1));
        self.schedule_redispatch(effect, next_attempt, backoff, scope)
            .await;
    }

    async fn retry_or_give_up(&self, effect: AcceptedEffect, scope: DedupScope) {
        if !self.retry.allows_retry(effect.attempt) {
            self.abandon_and_release(
                effect.id,
                TerminalReason::Other("attempt cap exceeded".into()),
                scope,
            )
            .await;
            return;
        }

        let dispatched_attempt = effect.attempt + 1;
        let backoff = self.retry.backoff(dispatched_attempt);
        let next_at = timestamp_after(backoff);

        // Fix 1 (PR2 review): the bookkeeping write recording the retry
        // count is for durability/observability only (the same principle
        // AD-7 already applies to `finish_success`'s bookkeeping write) — it
        // is not a precondition for the in-process retry, which operates
        // purely on the in-memory `effect` value. Note the failure and keep
        // going rather than abandoning the effect, which used to leave it
        // permanently `InFlight` with its dedup reservation leaked forever.
        if self
            .state
            .mark_retryable(effect.id, dispatched_attempt, next_at)
            .await
            .is_err()
        {
            tracing::warn!(
                effect_id = %effect.id,
                "mark_retryable bookkeeping write failed; redispatching anyway"
            );
        }

        // Fix 3: dedup stays reserved for the whole backoff sleep, released
        // only immediately before the effect re-enters the queue inside
        // `schedule_redispatch` — closing the duplicate-delivery window an
        // earlier release used to leave open.
        self.schedule_redispatch(effect, dispatched_attempt, backoff, scope)
            .await;
    }

    /// Shared by fixes 1/2/3: schedules the backoff-sleep-then-redispatch
    /// task (tracked, fix 5) that both `retry_or_give_up` and
    /// `finish_success`'s exhausted-bookkeeping path use. Releases the dedup
    /// reservation right before the effect re-enters the queue, never
    /// earlier (fix 3) — AD-6: retryable effects re-enter via
    /// `tokio::time::sleep` then re-`send`, not via `claim_due` (that stays
    /// reserved for crash recovery / the fix-4 reclaim loop, AD-8).
    async fn schedule_redispatch(
        &self,
        mut effect: AcceptedEffect,
        next_attempt: u32,
        backoff: Duration,
        scope: DedupScope,
    ) {
        effect.attempt = next_attempt;
        let queue = self.queue.clone();
        let dedup = self.dedup.clone();
        self.spawn_tracked(async move {
            tokio::time::sleep(backoff).await;
            let _ = dedup.release(&scope).await;
            let _ = queue.send(effect).await;
        })
        .await;
    }
}

async fn execute_catching_panics(
    executor: Arc<dyn ExternalEffectExecutor>,
    description: Arc<ego_domain::ExternalEffectDescription>,
    ctx: EffectContext,
) -> AttemptOutcome {
    match tokio::spawn(async move { executor.execute(&description, &ctx).await }).await {
        Ok(outcome) => outcome,
        Err(join_err) if join_err.is_panic() => {
            AttemptOutcome::RetryableFailure("executor panicked".to_string())
        }
        Err(_) => AttemptOutcome::RetryableFailure("executor task cancelled".to_string()),
    }
}

/// A cheap, deterministic dedup fingerprint over payload + destination —
/// good enough to detect "same scope, different effect" (spec: dedup
/// Conflict), not a cryptographic hash.
fn fingerprint(payload: &[u8], destination: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    payload.hash(&mut hasher);
    destination.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::policy::{DeliveryConfig, RunnerMode};
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
            fingerprint: u64,
        ) -> Result<DedupOutcome, EffectStoreError> {
            if self.failures_left.load(Ordering::SeqCst) > 0 {
                self.failures_left.fetch_sub(1, Ordering::SeqCst);
                return Err(EffectStoreError::TemporarilyUnavailable(
                    "dedup store flaky".into(),
                ));
            }
            self.inner.reserve(scope, fingerprint).await
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
            _fingerprint: u64,
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
        retry: RetryPolicy,
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
    async fn retryable_failure_re_enqueues_after_backoff_and_eventually_succeeds() {
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
        let (queue, mut receiver) = EffectQueue::bounded(8);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            fast_retry,
            queue,
        ));

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        // The retry re-enters through the queue after backoff — drive it
        // through the same pipeline again.
        let redispatched = tokio::time::timeout(StdDuration::from_secs(1), receiver.recv())
            .await
            .expect("redispatch arrives within timeout")
            .expect("queue not closed");
        assert_eq!(redispatched.attempt, 1);
        runner.drain_one(redispatched).await;

        assert_eq!(executor.call_count(), 2, "executor ran once per attempt");
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
        store.reserve(&scope, 999).await.unwrap();

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
        let fast_retry = RetryPolicy {
            max_attempts: 3,
            base_backoff: StdDuration::from_millis(5),
            max_backoff: StdDuration::from_millis(5),
        };
        let (queue, mut receiver) = EffectQueue::bounded(8);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            fast_retry,
            queue,
        ));

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;
        // Panic is caught and classified as a retryable failure, not a
        // crash — the effect re-enters the queue for a second attempt.
        let redispatched = tokio::time::timeout(StdDuration::from_secs(1), receiver.recv())
            .await
            .expect("redispatch arrives within timeout")
            .expect("queue not closed");
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
    async fn bookkeeping_write_exhausted_redispatches_instead_of_only_staying_in_flight() {
        let store = Arc::new(FlakyBookkeepingStore::new(u32::MAX));
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        registry
            .register("invoice.created", Arc::new(ScriptedExecutor::new(vec![])))
            .unwrap();
        let fast_retry = RetryPolicy {
            max_attempts: 3,
            base_backoff: StdDuration::from_millis(5),
            max_backoff: StdDuration::from_millis(5),
        };
        let (queue, mut receiver) = EffectQueue::bounded(8);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            dedup as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            fast_retry,
            queue,
        ));

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();

        runner.drain_one(effect).await;

        // Never marked Succeeded or TerminalFailed by the bookkeeping path
        // itself — still InFlight.
        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::InFlight,
                ..
            }
        ));

        // Fix 2: AD-7's "re-dispatched" promise is now real — a genuine
        // redispatch attempt reaches the queue instead of the effect just
        // sitting there with nothing further scheduled.
        let redispatched = tokio::time::timeout(StdDuration::from_secs(1), receiver.recv())
            .await
            .expect("a redispatch is actually scheduled, not just 'stays in-flight'")
            .expect("queue not closed");
        assert_eq!(redispatched.id, id);
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

    // --- Fix 1 & 3: mark_retryable bookkeeping failure no longer abandons
    // the effect or leaks/early-releases the dedup reservation -----------

    #[tokio::test]
    async fn mark_retryable_failure_still_redispatches_and_dedup_stays_reserved_until_redispatch()
    {
        let state = Arc::new(AlwaysFailingMarkRetryableStore::new());
        let dedup = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![AttemptOutcome::RetryableFailure(
            "timeout".into(),
        )]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        let fast_retry = RetryPolicy {
            max_attempts: 3,
            base_backoff: StdDuration::from_millis(30),
            max_backoff: StdDuration::from_millis(30),
        };
        let (queue, mut receiver) = EffectQueue::bounded(8);
        let runner = Arc::new(DeliveryRunner::new(
            state.clone() as Arc<dyn EffectStateStore>,
            dedup.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            fast_retry,
            queue,
        ));

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        state.accept(effect.clone()).await.unwrap();

        let scope = DedupScope {
            tenant: effect.tenant.clone(),
            effect_type: effect.description.effect_type.clone(),
            key: effect.description.idempotency_key.clone(),
        };
        let fp = fingerprint(&effect.description.payload, &effect.description.destination);

        runner.drain_one(effect).await;

        // Fix 1: `mark_retryable` always fails here, but the effect must
        // still be scheduled for redispatch, not abandoned permanently.
        // Fix 3: immediately after `drain_one` returns, the backoff sleep
        // hasn't elapsed yet, so the dedup reservation must still be held.
        let still_reserved = dedup.reserve(&scope, fp).await.unwrap();
        assert_eq!(
            still_reserved,
            DedupOutcome::Duplicate,
            "dedup must stay reserved through the backoff sleep, not be released early"
        );

        let redispatched = tokio::time::timeout(StdDuration::from_secs(1), receiver.recv())
            .await
            .expect("redispatch arrives despite mark_retryable failing")
            .expect("queue not closed");
        assert_eq!(redispatched.attempt, 1);

        // Only now, right before the effect actually re-enters the
        // pipeline, is the reservation released.
        let now_fresh = dedup.reserve(&scope, fp).await.unwrap();
        assert_eq!(
            now_fresh,
            DedupOutcome::Fresh,
            "dedup must be released right before redispatch, not earlier and not never"
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

    // --- Fix 5: shutdown waits for outstanding tracked tasks --------------

    #[tokio::test]
    async fn shutdown_waits_up_to_the_drain_deadline_for_an_outstanding_redispatch_task() {
        let store = Arc::new(InMemoryEffectStore::new());
        let mut registry = ExecutorRegistry::new();
        let executor = Arc::new(ScriptedExecutor::new(vec![AttemptOutcome::RetryableFailure(
            "timeout".into(),
        )]));
        registry
            .register("invoice.created", executor.clone())
            .unwrap();
        // The backoff is deliberately far longer than the drain deadline
        // below, so shutdown is guaranteed to arrive while the redispatch
        // task is still sleeping regardless of scheduling jitter under a
        // busy/parallel test run — this test proves the *bound*, not a
        // race against exact timing.
        let retry = RetryPolicy {
            max_attempts: 3,
            base_backoff: StdDuration::from_secs(3),
            max_backoff: StdDuration::from_secs(3),
        };
        let (queue, receiver) = EffectQueue::bounded(8);
        let runner = Arc::new(DeliveryRunner::new(
            store.clone() as Arc<dyn EffectStateStore>,
            store.clone() as Arc<dyn EffectDedupStore>,
            Arc::new(registry),
            retry,
            queue.clone(),
        ));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let id = EffectId::new();
        let effect = accepted(id, "invoice.created", "uow-1:0");
        store.accept(effect.clone()).await.unwrap();
        queue.send(effect).await.unwrap();

        let drain_deadline = StdDuration::from_millis(200);
        let loop_handle = tokio::spawn(runner.clone().run_inner(
            receiver,
            2,
            shutdown_rx,
            RECLAIM_INTERVAL,
            drain_deadline,
        ));

        tokio::time::timeout(StdDuration::from_secs(1), async {
            while executor.call_count() == 0 {
                tokio::time::sleep(StdDuration::from_millis(5)).await;
            }
        })
        .await
        .expect("first attempt runs");

        let started_shutdown = std::time::Instant::now();
        shutdown_tx.send(true).unwrap();

        tokio::time::timeout(StdDuration::from_secs(2), loop_handle)
            .await
            .expect("run_inner returns within the bounded shutdown-drain deadline")
            .expect("drain loop task did not panic");

        let elapsed = started_shutdown.elapsed();
        assert!(
            elapsed >= StdDuration::from_millis(150),
            "run_inner must actually wait for the outstanding redispatch task, not return \
             instantly on shutdown (elapsed: {elapsed:?})"
        );
        assert!(
            elapsed < StdDuration::from_secs(2),
            "run_inner must give up around the drain deadline rather than waiting out the \
             full 3s backoff of the still-outstanding redispatch task (elapsed: {elapsed:?})"
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
}
