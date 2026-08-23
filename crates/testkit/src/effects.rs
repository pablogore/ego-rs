//! Recording `ExternalEffectExecutor` test double (CORE-019 Phase 12.1).
//!
//! Records every delivery attempt (`effect_type`, `destination`, `payload`,
//! attempt number) so a test can assert on delivery/retry/dedup behavior
//! without standing up a real external system.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use ego_domain::ExternalEffectDescription;
use ego_runtime::effects::{
    AcceptedEffect, AttemptOutcome, DedupOutcome, DedupScope, EffectContext, EffectDedupStore,
    EffectFingerprint, EffectId, EffectStateStore, EffectStoreError, ExternalEffectExecutor,
    InMemoryEffectStore, StoredEffect, TerminalReason, Timestamp,
};

/// One recorded delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedAttempt {
    /// The `effect_type` of the effect this attempt was for.
    pub effect_type: String,
    /// The effect's destination, passed through unexamined.
    pub destination: String,
    /// The effect's payload, passed through unexamined.
    pub payload: Vec<u8>,
    /// The 1-based attempt number for this dispatch.
    pub attempt: u32,
}

/// Records every attempt an [`ExternalEffectExecutor`] receives — same-contract
/// principle: a real implementation of the real production trait, not a
/// look-alike.
///
/// Configure a scripted outcome sequence via [`RecordingExecutor::with_outcomes`]
/// to exercise retry (e.g. a `RetryableFailure` followed by a `Success`); the
/// final scripted outcome repeats once the sequence is exhausted, so a real
/// delivery runner attempting more times than were explicitly scripted never
/// panics.
pub struct RecordingExecutor {
    attempts: Mutex<Vec<RecordedAttempt>>,
    outcomes: Vec<AttemptOutcome>,
}

impl RecordingExecutor {
    /// An executor that always succeeds, on any attempt.
    pub fn always_succeeds() -> Self {
        Self::with_outcomes(vec![AttemptOutcome::Success])
    }

    /// An executor that replays `outcomes` in order (indexed by the 1-based
    /// attempt number), repeating the final entry once exhausted.
    ///
    /// # Panics
    /// Panics if `outcomes` is empty — there would be nothing to return.
    pub fn with_outcomes(outcomes: Vec<AttemptOutcome>) -> Self {
        assert!(
            !outcomes.is_empty(),
            "RecordingExecutor::with_outcomes needs at least one scripted outcome"
        );
        Self {
            attempts: Mutex::new(Vec::new()),
            outcomes,
        }
    }

    /// Every attempt recorded so far, in delivery order.
    pub fn attempts(&self) -> Vec<RecordedAttempt> {
        self.attempts.lock().unwrap().clone()
    }
}

#[async_trait]
impl ExternalEffectExecutor for RecordingExecutor {
    async fn execute(
        &self,
        effect: &ExternalEffectDescription,
        ctx: &EffectContext,
    ) -> AttemptOutcome {
        self.attempts.lock().unwrap().push(RecordedAttempt {
            effect_type: effect.effect_type.clone(),
            destination: effect.destination.clone(),
            payload: effect.payload.clone(),
            attempt: ctx.attempt,
        });

        let idx = (ctx.attempt as usize).saturating_sub(1);
        self.outcomes
            .get(idx)
            .or_else(|| self.outcomes.last())
            .cloned()
            .expect("checked non-empty at construction")
    }
}

/// One [`EffectStateStore`]/[`EffectDedupStore`] operation
/// [`FaultInjectingEffectStore`] can script a fault against (PROD-002 AD-12,
/// design.md §3.5).
///
/// Trimmed to exactly the calls `crates/runtime/src/effects/runner.rs`'s
/// `DeliveryRunner` (the sole production caller this double exists to
/// exercise) actually makes. `EffectStateStore::accept` (called only by
/// `RuntimeEffectAcceptor`, never by `DeliveryRunner`) and
/// `EffectStateStore::recover_in_flight` (no production caller anywhere,
/// design.md's G13 note) are deliberately excluded — scripting a fault
/// against an operation nothing under test ever calls would prove nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoreOp {
    /// [`EffectStateStore::claim_due`].
    ClaimDue,
    /// [`EffectStateStore::mark_in_flight`].
    MarkInFlight,
    /// [`EffectStateStore::mark_succeeded`].
    MarkSucceeded,
    /// [`EffectStateStore::mark_retryable`].
    MarkRetryable,
    /// [`EffectStateStore::mark_terminal`].
    MarkTerminal,
    /// [`EffectDedupStore::reserve`].
    Reserve,
    /// [`EffectDedupStore::commit_success`].
    CommitSuccess,
    /// [`EffectDedupStore::release`].
    Release,
}

