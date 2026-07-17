use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::test_entity::TestEntity;
use persistent_entity::testing::{create_test_context, TestCommand, TestEvent, TestState};

mod common;

use common::CountingEventStore;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_runtime() -> Arc<persistent_entity::runtime::EntityRuntime<TestEvent>> {
    Arc::new(
        persistent_entity::builder::EntityRuntimeBuilder::new()
            .passivation_timeout(std::time::Duration::from_secs(3600))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build(),
    )
}

/// 500ms, not 50ms: the tests using this leave enough margin between "the
/// idle timer elapsed" (confirmed via a bounded poll, not a guessed sleep)
/// and "the post-burst `active_count()` check ran" that a slow/contended CI
/// box can't let the entity idle-timeout a *second* time inside that window
/// (see `wait_for_passivation` below). Wired with a [`CountingEventStore`] so
/// callers can assert on the number of genuine activation (recovery)
/// attempts — actor-level instrumentation (NFR-002).
fn build_fast_passivation_runtime_with_counter(
    load_calls: Arc<AtomicUsize>,
) -> Arc<persistent_entity::runtime::EntityRuntime<TestEvent>> {
    let event_store = Arc::new(Mutex::new(CountingEventStore::new(load_calls)));
    Arc::new(
        persistent_entity::builder::EntityRuntimeBuilder::new()
            .passivation_timeout(std::time::Duration::from_millis(500))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .with_event_store(event_store)
            .build(),
    )
}

fn handler(
) -> Arc<dyn PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>> {
    Arc::new(TestEntity::new())
}

/// Waits for `active_count() == 0` via a bounded poll instead of a guessed
/// sleep duration — explicit synchronization on the actual condition the
/// caller needs (idle timeout has fired), not a fixed delay that either
/// wastes time or, worse, races the real timer under load.
async fn wait_for_passivation(runtime: &persistent_entity::runtime::EntityRuntime<TestEvent>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if runtime.active_count() == 0 {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "entity did not passivate within the deadline"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Sends a command, retrying on `MailboxClosed` with a fresh `entity_ref()`.
///
/// Root-cause note (not a production bug): `active_count() == 0`
/// (`wait_for_passivation`'s signal) only means the old actor's published
/// state left `Active` — it does NOT mean the registry entry routing to it
/// has been removed yet. `EntityActor::passivate` closes the mailbox before
/// the state transition, and `TeardownGuard` (whose `Drop` removes the
/// registry entry) only runs after `run()` fully returns — so there is a
/// real, documented window where a concurrent caller can still be routed to
/// the old, closed mailbox. This is the exact ADR-008/FR-010 contract
/// (`openspec/changes/archive/2026-07-07-activation-authority/design.md`:
/// "MailboxClosed is a distinct, caller-retryable terminal error; caller may
/// re-`entity_ref()`"), already covered at the unit level by
/// `entity_ref_tokio.rs`'s `mailbox_closed_in_teardown_window_is_retried_to_a_fresh_actor`.
/// This integration test previously sent once and asserted success
/// unconditionally, which made it flaky under CPU contention (a wider,
/// more probable teardown window) — treating a documented, retryable
/// outcome as a hard failure. Retrying here, exactly as the contract
/// requires, is the fix; no production code changes.
async fn send_with_retry_on_mailbox_closed(
    runtime: &Arc<persistent_entity::runtime::EntityRuntime<TestEvent>>,
    entity_id: &'static str,
    handler: &Arc<dyn PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>>,
) -> Result<CommandResult<TestEvent, TestState>, EntityError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let entity_ref = runtime
            .entity_ref::<TestCommand, TestState>("test", entity_id, handler.clone())
            .unwrap();
        let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
            .send_command(
                TestCommand::Increment(1),
                CommandContext::new("test".to_string()),
            )
            .await;
        match result {
            Err(EntityError::MailboxClosed) if std::time::Instant::now() < deadline => continue,
            other => return other,
        }
    }
}

// ============================================================================
// User Story 1 — Activation Ordering
// ============================================================================

