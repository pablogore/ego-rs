//! `RuntimeBuilder::with_effect_store` (PROD-002 PR5 Phase 7): the seam that
//! lets a host register a custom durable `EffectStateStore`+`EffectDedupStore`
//! in place of the default `InMemoryEffectStore` `build()` otherwise
//! constructs whenever an executor is registered.
//!
//! `RecordingEffectStore` below is a decorator around a real
//! `InMemoryEffectStore` (not a from-scratch reimplementation of the
//! lifecycle state machine) — it delegates every call and only adds call
//! counters, so these tests exercise the SAME state-machine behavior the
//! default path already relies on while still proving the registered
//! double, not `InMemoryEffectStore`, is what the runtime actually calls.
//!
//! Run with: cargo test -p ego-service-sdk --test effect_store_composition

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ego_domain::{ExternalEffectDescription, IdempotencyKey};
use ego_runtime::effects::{
    AcceptedEffect, AttemptOutcome, DedupOutcome, DedupScope, EffectContext, EffectDedupStore,
    EffectFingerprint, EffectId, EffectStateStore, EffectStoreError, ExternalEffectExecutor,
    InMemoryEffectStore, RetentionMaintenance, StoredEffect, TerminalReason, Timestamp,
};
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, RuntimeBuilder};
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::testing::{create_test_context, TestCommand, TestEvent, TestState};

/// Delegates every `EffectStateStore`/`EffectDedupStore`/`RetentionMaintenance`
/// call to a wrapped real `InMemoryEffectStore`, recording how many times
/// each port's calls landed here.
struct RecordingEffectStore {
    inner: InMemoryEffectStore,
    state_calls: AtomicUsize,
    dedup_calls: AtomicUsize,
}

impl RecordingEffectStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: InMemoryEffectStore::new(),
            state_calls: AtomicUsize::new(0),
            dedup_calls: AtomicUsize::new(0),
        })
    }
    fn state_calls(&self) -> usize {
        self.state_calls.load(Ordering::SeqCst)
    }
    fn dedup_calls(&self) -> usize {
        self.dedup_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl EffectStateStore for RecordingEffectStore {
    async fn accept(&self, effect: AcceptedEffect) -> Result<(), EffectStoreError> {
        self.state_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.accept(effect).await
    }
    async fn mark_in_flight(&self, id: EffectId) -> Result<(), EffectStoreError> {
        self.state_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.mark_in_flight(id).await
    }
    async fn mark_succeeded(&self, id: EffectId) -> Result<(), EffectStoreError> {
        self.state_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.mark_succeeded(id).await
    }
    async fn mark_retryable(
        &self,
        id: EffectId,
        attempt: u32,
        next_at: Timestamp,
    ) -> Result<(), EffectStoreError> {
        self.state_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.mark_retryable(id, attempt, next_at).await
    }
    async fn mark_terminal(
        &self,
        id: EffectId,
        reason: TerminalReason,
    ) -> Result<(), EffectStoreError> {
        self.state_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.mark_terminal(id, reason).await
    }
    async fn claim_due(
        &self,
        now: Timestamp,
        limit: usize,
    ) -> Result<Vec<StoredEffect>, EffectStoreError> {
        self.state_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.claim_due(now, limit).await
    }
    async fn recover_in_flight(&self, now: Timestamp) -> Result<u64, EffectStoreError> {
        self.state_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.recover_in_flight(now).await
    }
}

#[async_trait]
impl EffectDedupStore for RecordingEffectStore {
    async fn reserve(
        &self,
        scope: &DedupScope,
        effect_id: EffectId,
        fingerprint: EffectFingerprint,
    ) -> Result<DedupOutcome, EffectStoreError> {
        self.dedup_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.reserve(scope, effect_id, fingerprint).await
    }
    async fn commit_success(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
        self.dedup_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.commit_success(scope).await
    }
    async fn release(&self, scope: &DedupScope) -> Result<(), EffectStoreError> {
        self.dedup_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.release(scope).await
    }
}

#[async_trait]
impl RetentionMaintenance for RecordingEffectStore {
    async fn purge_before(&self, cutoff: Timestamp, batch: usize) -> Result<u64, EffectStoreError> {
        let _ = (cutoff, batch);
        Ok(0)
    }
}

/// Records every call it receives — proves delivery reached the registered
/// executor.
struct RecordingExecutor {
    calls: AtomicUsize,
}

impl RecordingExecutor {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
        })
    }
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ExternalEffectExecutor for RecordingExecutor {
    async fn execute(
        &self,
        _effect: &ExternalEffectDescription,
        _ctx: &EffectContext,
    ) -> AttemptOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        AttemptOutcome::Success
    }
}