/// Deterministic, scripted fault vocabulary (PROD-002 AD-12, design.md §3.5).
/// Never touched directly by a test — see [`FaultInjectingEffectStore::fail_next`]
/// and [`FaultInjectingEffectStore::crash_after`].
#[derive(Default)]
struct FaultPlan {
    /// Scripted errors per op, consumed in FIFO order; once an op's queue is
    /// empty, calls to it fall through to the real store again.
    fail_calls: HashMap<StoreOp, VecDeque<EffectStoreError>>,
    /// A one-shot ambiguity window: the next call to this op lets its write
    /// land against the real store, then hides the `Ok` from the caller.
    crash_after: Option<StoreOp>,
}

/// The transient error a [`FaultInjectingEffectStore::crash_after`]-armed
/// call returns once its real write has already landed — modeling a response
/// lost after a real backend committed, exactly the shape
/// [`EffectStoreError::TemporarilyUnavailable`] already documents.
fn ambiguous_write_error() -> EffectStoreError {
    EffectStoreError::TemporarilyUnavailable(
        "crash_after: the write landed but the response was lost before the caller observed it"
            .to_string(),
    )
}

/// A real [`EffectStateStore`] + [`EffectDedupStore`] implementation wrapping
/// a live [`InMemoryEffectStore`] (PROD-002 AD-12, design.md §3.5) — lets a
/// test exercise `DeliveryRunner`'s retry/recovery/ambiguity behavior without
/// a real durable backend, Docker, sleeps, or random fault injection.
///
/// Every normal-path call (transitions, dedup classification, retry
/// bookkeeping, claim filtering) delegates straight through to a real
/// `InMemoryEffectStore` — same-contract principle, this type never
/// reimplements the effect lifecycle state machine. It only adds a
/// deterministic, scripted [`StoreOp`] fault plan on top. With an empty fault
/// plan it behaves exactly like a plain `InMemoryEffectStore`.
#[derive(Default)]
pub struct FaultInjectingEffectStore {
    inner: RwLock<Arc<InMemoryEffectStore>>,
    plan: Mutex<FaultPlan>,
}

impl FaultInjectingEffectStore {
    /// A fresh double wrapping a fresh, empty `InMemoryEffectStore`, with an
    /// empty fault plan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Scripts `error` to be returned by the next call to `op`, without
    /// touching the real backing store — models a transient (or permanent)
    /// failure exactly as a real durable backend would surface one. Scripted
    /// errors for the same `op` are consumed in FIFO order; once `op`'s queue
    /// is empty, calls to it fall through to the real store again.
    pub fn fail_next(&self, op: StoreOp, error: EffectStoreError) {
        self.plan
            .lock()
            .unwrap()
            .fail_calls
            .entry(op)
            .or_default()
            .push_back(error);
    }

    /// Arms a one-shot ambiguity window on `op` (design.md §3.5): the NEXT
    /// call to `op` lets its write land against the real backing store, then
    /// returns a transient error instead of `Ok` — the write succeeded, but
    /// the caller never learns it did, exactly like a response lost after a
    /// real backend commits. Consumed on that first matching call; later
    /// calls to `op` are unaffected.
    pub fn crash_after(&self, op: StoreOp) {
        self.plan.lock().unwrap().crash_after = Some(op);
    }

    /// Destroys all volatile backing state, modeling a full process crash
    /// (design.md §3.5) — mirrors what really happens to a plain
    /// `InMemoryEffectStore` on a real crash. Nothing accepted or reserved
    /// before this call remains observable through this double afterward;
    /// there is nothing left to recover. This is NOT the operation
    /// recovery-logic tests should use — see [`Self::simulate_runner_crash`].
    pub fn simulate_process_crash(&self) {
        *self.inner.write().unwrap() = Arc::new(InMemoryEffectStore::new());
    }