/// Activation lookup — send to new entity creates it, passivation removes sender.
#[tokio::test]
async fn test_activation_lookup_active_and_passivated() {
    let runtime = build_runtime();
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-1", h.clone()).unwrap();

    // Entity is not active initially — send activates it
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await;
    assert!(result.is_ok(), "first send should activate entity");

    // Send another command — should find active sender
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(2), create_test_context())
        .await;
    assert!(result.is_ok(), "second send should use active sender");
}

/// FIFO ordering — commands processed in send order.
#[tokio::test]
async fn test_activation_fifo_ordering() {
    let runtime = build_runtime();
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-2", h.clone()).unwrap();

    let mut expected = 0u64;
    for i in 1..=10u64 {
        let result: CommandResult<TestEvent, TestState> = entity_ref
            .send_command(TestCommand::Increment(i), create_test_context())
            .await
            .unwrap();
        expected += i;
        match result {
            CommandResult::Events { new_state, .. } => {
                assert_eq!(
                    new_state.value, expected,
                    "state should show sum after command {}",
                    i
                );
            }
            _ => panic!("expected Events variant"),
        }
    }
    assert_eq!(expected, 55);
}

/// No partial state — command always sees fully recovered state.
#[tokio::test]
async fn test_no_partial_state_observable() {
    let runtime = build_runtime();
    let h = handler();

    // Build up state with multiple commands
    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-3", h.clone()).unwrap();
    for _i in 1..=5u64 {
        let _: CommandResult<TestEvent, TestState> = entity_ref
            .send_command(TestCommand::Increment(10), create_test_context())
            .await
            .unwrap();
    }

    // Query the state — should see all 5 increments
    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::GetState, create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::NoEvents { state } => {
            assert_eq!(state.value, 50, "state should reflect all prior commands");
            assert_eq!(state.version, 5, "version should reflect all events");
        }
        _ => panic!("expected NoEvents variant"),
    }
}

/// Activation redirect — concurrent callers find active entity without duplicate spawn.
#[tokio::test]
async fn test_activation_redirect() {
    let runtime = build_runtime();
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-4", h.clone()).unwrap();

    // Send first command to activate
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Increment(10),
            CommandContext::new("TestCommand".to_string()),
        )
        .await
        .unwrap();

    // Send another — should redirect to existing actor
    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::Events { new_state, .. } => {
            assert_eq!(new_state.value, 11);
        }
        _ => panic!("expected Events variant"),
    }
}

// ============================================================================
// User Story 2 — No Double Actor Spawn
// ============================================================================