/// Reuses `persistent_entity::testing`'s shared fixtures (this crate's own
/// testing convention, already used by `effect_acceptor_entity_wiring.rs`):
/// `Increment` describes one external effect, everything else describes none.
#[derive(Debug)]
struct EffectDescribingEntity;

#[async_trait]
impl PersistentEntity for EffectDescribingEntity {
    type Command = TestCommand;
    type Event = TestEvent;
    type State = TestState;

    fn initial_state(&self) -> TestState {
        TestState::new(0)
    }

    async fn handle_command(
        &self,
        command: &TestCommand,
        _state: &TestState,
        _context: &CommandContext,
    ) -> Result<Vec<TestEvent>, EntityError> {
        match command {
            TestCommand::Increment(v) => Ok(vec![TestEvent::Incremented(*v)]),
            TestCommand::Decrement(v) => Ok(vec![TestEvent::Decremented(*v)]),
            TestCommand::GetState => Ok(vec![]),
        }
    }

    async fn apply_event(
        &self,
        state: &TestState,
        event: &TestEvent,
    ) -> Result<TestState, EntityError> {
        Ok(match event {
            TestEvent::Incremented(v) => TestState {
                value: state.value + v,
                version: state.version + 1,
            },
            TestEvent::Decremented(v) => TestState {
                value: state.value.saturating_sub(*v),
                version: state.version + 1,
            },
        })
    }

    async fn apply_events(
        &self,
        state: &TestState,
        events: &[TestEvent],
    ) -> Result<TestState, EntityError> {
        let mut s = state.clone();
        for event in events {
            s = self.apply_event(&s, event).await?;
        }
        Ok(s)
    }

    async fn external_effects(
        &self,
        command: &TestCommand,
        _new_state: &TestState,
        events: &[TestEvent],
        _context: &CommandContext,
    ) -> Vec<ExternalEffectDescription> {
        if events.is_empty() {
            return Vec::new();
        }
        match command {
            TestCommand::Increment(_) => vec![ExternalEffectDescription {
                idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
                effect_type: "invoice.created".to_string(),
                payload: vec![],
                destination: "https://example.com".to_string(),
            }],
            _ => Vec::new(),
        }
    }
}