    /// Models a runner (not process) crash (design.md §3.5): the backing
    /// store keeps every record exactly as it is, so a subsequent
    /// `recover_in_flight`/`claim_due` still sees whatever was `InFlight`
    /// before the crash — this is the operation recovery-logic tests use.
    ///
    /// ponytail: a documented no-op. `InMemoryEffectStore` (and this double,
    /// which only ever wraps it) carries no owner/epoch/lease concept at all
    /// (AD-6) — an `InFlight` record already *is* what an abandoned claim
    /// looks like at this level, so there is no separate "ownership" flag to
    /// clear here. This method exists to name the AD-12-required semantic
    /// distinction from [`Self::simulate_process_crash`] at call sites and to
    /// let a test state its intent explicitly; the actual recovery assertion
    /// runs through the real `recover_in_flight`/`claim_due` machinery,
    /// completely unmodified.
    pub fn simulate_runner_crash(&self) {}

    fn current_inner(&self) -> Arc<InMemoryEffectStore> {
        self.inner.read().unwrap().clone()
    }

    fn take_fail(&self, op: StoreOp) -> Option<EffectStoreError> {
        self.plan
            .lock()
            .unwrap()
            .fail_calls
            .get_mut(&op)
            .and_then(VecDeque::pop_front)
    }

    fn take_crash_after(&self, op: StoreOp) -> bool {
        let mut plan = self.plan.lock().unwrap();
        if plan.crash_after == Some(op) {
            plan.crash_after = None;
            true
        } else {
            false
        }
    }

    /// The one place a fault-injectable call goes through: a scripted
    /// `fail_next` error short-circuits before `real` is ever awaited; an
    /// armed `crash_after` lets `real` land, then hides its `Ok` behind
    /// [`ambiguous_write_error`]; otherwise `real`'s own outcome passes
    /// through untouched.
    async fn with_fault<T: Send>(
        &self,
        op: StoreOp,
        real: impl std::future::Future<Output = Result<T, EffectStoreError>> + Send,
    ) -> Result<T, EffectStoreError> {
        if let Some(err) = self.take_fail(op) {
            return Err(err);
        }
        if self.take_crash_after(op) {
            let _ = real.await;
            return Err(ambiguous_write_error());
        }
        real.await
    }
}

#[async_trait]
impl EffectStateStore for FaultInjectingEffectStore {
    async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
        // No `StoreOp::Accept` exists (see the enum's doc) — `accept` is
        // never called by `DeliveryRunner`, so this always delegates
        // straight through.
        self.current_inner().accept(effect).await
    }

    async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
        let inner = self.current_inner();
        self.with_fault(StoreOp::MarkInFlight, inner.mark_in_flight(id))
            .await
    }

    async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
        let inner = self.current_inner();
        self.with_fault(StoreOp::MarkSucceeded, inner.mark_succeeded(id))
            .await
    }

    async fn mark_retryable(
        &self,
        id: EffectId,
        attempt: u32,
        next_at: Timestamp,
    ) -> Result<(), EffectStoreError> {
        let inner = self.current_inner();
        self.with_fault(
            StoreOp::MarkRetryable,
            inner.mark_retryable(id, attempt, next_at),
        )
        .await
    }

    async fn mark_terminal(
        &self,
        id: EffectId,
        reason: TerminalReason,
    ) -> Result<(), EffectStoreError> {
        let inner = self.current_inner();
        self.with_fault(StoreOp::MarkTerminal, inner.mark_terminal(id, reason))
            .await
    }

    async fn claim_due(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<StoredEffect>, EffectStoreError> {
        let inner = self.current_inner();
        self.with_fault(StoreOp::ClaimDue, inner.claim_due(now, limit))
            .await
    }

    async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
        // No `StoreOp::RecoverInFlight` exists (see the enum's doc) —
        // `DeliveryRunner` never calls this method at all, so this always
        // delegates straight through.
        self.current_inner().recover_in_flight(now).await
    }
}

#[async_trait]
impl EffectDedupStore for FaultInjectingEffectStore {
    async fn reserve(
        &self,
        scope: &DedupScope,
        effect_id: EffectId,
        fingerprint: EffectFingerprint,
    ) -> Result<DedupOutcome, EffectStoreError> {
        let inner = self.current_inner();
        self.with_fault(
            StoreOp::Reserve,
            inner.reserve(scope, effect_id, fingerprint),
        )
        .await
    }

    async fn commit_success(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
        let inner = self.current_inner();
        self.with_fault(StoreOp::CommitSuccess, inner.commit_success(scope))
            .await
    }

