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

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{watch, Semaphore};

use super::executor::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
use super::policy::RetryPolicy;
use super::queue::{EffectQueue, EffectQueueReceiver};
use super::registry::ExecutorRegistry;
use super::store::{
    AcceptedEffect, DedupOutcome, DedupScope, EffectDedupStore, EffectStateStore, TerminalReason,
    Timestamp,
};

/// AD-7: bounded number of times the post-success bookkeeping write
/// (`commit_success` + `mark_succeeded`) is retried before giving up on this
/// pass and leaving the effect `InFlight` rather than losing it.
const BOOKKEEPING_RETRY_ATTEMPTS: u32 = 3;

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
        }
    }

    /// Spawned drain loop (`Deferred` profile, AD-6): receives from the
    /// queue, bounds concurrency with a `Semaphore`, and stops on shutdown
    /// signal via `watch`.
    pub(crate) async fn run(
        self: Arc<Self>,
        mut receiver: EffectQueueReceiver,
        concurrency: usize,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let semaphore = Arc::new(Semaphore::new(concurrency.max(1)));
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                maybe_effect = receiver.recv() => {
                    match maybe_effect {
                        Some(effect) => {
                            let permit = semaphore.clone().acquire_owned().await.expect("semaphore not closed");
                            let this = self.clone();
                            tokio::spawn(async move {
                                this.drain_one(effect).await;
                                drop(permit);
                            });
                        }
                        None => break,
                    }
                }
            }
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
        if self.state.mark_in_flight(effect.id).await.is_err() {
            return;
        }

        match self.dedup.reserve(&scope, fingerprint).await {
            Ok(DedupOutcome::Duplicate) => {
                let _ = self
                    .state
                    .mark_terminal(effect.id, TerminalReason::Other("deduplicated".into()))
                    .await;
                return;
            }
            Ok(DedupOutcome::Conflict) => {
                let _ = self
                    .state
                    .mark_terminal(
                        effect.id,
                        TerminalReason::InvalidEffect("dedup scope conflict".into()),
                    )
                    .await;
                return;
            }
            Ok(DedupOutcome::Fresh) => {}
            Err(_) => {
                // ponytail: a dedup-store error at reserve time is treated as
                // terminal for this attempt rather than retried indefinitely —
                // a future durable dedup store's transient/permanent split
                // could inform a bounded retry here if this proves too
                // aggressive in practice.
                let _ = self
                    .state
                    .mark_terminal(
                        effect.id,
                        TerminalReason::Other("dedup store unavailable".into()),
                    )
                    .await;
                return;
            }
        }

        let Some(executor) = self.registry.get(&effect.description.effect_type) else {
            let _ = self
                .state
                .mark_terminal(effect.id, TerminalReason::ExecutorMissing)
                .await;
            let _ = self.dedup.release(&scope).await;
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
            AttemptOutcome::Success => self.finish_success(effect, &scope).await,
            AttemptOutcome::RetryableFailure(_) => self.retry_or_give_up(effect, &scope).await,
            AttemptOutcome::TerminalFailure(reason) => {
                let _ = self
                    .state
                    .mark_terminal(effect.id, TerminalReason::Other(reason))
                    .await;
                let _ = self.dedup.release(&scope).await;
            }
        }
    }

    /// AD-7: on a successful attempt, bounded-retry the idempotent
    /// bookkeeping write; if it still fails, leave the effect `InFlight`
    /// rather than losing it. The existing crash-recovery path
    /// (`EffectStateStore::recover_in_flight` + `claim_due`, Phase 1) is what
    /// eventually reclaims and re-executes it — safe because
    /// idempotency-key propagation is mandatory (design.md §6.5).
    async fn finish_success(&self, effect: AcceptedEffect, scope: &DedupScope) {
        for _ in 0..BOOKKEEPING_RETRY_ATTEMPTS {
            if self.dedup.commit_success(scope).await.is_ok()
                && self.state.mark_succeeded(effect.id).await.is_ok()
            {
                return;
            }
        }
        // Bookkeeping still failing after bounded retries: stays `InFlight`,
        // never lost.
    }

    async fn retry_or_give_up(&self, effect: AcceptedEffect, scope: &DedupScope) {
        if !self.retry.allows_retry(effect.attempt) {
            let _ = self
                .state
                .mark_terminal(
                    effect.id,
                    TerminalReason::Other("attempt cap exceeded".into()),
                )
                .await;
            let _ = self.dedup.release(scope).await;
            return;
        }

        let dispatched_attempt = effect.attempt + 1;
        let backoff = self.retry.backoff(dispatched_attempt);
        let next_at = timestamp_after(backoff);

        if self
            .state
            .mark_retryable(effect.id, dispatched_attempt, next_at)
            .await
            .is_err()
        {
            return;
        }
        let _ = self.dedup.release(scope).await;

        // AD-6: retryable effects re-enter via `tokio::time::sleep` then
        // re-`send`, not via `claim_due` (that stays reserved for crash
        // recovery, AD-8).
        let queue = self.queue.clone();
        let mut redispatch = effect;
        redispatch.attempt = dispatched_attempt;
        tokio::spawn(async move {
            tokio::time::sleep(backoff).await;
            let _ = queue.send(redispatch).await;
        });
    }
}

async fn execute_catching_panics(
    executor: Arc<dyn ExternalEffectExecutor>,
    description: ego_domain::ExternalEffectDescription,
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
    use crate::effects::store::{EffectId, EffectState, EffectStoreError, InMemoryEffectStore};
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
            description: description(effect_type, key),
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
        ) -> Result<Vec<super::super::store::StoredEffect>, EffectStoreError> {
            self.inner.claim_due(now, limit).await
        }
        async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
            self.inner.recover_in_flight(now).await
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
    async fn bookkeeping_write_exhausted_leaves_effect_in_flight_not_lost() {
        let store = Arc::new(FlakyBookkeepingStore::new(u32::MAX));
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

        // Never marked Succeeded or TerminalFailed — still InFlight.
        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::InFlight,
                ..
            }
        ));
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
}
