//! CORE-019 PR4 review Finding F-03: proves the constructed `EffectAcceptor`
//! actually reaches a REAL, production-spawned `EntityActor` — not merely
//! that `Runtime::effect_acceptor()` returns something usable in isolation.
//!
//! Prior to this fix, `RuntimeBuilder::register_effect_executor(..)` had
//! ZERO effect on any actor spawned through the normal
//! `persistent_entity::runtime::EntityRuntime::entity_ref` path: the
//! production spawn path (`TokioEntityRef::new`) unconditionally
//! hard-coded `effect_acceptor: None` on every actor it constructed.
//!
//! This test never calls `Runtime::effect_acceptor()` to drive its own
//! assertions manually — it only uses that accessor to hand the acceptor to
//! `persistent_entity::builder::EntityRuntimeBuilder::with_effect_acceptor`,
//! then spawns a real entity actor and sends it a real command, proving the
//! wiring itself (not the accessor) is real.
//!
//! Run with: cargo test -p ego-service-sdk --test effect_acceptor_entity_wiring

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ego_domain::{ExternalEffectDescription, IdempotencyKey};
use ego_runtime::effects::{AttemptOutcome, EffectContext, ExternalEffectExecutor};
use ego_service_sdk::runtime::RuntimeBuilder;
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::testing::{create_test_context, TestCommand, TestEvent, TestState};

/// Records every call it actually receives — the proof that delivery, not
/// just acceptance, reached the registered executor.
struct RecordingExecutor {
    calls: AtomicUsize,
}

impl RecordingExecutor {
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

/// Describes one external effect for `Increment`, none for `Decrement`/
/// `GetState` — reuses `persistent_entity::testing`'s shared
/// `TestCommand`/`TestEvent`/`TestState` fixtures (this crate's own testing
/// conventions) rather than inventing a parallel set of test types.
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

/// F-03's core proof: a host that registers an executor on
/// `service_sdk::RuntimeBuilder` AND wires `Runtime::effect_acceptor()` into
/// `persistent_entity::builder::EntityRuntimeBuilder::with_effect_acceptor`
/// gets a real, working delivery pipeline on actors spawned the normal way
/// (`EntityRuntime::entity_ref`) — not merely on the acceptor in isolation.
#[tokio::test]
async fn effect_executor_registered_on_runtime_builder_is_invoked_by_a_really_spawned_actor() {
    let executor = Arc::new(RecordingExecutor::new());
    let sdk_runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .register_effect_executor(["invoice.created"], executor.clone())
        .unwrap()
        .build();
    sdk_runtime
        .start_effects()
        .await
        .expect("an executor was registered — start_effects must succeed");

    let acceptor = sdk_runtime
        .effect_acceptor()
        .expect("start_effects must make build()'s wired acceptor available");

    let entity_runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .with_effect_acceptor(acceptor)
        .build();

    let entity_ref = entity_runtime
        .entity_ref("probe", "wired-1", Arc::new(EffectDescribingEntity))
        .expect("spawning a fresh actor must succeed");

    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await
        .expect("the command itself must succeed regardless of effect delivery timing");

    assert!(
        matches!(result, CommandResult::Events { .. }),
        "with a real acceptor wired through, the reply must be a normal Events commit, \
         not EffectsAcceptanceFailed: got {result:?}"
    );

    tokio::time::timeout(Duration::from_secs(1), async {
        while executor.call_count() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect(
        "the executor registered on RuntimeBuilder must actually be invoked by the actor \
         spawned through EntityRuntime::entity_ref — proving the wiring is real, not just \
         that the accessor works in isolation",
    );
}

/// Companion/edge-case proof: a host that forgets (or never opts into)
/// `with_effect_acceptor` keeps today's correct fail-closed default — the
/// executor is never reached, and the reply is the documented
/// `EffectsAcceptanceFailed` outcome, not a silently discarded effect.
#[tokio::test]
async fn without_with_effect_acceptor_the_executor_is_never_reached() {
    let executor = Arc::new(RecordingExecutor::new());
    let _sdk_runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .register_effect_executor(["invoice.created"], executor.clone())
        .unwrap()
        .build();
    // Deliberately NOT wiring `.with_effect_acceptor(..)` below — this is
    // the default, opt-in-required path every pre-existing host is on.

    let entity_runtime = EntityRuntimeBuilder::<TestEvent>::new().build();

    let entity_ref = entity_runtime
        .entity_ref("probe", "unwired-1", Arc::new(EffectDescribingEntity))
        .expect("spawning a fresh actor must succeed");

    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await
        .expect("acceptance failure is not a command failure (AD-9)");

    assert!(
        matches!(result, CommandResult::EffectsAcceptanceFailed { .. }),
        "without with_effect_acceptor, a described effect must fail closed, not silently \
         succeed as a normal commit: got {result:?}"
    );
    assert_eq!(
        executor.call_count(),
        0,
        "an executor registered on RuntimeBuilder but never wired into the entity runtime \
         must never be reached by a really-spawned actor"
    );
}