    async fn release(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
        let inner = self.current_inner();
        self.with_fault(StoreOp::Release, inner.release(scope))
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
    use ego_runtime::effects::{AttemptOutcome, EffectContext, EffectId, ExternalEffectExecutor};

    use super::RecordingExecutor;

    fn description(effect_type: &str, key: &str) -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new(key).unwrap(),
            effect_type: effect_type.to_string(),
            payload: vec![1, 2, 3],
            destination: "https://example.com/probe".to_string(),
        }
    }

    fn ctx(attempt: u32) -> EffectContext {
        EffectContext {
            effect_id: EffectId::new(),
            tenant: TenantId::new("tenant-a").unwrap(),
            attempt,
            idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
        }
    }

    #[tokio::test]
    async fn records_effect_type_destination_payload_and_attempt() {
        let executor = RecordingExecutor::always_succeeds();

        let outcome = executor
            .execute(&description("invoice.created", "uow-1:0"), &ctx(1))
            .await;

        assert_eq!(outcome, AttemptOutcome::Success);
        let attempts = executor.attempts();
        assert_eq!(attempts.len(), 1, "exactly one attempt must be recorded");
        assert_eq!(attempts[0].effect_type, "invoice.created");
        assert_eq!(attempts[0].destination, "https://example.com/probe");
        assert_eq!(attempts[0].payload, vec![1, 2, 3]);
        assert_eq!(attempts[0].attempt, 1);
    }

    #[tokio::test]
    async fn scripted_outcomes_support_retry_then_success_and_repeat_the_last_entry() {
        let executor = Arc::new(RecordingExecutor::with_outcomes(vec![
            AttemptOutcome::RetryableFailure("timeout".to_string()),
            AttemptOutcome::Success,
        ]));

        let first = executor
            .execute(&description("invoice.created", "uow-2:0"), &ctx(1))
            .await;
        let second = executor
            .execute(&description("invoice.created", "uow-2:0"), &ctx(2))
            .await;
        // A 3rd attempt beyond the scripted sequence repeats the last entry
        // rather than panicking — real delivery runners may attempt more
        // times than were explicitly scripted.
        let third = executor
            .execute(&description("invoice.created", "uow-2:0"), &ctx(3))
            .await;

        assert_eq!(
            first,
            AttemptOutcome::RetryableFailure("timeout".to_string())
        );
        assert_eq!(second, AttemptOutcome::Success);
        assert_eq!(third, AttemptOutcome::Success);
        assert_eq!(
            executor.attempts().len(),
            3,
            "every attempt is recorded, including the repeated one"
        );
    }
}