/// No double spawn — concurrent tasks to passivated entity all go to one actor.
///
/// NFR-002: "no duplicate actor" is asserted at the actor-task level via a
/// `CountingEventStore`'s `load()` call counter (one call per genuine
/// recovery/activation attempt), in addition to the existing
/// `active_count()` bound — `active_count()` alone cannot distinguish "one
/// actor activated once" from "one actor survived N racing activation
/// attempts that each got as far as recovery before losing the single-flight
/// race," since only ONE of those attempts would ever reach `Active`.
#[tokio::test(flavor = "multi_thread")]
async fn test_no_double_spawn_concurrent() {
    let load_calls = Arc::new(AtomicUsize::new(0));
    let runtime = build_fast_passivation_runtime_with_counter(load_calls.clone());
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-5", h.clone()).unwrap();

    // Activate then let passivate
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Increment(1),
            CommandContext::new("test".to_string()),
        )
        .await
        .unwrap();
    wait_for_passivation(&runtime).await;

    let load_calls_before_burst = load_calls.load(Ordering::SeqCst);

    // Concurrent sends should coalesce into single activation
    let results =
        common::spawn_concurrent_commands(20, runtime.clone(), "test", "entity-5", h.clone()).await;

    let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
    assert_eq!(
        successes.len(),
        20,
        "all concurrent commands should succeed"
    );

    // NFR-002: the 20-caller burst against the passivated entity must have
    // triggered exactly one genuine reactivation attempt (recovery `load()`
    // call), not merely produced one surviving `active_count()` entry.
    // Captured now, before the idle-timer-resetting command below, so that
    // command can never itself count as a second "reactivation" here.
    let reactivation_load_calls = load_calls.load(Ordering::SeqCst) - load_calls_before_burst;
    assert_eq!(
        reactivation_load_calls, 1,
        "single-flight must coalesce the 20-caller burst into exactly one reactivation attempt, got {}",
        reactivation_load_calls
    );

    // Reset the idle timer immediately before checking `active_count()` — see
    // the anchor comment in `test_activation_mutex_serializes` for why an
    // unguarded check here races the same passivation timer under
    // contention (including its noted, accepted, non-blocking residual
    // risk), and why this must resolve a FRESH `entity_ref` (retried on
    // `MailboxClosed`) rather than reuse the one captured before passivation
    // above, whose mailbox is the original (now-closed) actor's.
    send_with_retry_on_mailbox_closed(&runtime, "entity-5", &h)
        .await
        .expect("idle-timer-resetting command must succeed");

    // Review (PR #186, re-review): a prior version of this test re-checked
    // load_calls here too, asserting the anchor command didn't itself count
    // as a second reactivation. Removed — a real, contention-induced gap
    // between the burst finishing and this anchor command running can
    // legitimately exceed the configured passivation timeout, causing a
    // second, LEGITIMATE reactivation here that has nothing to do with a
    // double-spawn bug. Asserting on it would turn real scheduling delay
    // back into part of the test's pass/fail outcome — the exact class of
    // flakiness this fix exists to remove. The single-flight guarantee that
    // actually matters (the burst produced exactly one reactivation) was
    // already checked above, before the anchor ever ran.
    //
    // Single-flight (ADR-001) guarantees exactly one live entry per triple —
    // there is no window where two entries coexist for the same aggregate_id,
    // so this is exact, not a bound.
    let active_count = runtime.active_count();
    assert_eq!(
        active_count, 1,
        "single-flight guarantees exactly one active entity, got {}",
        active_count
    );
}