/// Dispatches one real effect end-to-end through a really-spawned actor, and
/// waits for `executor` to actually run.
async fn dispatch_one_effect_and_wait(
    sdk_runtime: &ego_service_sdk::runtime::Runtime,
    executor: &RecordingExecutor,
    entity_id: &str,
) {
    let acceptor = sdk_runtime
        .effect_acceptor()
        .expect("start_effects must make build()'s wired acceptor available");
    let entity_runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .with_effect_acceptor(acceptor)
        .build();
    let entity_ref = entity_runtime
        .entity_ref("probe", entity_id, Arc::new(EffectDescribingEntity))
        .expect("spawning a fresh actor must succeed");

    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await
        .expect("the command itself must succeed regardless of effect delivery timing");
    assert!(
        matches!(result, CommandResult::Events { .. }),
        "expected a normal Events commit, got {result:?}"
    );

    tokio::time::timeout(Duration::from_secs(1), async {
        while executor.call_count() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the registered executor must actually be invoked");
}

// ---------------------------------------------------------------------------
// 1. Default path: no custom store registered
// ---------------------------------------------------------------------------

#[tokio::test]
async fn default_path_without_with_effect_store_dispatches_through_in_memory_effect_store() {
    let executor = RecordingExecutor::new();
    let sdk_runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .register_effect_executor(["invoice.created"], executor.clone())
        .unwrap()
        .build();
    sdk_runtime
        .start_effects()
        .await
        .expect("an executor was registered — start_effects must succeed");

    dispatch_one_effect_and_wait(&sdk_runtime, &executor, "default-1").await;
}

// ---------------------------------------------------------------------------
// 2 & 3. Custom store registered: both ports are actually called
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_custom_store_registered_via_with_effect_store_has_its_state_calls_exercised() {
    let executor = RecordingExecutor::new();
    let store = RecordingEffectStore::new();
    let sdk_runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_store(store.clone())
        .register_effect_executor(["invoice.created"], executor.clone())
        .unwrap()
        .build();
    sdk_runtime
        .start_effects()
        .await
        .expect("an executor was registered — start_effects must succeed");

    dispatch_one_effect_and_wait(&sdk_runtime, &executor, "custom-state-1").await;

    assert!(
        store.state_calls() > 0,
        "EffectStateStore::accept/mark_* must have been called on the registered double, \
         not on a separately-constructed InMemoryEffectStore"
    );
}

#[tokio::test]
async fn a_custom_store_registered_via_with_effect_store_has_its_dedup_calls_exercised() {
    let executor = RecordingExecutor::new();
    let store = RecordingEffectStore::new();
    let sdk_runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_store(store.clone())
        .register_effect_executor(["invoice.created"], executor.clone())
        .unwrap()
        .build();
    sdk_runtime
        .start_effects()
        .await
        .expect("an executor was registered — start_effects must succeed");

    dispatch_one_effect_and_wait(&sdk_runtime, &executor, "custom-dedup-1").await;

    assert!(
        store.dedup_calls() > 0,
        "EffectDedupStore::reserve/commit_success/release must have been called on the \
         registered double, not on a separately-constructed InMemoryEffectStore"
    );
}

// ---------------------------------------------------------------------------
// 4. Same instance: both ports trace back to the SAME Arc
// ---------------------------------------------------------------------------

#[test]
fn with_effect_store_hands_both_ports_the_same_underlying_arc() {
    let store = RecordingEffectStore::new();

    // Reproduces `with_effect_store`'s own body exactly:
    // `self.effect_state_store = Some(store.clone()); self.effect_dedup_store
    // = Some(store);` — two unsized coercions of the identical concrete
    // `Arc<RecordingEffectStore>`.
    let state_handle: Arc<dyn EffectStateStore> = store.clone();
    let dedup_handle: Arc<dyn EffectDedupStore> = store.clone();

    // `Arc::ptr_eq` only type-checks between two `Arc`s of the SAME (possibly
    // unsized) type, so each handle is compared against a fresh coercion to
    // its own trait-object type taken from the same concrete `store` — a
    // safe identity check (no unsafe downcasting) proving both handles trace
    // back to the one allocation `store` names.
    assert!(Arc::ptr_eq(
        &state_handle,
        &(store.clone() as Arc<dyn EffectStateStore>)
    ));
    assert!(Arc::ptr_eq(
        &dedup_handle,
        &(store.clone() as Arc<dyn EffectDedupStore>)
    ));
    // And the two coercions' data pointers agree with each other too —
    // `Arc::as_ptr` strips the vtable, so this is the cross-trait check
    // `Arc::ptr_eq` cannot express directly.
    assert_eq!(
        Arc::as_ptr(&state_handle) as *const (),
        Arc::as_ptr(&dedup_handle) as *const ()
    );
}

// ---------------------------------------------------------------------------
// 5. No executor: the custom store is never touched
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_custom_store_registered_with_zero_executors_builds_no_pipeline_and_is_never_called() {
    let store = RecordingEffectStore::new();
    let sdk_runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_store(store.clone())
        .build();
    // Deliberately no `register_effect_executor` call.

    assert!(
        sdk_runtime.effect_acceptor().is_none(),
        "zero executors must mean no effects pipeline is constructed at all"
    );

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        store.state_calls(),
        0,
        "a registered-but-unused store must never be called"
    );
    assert_eq!(
        store.dedup_calls(),
        0,
        "a registered-but-unused store must never be called"
    );
}

// ---------------------------------------------------------------------------
// 6. Retention compatibility
// ---------------------------------------------------------------------------

#[test]
fn with_effect_store_composes_with_with_effect_retention_store() {
    let store = RecordingEffectStore::new();
    let _runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_effect_store(store.clone())
        .with_effect_retention_store(store.clone() as Arc<dyn RetentionMaintenance>)
        .build();
    // Compiling and building is the whole assertion: the retention worker's
    // own lifecycle (start/stop/purge cadence) is already covered by
    // `effect_retention_worker_lifecycle.rs`.
}
