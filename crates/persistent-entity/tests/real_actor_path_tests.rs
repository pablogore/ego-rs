/// Integration tests for the real `TokioEntityRef` → `EntityActor` path.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;

use ego_domain::persistence::{PersistenceError, Snapshot};
use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::effect_acceptor::{EffectAcceptanceError, EffectAcceptor};
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::test_entity::TestEntity;
use persistent_entity::testing::{TestCommand, TestEvent, TestState};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn handler(
) -> Arc<dyn persistent_entity::persistent_entity::PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>>
{
    Arc::new(TestEntity::new())
}

fn ctx() -> CommandContext {
    CommandContext::new("counter".to_string())
}

// ---------------------------------------------------------------------------
// Failing snapshot store — causes recovery to return an error
// ---------------------------------------------------------------------------

struct FailingSnapshotStore;

impl Snapshot for FailingSnapshotStore {
    fn save_snapshot(
        &mut self,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _version: i64,
        _payload: serde_json::Value,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn load_snapshot(
        &self,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Option<(i64, serde_json::Value)>, PersistenceError> {
        Err(PersistenceError::Internal(
            "injected recovery failure".to_string(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Basic command execution inside a current_thread runtime
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn test_real_actor_command_reply() {
    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .build();
    let entity_ref = runtime.entity_ref::<TestCommand, TestState>("counter", "c1", handler()).unwrap();

    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        entity_ref.send_command(TestCommand::Increment(5), ctx()).await;

    assert!(result.is_ok(), "command should succeed: {:?}", result.err());
    match result.unwrap() {
        CommandResult::Events { new_state, events } => {
            assert_eq!(new_state.value, 5);
            assert_eq!(events.len(), 1);
        }
        other => panic!("expected CommandResult::Events, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Recovery from pre-seeded events
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn test_recovery_replays_seeded_events() {
    use ego_domain::persistence::{EventStore, StoredEvent};
    use persistent_entity::persistence::InMemoryEventStore;

    let event_store = Arc::new(Mutex::new(InMemoryEventStore::<TestEvent>::new()));

    // EntityTriple::aggregate_id() returns "{entity_type}-{entity_id}".
    // Pre-seed under the same key the actor will use for recovery.
    let aggregate_key = "counter-c-recovery";
    let events: Vec<StoredEvent<TestEvent>> = (1u64..=3)
        .map(|v| StoredEvent::without_correlation(TestEvent::Incremented(v)))
        .collect();
    {
        let mut store = event_store.lock();
        store
            .append(aggregate_key, None, 0, events)
            .expect("pre-seed must succeed");
    }

    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_event_store(event_store)
        .build();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("counter", "c-recovery", handler()).unwrap();

    // GetState emits no events — the actor returns the recovered state as-is.
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        entity_ref.send_command(TestCommand::GetState, ctx()).await;

    assert!(result.is_ok(), "GetState should succeed: {:?}", result.err());
    match result.unwrap() {
        CommandResult::NoEvents { state } => {
            // 3 events seeded, each increments state.version by 1.
            assert_eq!(
                state.version, 3,
                "recovered state should reflect 3 replayed events"
            );
            // values are 1+2+3 = 6
            assert_eq!(state.value, 6, "recovered value should be 1+2+3=6");
        }
        other => panic!("expected CommandResult::NoEvents, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Passivation: actor exits after timeout, registry updated
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_passivation_updates_registry() {
    let runtime = Arc::new(
        EntityRuntimeBuilder::<TestEvent>::new()
            // Very short timeout so passivation fires quickly.
            .passivation_timeout(Duration::from_millis(10))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build(),
    );

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("counter", "p1", handler()).unwrap();

    // Send a command before passivation fires — must be replied.
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        entity_ref.send_command(TestCommand::Increment(1), ctx()).await;
    assert!(result.is_ok(), "pre-passivation command should succeed");

    // Wait for the actor to passivate.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if runtime.passivated_count() >= 1 {
            break;
        }
        if std::time::Instant::now() > deadline {
            panic!("actor did not passivate within 2 s");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    assert_eq!(
        runtime.passivated_count(),
        1,
        "registry must record one passivated entity"
    );
}

// ---------------------------------------------------------------------------
// Recovery failure: actor must not hang, active_count stays zero
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_recovery_failure_returns_error() {
    let failing_snapshot_store = Arc::new(Mutex::new(FailingSnapshotStore));

    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_snapshot_store(failing_snapshot_store)
        .build();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("counter", "fail-1", handler()).unwrap();

    // The actor fails recovery synchronously inside its spawned task.
    // send_command must get a reply (Err) rather than hanging forever.
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        tokio::time::timeout(
            Duration::from_secs(2),
            entity_ref.send_command(TestCommand::Increment(1), ctx()),
        )
        .await
        .expect("send_command must complete within 2 s (no hang)");

    assert!(
        result.is_err(),
        "command after recovery failure must return Err"
    );

    // The actor must have exited without marking itself active.
    // Give the task a moment to finalise.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        runtime.active_count(),
        0,
        "failed actor must not appear as active"
    );
}

// ---------------------------------------------------------------------------
// CORE-019 Phase 12.2: EntityRuntimeBuilder::with_effect_acceptor reaches a
// REAL tokio::spawn-ed EntityActor via the full TokioEntityRef::new path —
// closes the PR3/PR4-documented gap (design.md Phase 9 notes: "actually
// plumbing that acceptor into persistent_entity::builder::EntityRuntimeBuilder
// / EntityRuntime / TokioEntityRef::new ... is left to whichever host
// constructs both runtimes ... Phase 12/PR5's explicit scope").
// ---------------------------------------------------------------------------

/// Describes one external effect for `Increment`, none otherwise — mirrors
/// `actor.rs`'s unit-test-only `EffectEmittingHandler`, but spawned through
/// the real builder/`TokioEntityRef` path instead of a hand-built
/// `EntityActor` struct literal.
#[derive(Debug)]
struct EffectEmittingEntity;

#[async_trait]
impl PersistentEntity for EffectEmittingEntity {
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

    async fn apply_event(&self, state: &TestState, event: &TestEvent) -> Result<TestState, EntityError> {
        match event {
            TestEvent::Incremented(v) => Ok(TestState {
                value: state.value + v,
                version: state.version + 1,
            }),
            TestEvent::Decremented(v) => Ok(TestState {
                value: state.value.saturating_sub(*v),
                version: state.version + 1,
            }),
        }
    }

    async fn apply_events(&self, state: &TestState, events: &[TestEvent]) -> Result<TestState, EntityError> {
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
                idempotency_key: IdempotencyKey::new("real-actor-path:0").unwrap(),
                effect_type: "probe.effect".to_string(),
                payload: vec![9, 9, 9],
                destination: "https://example.com/probe".to_string(),
            }],
            _ => Vec::new(),
        }
    }
}

struct RecordingAcceptor {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl EffectAcceptor for RecordingAcceptor {
    async fn accept(
        &self,
        _tenant: &TenantId,
        effects: Vec<ExternalEffectDescription>,
    ) -> Result<(), EffectAcceptanceError> {
        self.calls.fetch_add(effects.len(), Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn builder_wired_effect_acceptor_reaches_a_real_spawned_actor() {
    let calls = Arc::new(AtomicUsize::new(0));
    let acceptor: Arc<dyn EffectAcceptor> = Arc::new(RecordingAcceptor { calls: calls.clone() });

    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_effect_acceptor(acceptor)
        .build();

    let entity_ref = runtime
        .entity_ref::<TestCommand, TestState>("counter", "effects-wired-1", Arc::new(EffectEmittingEntity))
        .unwrap();

    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        entity_ref.send_command(TestCommand::Increment(1), ctx()).await;

    assert!(result.is_ok(), "command should succeed: {:?}", result.err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the acceptor registered via EntityRuntimeBuilder::with_effect_acceptor must be reached \
         by a real tokio::spawn-ed EntityActor through TokioEntityRef::new — this is the exact \
         gap PR4 documented and left to this PR"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn without_with_effect_acceptor_described_effects_fail_closed_not_silently_dropped() {
    // Triangulation, updated for PR4's F-03 review-round fix (AD-9): the
    // zero-cost default (no acceptor configured) commits the event as
    // normal, but a described effect that has nowhere to go now fails
    // closed with an honest `EffectsAcceptanceFailed` reply — it is never
    // silently dropped as if nothing had been described (see
    // `actor.rs`'s `missing_acceptor_with_described_effects_fails_closed_not_silently_discarded`
    // and the companion `ego-service-sdk` proof,
    // `without_with_effect_acceptor_the_executor_is_never_reached`).
    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .build();

    let entity_ref = runtime
        .entity_ref::<TestCommand, TestState>("counter", "effects-unwired-1", Arc::new(EffectEmittingEntity))
        .unwrap();

    let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
        entity_ref.send_command(TestCommand::Increment(1), ctx()).await;

    assert!(
        matches!(result, Ok(CommandResult::EffectsAcceptanceFailed { .. })),
        "no acceptor configured: the commit must still happen, but a described effect must fail \
         closed, not silently succeed as a normal commit: got {result:?}"
    );
}