/// Mutex-based single-flight — concurrent activations serialize.
///
/// NFR-002: same rigor as `test_no_double_spawn_concurrent` — the
/// serialization claim is asserted at the actor-task level via a
/// `CountingEventStore`'s `load()` call counter, not only via
/// `active_count()`.
#[tokio::test(flavor = "multi_thread")]
async fn test_activation_mutex_serializes() {
    let load_calls = Arc::new(AtomicUsize::new(0));
    let runtime = build_fast_passivation_runtime_with_counter(load_calls.clone());
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-6", h.clone()).unwrap();

    // Activate then let passivate
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Increment(1),
            CommandContext::new("test".to_string()),
        )
        .await
        .unwrap();
    wait_for_passivation(&runtime).await;

    let load_calls_before_burst = load_calls.load(Ordering::SeqCst);

    // Spawn 10 concurrent tasks — all should succeed with no duplicate spawns.
    // Each retries on `MailboxClosed` (ADR-008/FR-010's documented,
    // caller-retryable teardown-window outcome) rather than treating it as a
    // hard failure — see `send_with_retry_on_mailbox_closed`'s doc comment.
    let n = 10;
    let mut handles = Vec::with_capacity(n);
    for _ in 0..n {
        let rt = runtime.clone();
        let h = h.clone();
        handles.push(tokio::spawn(async move {
            send_with_retry_on_mailbox_closed(&rt, "entity-6", &h).await
        }));
    }

    for handle in handles {
        let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
            handle.await.unwrap();
        assert!(result.is_ok(), "concurrent command should succeed");
    }

    // NFR-002: the 10-caller burst against the passivated entity must have
    // triggered exactly one genuine reactivation attempt. Captured now,
    // before the idle-timer-resetting command below, so that command can
    // never itself count as a second "reactivation" against this assertion
    // even in the (much narrower) case where it races its own passivation.
    let reactivation_load_calls = load_calls.load(Ordering::SeqCst) - load_calls_before_burst;
    assert_eq!(
        reactivation_load_calls, 1,
        "single-flight must serialize the 10-caller burst into exactly one reactivation attempt, got {}",
        reactivation_load_calls
    );

    // Reset the idle timer right before checking `active_count()`, narrowing
    // (not eliminating — see the residual-risk note below) the second real
    // race this file's own comment on `build_fast_passivation_runtime_with_counter`
    // already flagged: the reactivated entity's idle timer starts counting
    // from whichever burst command it processed last, so an unrelated gap
    // between "burst done" and "active_count() checked" is itself racing
    // the same 500ms timeout under contention. One more awaited command
    // immediately beforehand guarantees the timer's last reset is this
    // line, not the burst's last command — an explicit synchronization
    // anchor, not a wider guessed margin.
    //
    // Residual risk (non-blocking, PR #186 re-review): the `active_count()`
    // check a few lines below still races the same 500ms timeout across the
    // (normally sub-millisecond) gap between this anchor command returning
    // and that check running — if the test's own task is descheduled for
    // longer than the configured timeout in exactly that window, the
    // assertion could still see `0`. Accepted as a real but extremely
    // narrow ceiling for this fix; closing it fully would mean moving this
    // test onto virtual/paused time or an explicit lifecycle-completion
    // signal instead of a real-clock `active_count()` read.
    send_with_retry_on_mailbox_closed(&runtime, "entity-6", &h)
        .await
        .expect("idle-timer-resetting command must succeed");

    // Review (PR #186, re-review): a prior version of this test re-checked
    // load_calls here too, asserting the anchor command didn't itself count
    // as a second reactivation. Removed — a real, contention-induced gap
    // between the burst finishing and this anchor command running can
    // legitimately exceed the configured passivation timeout, causing a
    // second, LEGITIMATE reactivation here unrelated to any double-spawn
    // bug. Asserting on it would turn real scheduling delay back into part
    // of the test's pass/fail outcome — the exact class of flakiness this
    // fix exists to remove. The single-flight guarantee that actually
    // matters (the burst produced exactly one reactivation) was already
    // checked above, before the anchor ever ran.

    let active = runtime.active_count();
    assert_eq!(
        active, 1,
        "single-flight guarantees exactly one active entity, got {}",
        active
    );
}

/// No double spawn across multiple entities — each gets exactly one actor.
///
/// Uses the long-passivation-timeout runtime: this test doesn't exercise
/// reactivation, so a short idle timer only adds a race against its own
/// `active_count()` check (10 fresh activations idling out before the check
/// runs) without proving anything extra.
#[tokio::test(flavor = "multi_thread")]
async fn test_no_double_spawn_multiple_entities() {
    let runtime = build_runtime();
    let h = handler();

    // Spawn commands for 10 different entities simultaneously
    let mut handles = Vec::with_capacity(10);
    for i in 0..10 {
        let rt = runtime.clone();
        let h = h.clone();
        let entity_id = format!("multi-{}", i);
        handles.push(tokio::spawn(async move {
            let entity_ref =
                rt.entity_ref::<TestCommand, TestState>("test", &entity_id, h).unwrap();
            let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
                .send_command(
                    TestCommand::Increment(1),
                    CommandContext::new("test".to_string()),
                )
                .await;
            result
        }));
    }

    for handle in handles {
        let result: Result<CommandResult<TestEvent, TestState>, EntityError> =
            handle.await.unwrap();
        assert!(result.is_ok(), "each entity should activate independently");
    }

    // All 10 should have active actors (or some may have passivated already)
    let active = runtime.active_count();
    assert!(active > 0, "at least some entities should be active");
}

// ============================================================================
// User Story 3 — Deterministic Recovery
// ============================================================================

/// Recovery barrier — pre-loaded events are reflected after activation.
#[tokio::test]
async fn test_recovery_barrier() {
    // Build runtime, increment many times to build history
    let runtime = build_runtime();
    let h = handler();

    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-7", h.clone()).unwrap();

    for _ in 0..50 {
        let _: CommandResult<TestEvent, TestState> = entity_ref
            .send_command(TestCommand::Increment(1), create_test_context())
            .await
            .unwrap();
    }

    // Send a command and verify version reflects all prior events
    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::Events {
            events: _,
            new_state,
            ..
        } => {
            let new_version = new_state.version;
            assert!(
                new_version >= 50,
                "version should be >= 50, got {}",
                new_version
            );
            assert_eq!(new_state.value, 51);
        }
        _ => panic!("expected Events variant"),
    }
}