#[cfg(test)]
mod fault_injecting_effect_store_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
    use ego_runtime::effects::{
        AcceptedEffect, DedupOutcome, DedupScope, EffectFingerprint, EffectId, EffectState,
        EffectStateStore, EffectStoreError, ExecutorRegistry, RetryPolicy, RunnerMode,
        RuntimeEffectAcceptor,
    };
    use ego_runtime::effects::{DeliveryConfig, EffectDedupStore};
    use persistent_entity::effect_acceptor::EffectAcceptor;

    use super::{FaultInjectingEffectStore, StoreOp};
    use crate::RecordingExecutor;

    fn accepted_effect(id: EffectId, key: &str) -> AcceptedEffect {
        AcceptedEffect {
            id,
            tenant: TenantId::new("tenant-a").unwrap(),
            attempt: 0,
            description: Arc::new(description(key)),
        }
    }

    fn description(key: &str) -> ExternalEffectDescription {
        ExternalEffectDescription {
            idempotency_key: IdempotencyKey::new(key).unwrap(),
            effect_type: "probe.effect".to_string(),
            payload: vec![1, 2, 3],
            destination: "https://example.com/probe".to_string(),
        }
    }

    fn dedup_scope(key: &str) -> DedupScope {
        DedupScope {
            tenant: TenantId::new("tenant-a").unwrap(),
            effect_type: "probe.effect".to_string(),
            key: IdempotencyKey::new(key).unwrap(),
        }
    }

    /// Zero backoff everywhere so no test ever spends real wall-clock time
    /// waiting out `RetryPolicy::backoff` — `mark_in_flight`/`mark_retryable`'s
    /// own bounded retry (`BOOKKEEPING_RETRY_ATTEMPTS`) never sleeps at all,
    /// only `reserve`'s policy-driven retry would, absent this.
    fn no_backoff_retry_policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 3,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        }
    }

    /// Builds a real `RuntimeEffectAcceptor` in `Inline` mode over `store` —
    /// `accept()` itself drives `DeliveryRunner::drain_one` synchronously, so
    /// a test never needs to wait out the real periodic reclaim tick.
    fn inline_acceptor(
        store: Arc<FaultInjectingEffectStore>,
        executor: Arc<RecordingExecutor>,
    ) -> RuntimeEffectAcceptor {
        let mut registry = ExecutorRegistry::new();
        registry.register("probe.effect", executor).unwrap();
        RuntimeEffectAcceptor::new(
            store.clone(),
            store,
            Arc::new(registry),
            DeliveryConfig {
                queue_capacity: 1,
                retry: no_backoff_retry_policy(),
                runner_mode: RunnerMode::Inline,
            },
        )
    }

    // ---- 6.1: scripted transient operation failures ----

    #[tokio::test]
    async fn scripted_transient_mark_in_flight_failures_are_absorbed_by_the_real_delivery_runner_retry_path(
    ) {
        let store = Arc::new(FaultInjectingEffectStore::new());
        store.fail_next(
            StoreOp::MarkInFlight,
            EffectStoreError::TemporarilyUnavailable("blip 1".into()),
        );
        store.fail_next(
            StoreOp::MarkInFlight,
            EffectStoreError::TemporarilyUnavailable("blip 2".into()),
        );

        let executor = Arc::new(RecordingExecutor::always_succeeds());
        let acceptor = inline_acceptor(store, executor.clone());

        let tenant = TenantId::new("tenant-a").unwrap();
        acceptor
            .accept(&tenant, vec![description("probe:0")])
            .await
            .expect(
                "mark_in_flight's own bounded retry (BOOKKEEPING_RETRY_ATTEMPTS=3) must absorb \
                 exactly two scripted transient failures before the real delivery runner \
                 proceeds to dispatch",
            );

        assert_eq!(
            executor.attempts().len(),
            1,
            "the effect must reach the executor exactly once, after the retried mark_in_flight \
             succeeded"
        );
    }

    #[tokio::test]
    async fn scripted_failures_are_consumed_in_fifo_order_then_fall_through_to_the_real_store() {
        let store = FaultInjectingEffectStore::new();
        let id = EffectId::new();
        store.accept(accepted_effect(id, "fifo")).await.unwrap();

        store.fail_next(
            StoreOp::MarkInFlight,
            EffectStoreError::TemporarilyUnavailable("first".into()),
        );
        store.fail_next(
            StoreOp::MarkInFlight,
            EffectStoreError::Backend("second".into()),
        );

        assert!(matches!(
            store.mark_in_flight(id).await,
            Err(EffectStoreError::TemporarilyUnavailable(msg)) if msg == "first"
        ));
        assert!(matches!(
            store.mark_in_flight(id).await,
            Err(EffectStoreError::Backend(msg)) if msg == "second"
        ));
        // The scripted queue is now empty — the real store takes over.
        store
            .mark_in_flight(id)
            .await
            .expect("once the fault queue is drained, calls must fall through to the real store");
        // Proof the fall-through call genuinely transitioned state: a second
        // real call now correctly rejects with InvalidTransition, which could
        // only happen if the prior call really flipped the state to InFlight.
        assert!(matches!(
            store.mark_in_flight(id).await,
            Err(EffectStoreError::InvalidTransition { .. })
        ));
    }

    // ---- 6.2: simulate_process_crash ----

    #[tokio::test]
    async fn simulate_process_crash_destroys_all_volatile_state() {
        let store = FaultInjectingEffectStore::new();
        let id = EffectId::new();
        store
            .accept(accepted_effect(id, "lost-on-process-crash"))
            .await
            .unwrap();
        store.mark_in_flight(id).await.unwrap();

        let scope = dedup_scope("lost-on-process-crash");
        let fp = EffectFingerprint::compute(b"payload", "dest");
        store.reserve(&scope, id, fp).await.unwrap();

        store.simulate_process_crash();

        let now = ego_runtime::effects::Timestamp::now();
        assert!(
            store.claim_due(now, 10).await.unwrap().is_empty(),
            "nothing must be claimable after a process crash"
        );
        assert_eq!(
            store.recover_in_flight(now).await.unwrap(),
            0,
            "nothing must be recoverable after a process crash — there is nothing left to \
             recover"
        );
        assert!(matches!(
            store.mark_succeeded(id).await,
            Err(EffectStoreError::NotFound(found)) if found == id
        ));
        assert_eq!(
            store.reserve(&scope, id, fp).await.unwrap(),
            DedupOutcome::Fresh,
            "the dedup reservation must also be gone — a fresh reserve() must see Fresh, not a \
             survivor"
        );
    }

    // ---- 6.3: simulate_runner_crash ----

    #[tokio::test]
    async fn simulate_runner_crash_preserves_backing_state_so_recovery_can_reclaim_it() {
        let store = FaultInjectingEffectStore::new();
        let id = EffectId::new();
        store
            .accept(accepted_effect(id, "abandoned-in-flight"))
            .await
            .unwrap();
        store.mark_in_flight(id).await.unwrap();

        store.simulate_runner_crash();

        let now = ego_runtime::effects::Timestamp::now();
        // Not simulate_process_crash: the record must still exist, merely
        // abandoned — recover_in_flight must actually find it.
        let recovered = store.recover_in_flight(now).await.unwrap();
        assert_eq!(
            recovered, 1,
            "the abandoned in-flight effect must still be there for recover_in_flight to find"
        );

        let claimed = store.claim_due(now, 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, id);
        store
            .mark_in_flight(id)
            .await
            .expect("redispatch after recovery must succeed");
    }

    // ---- 6.4: deterministic claim race ----

    #[tokio::test]
    async fn claim_due_and_mark_in_flight_together_guarantee_exclusivity_and_post_abandonment_redispatch(
    ) {
        let store = FaultInjectingEffectStore::new();
        let id = EffectId::new();
        store.accept(accepted_effect(id, "race")).await.unwrap();

        let now = ego_runtime::effects::Timestamp::now();

        // Claimant A claims and transitions first.
        let claimed_a = store.claim_due(now, 10).await.unwrap();
        assert_eq!(claimed_a.len(), 1);
        store.mark_in_flight(id).await.unwrap();

        // Claimant B's claim_due no longer sees the effect at all — it is no
        // longer Pending/RetryableFailed.
        let claimed_b = store.claim_due(now, 10).await.unwrap();
        assert!(
            claimed_b.is_empty(),
            "an already-InFlight effect must not be claimable a second time"
        );

        // Even a direct, out-of-band second mark_in_flight on the same id
        // (bypassing claim_due entirely) is rejected — the guarded transition
        // itself is what makes claim exclusivity real.
        let err = store.mark_in_flight(id).await.unwrap_err();
        assert!(matches!(
            err,
            EffectStoreError::InvalidTransition {
                from: EffectState::InFlight,
                to: EffectState::InFlight,
                ..
            }
        ));

        // Once the runner holding claimant A's claim is abandoned and
        // recovery runs, the effect becomes claimable again — redispatch by
        // a new claimant is possible.
        store.simulate_runner_crash();
        let recovered = store.recover_in_flight(now).await.unwrap();
        assert_eq!(recovered, 1);

        let claimed_again = store.claim_due(now, 10).await.unwrap();
        assert_eq!(
            claimed_again.len(),
            1,
            "after abandonment + recovery, the effect must be claimable again"
        );
        store.mark_in_flight(id).await.unwrap();
    }

    // ---- 6.5: crash_after ambiguity window ----

    #[tokio::test]
    async fn crash_after_commit_success_hides_a_landed_write_but_the_real_retry_reconciles_it() {
        let store = Arc::new(FaultInjectingEffectStore::new());
        store.crash_after(StoreOp::CommitSuccess);

        let executor = Arc::new(RecordingExecutor::always_succeeds());
        let acceptor = inline_acceptor(store.clone(), executor.clone());

        let tenant = TenantId::new("tenant-a").unwrap();
        acceptor
            .accept(&tenant, vec![description("probe:0")])
            .await
            .expect(
                "finish_success's own bounded retry loop must reconcile the ambiguous \
                 commit_success response",
            );

        assert_eq!(
            executor.attempts().len(),
            1,
            "the effect must be executed exactly once — the ambiguity is a bookkeeping-response \
             loss, not a redispatch"
        );

        // The write genuinely landed on the first (hidden) attempt: a fresh
        // reserve() under the same scope+fingerprint now sees OtherSucceeded
        // for a different probe id, proving no corruption and no
        // double-execution.
        let scope = dedup_scope("probe:0");
        let fp = EffectFingerprint::compute(&[1, 2, 3], "https://example.com/probe");
        let outcome = store.reserve(&scope, EffectId::new(), fp).await.unwrap();
        assert_eq!(outcome, DedupOutcome::OtherSucceeded);
    }
}