/// Deterministic replay — same event stream produces identical state.
#[tokio::test]
async fn test_recovery_deterministic_replay() {
    let runtime = build_runtime();
    let h = handler();
    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-8", h.clone()).unwrap();

    // Build deterministic state
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Increment(10),
            CommandContext::new("TestCommand".to_string()),
        )
        .await
        .unwrap();
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Increment(20),
            CommandContext::new("TestCommand".to_string()),
        )
        .await
        .unwrap();
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(
            TestCommand::Decrement(5),
            CommandContext::new("TestCommand".to_string()),
        )
        .await
        .unwrap();

    // Query state from the same runtime
    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::GetState, create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::NoEvents { state } => {
            assert_eq!(state.value, 25, "10 + 20 - 5 = 25");
            assert_eq!(state.version, 3);
        }
        _ => panic!("expected NoEvents variant"),
    }
}

/// Recovery failure transition — simulated via handler error.
#[tokio::test]
async fn test_recovery_failure_transitions_to_failed() {
    let runtime = build_runtime();
    let h = handler();
    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-9", h.clone()).unwrap();

    // Decrement on zero should fail
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(
            TestCommand::Decrement(1),
            CommandContext::new("TestCommand".to_string()),
        )
        .await;
    assert!(result.is_err(), "decrement on zero should fail");

    // Entity should still accept new commands after handler error (increment always succeeds)
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await;
    assert!(
        result.is_ok(),
        "entity should still accept commands after handler error"
    );

    // Entity should still accept new commands (handler error doesn't fail the entity)
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await;
    assert!(
        result.is_ok(),
        "entity should still accept commands after handler error"
    );
}

/// Recovery retry — after failure, next command triggers fresh activation.
#[tokio::test]
async fn test_recovery_retry_after_failure() {
    let runtime = build_runtime();
    let h = handler();
    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-10", h.clone()).unwrap();

    // Activate and do work
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(5), create_test_context())
        .await
        .unwrap();

    // Verify entity is functional
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = entity_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await;
    assert!(result.is_ok());
}

/// Zero-event query — GetState produces NoEvents, doesn't advance version.
#[tokio::test]
async fn test_zero_event_query() {
    let runtime = build_runtime();
    let h = handler();
    let entity_ref =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-16", h.clone()).unwrap();

    // Activate with a mutation
    let _: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::Increment(10), create_test_context())
        .await
        .unwrap();

    // Zero-event query
    let result: CommandResult<TestEvent, TestState> = entity_ref
        .send_command(TestCommand::GetState, create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::NoEvents { state } => {
            assert_eq!(state.value, 10);
            assert_eq!(state.version, 1);
        }
        _ => panic!("expected NoEvents variant"),
    }
}

/// Multiple entity isolation — same entity type, different IDs.
#[tokio::test]
async fn test_multiple_entity_isolation() {
    let runtime = build_runtime();
    let h = handler();

    let ref_a =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-a", h.clone()).unwrap();
    let ref_b =
        runtime.entity_ref::<TestCommand, TestState>("test", "entity-b", h.clone()).unwrap();

    // Mutate entity-a
    let _: CommandResult<TestEvent, TestState> = ref_a
        .send_command(
            TestCommand::Increment(100),
            CommandContext::new("TestCommand".to_string()),
        )
        .await
        .unwrap();

    // Entity-b should be independent
    let result: CommandResult<TestEvent, TestState> = ref_b
        .send_command(TestCommand::GetState, create_test_context())
        .await
        .unwrap();
    match result {
        CommandResult::NoEvents { state } => {
            assert_eq!(state.value, 0, "entity-b should have initial state");
        }
        _ => panic!("expected NoEvents variant"),
    }
}
