//! Phase 6 (CORE-006A) — FR-009 (Guaranteed Completion) adversarial proofs.
//!
//! These tests deliberately avoid timing-based assertions and
//! scheduler-dependent races wherever the raced behavior is *not* itself the
//! thing being proven (NFR-001/NFR-002, and the change's own framing). Each
//! test's doc comment states which FR it proves and which adversarial
//! scenario it targets. Where a "real" race is unavoidable given only the
//! crate's public API (no `pub(crate)` access from an integration test), the
//! doc comment says so explicitly and explains the deterministic
//! approximation chosen instead.
//!
//! ## Scope note vs. Phase 3 (TASK-008)
//!
//! Phase 3's `actor.rs` unit test
//! `panic_mid_processing_answers_all_already_enqueued_callers` already fully
//! covers "actor `Active`, N queued commands behind one that panics" — via a
//! hand-built `EntityActor` + `TeardownGuard` inside the crate. TASK-020's
//! literal task text describes exactly that scenario, so writing it again
//! here would be a pure duplicate. Per this phase's framing ("if a task
//! turns out to already be covered, stop and explain why instead of
//! manufacturing artificial work"), TASK-020's slot is used below for the
//! scenario the framing explicitly calls out that TASK-008 does *not*
//! cover: a panic during **recovery** (before the actor ever reaches
//! `Active`), proven through the real production entry point
//! (`EntityRuntime::entity_ref` → `TokioEntityRef::new`) rather than a
//! hand-built actor.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

use async_trait::async_trait;
use ego_domain::persistence::{EventStore, PersistenceError, StoredEvent};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::command_envelope::{ActorEnvelope, CommandEnvelope};
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::mailbox::BoundedMailbox;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::registry::EntityRegistry;
use persistent_entity::scheduler::EntityTriple;
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::test_entity::TestEntity;
use persistent_entity::testing::{create_test_context, TestCommand, TestEvent, TestState};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn handler(
) -> Arc<dyn PersistentEntity<Command = TestCommand, Event = TestEvent, State = TestState>> {
    Arc::new(TestEntity::new())
}

// ---------------------------------------------------------------------------
// Scenario 1 / TASK-020 — panic during recovery (FR-009)
// ---------------------------------------------------------------------------

/// `EventStore` whose `load()` panics — forces a genuine panic inside
/// `EntityActor::recover_state()`, before the actor ever publishes `Active`.
struct PanicOnLoadEventStore {
    load_calls: Arc<AtomicUsize>,
    /// Optional gate: if present, `load()` blocks on it before panicking.
    /// `None` for single-caller tests (no race to guard against). `Some` for
    /// multi-caller tests, so the test can guarantee every concurrent caller
    /// has already enqueued into the original mailbox before the panic (and
    /// the teardown it triggers) can happen — otherwise a straggler racing
    /// past the teardown window could legitimately spawn its own second
    /// activation attempt (FR-010 permits this; it just isn't what these
    /// tests want to isolate), which would also panic here and inflate
    /// `load_calls` past 1.
    /// Held behind the async lock because `&self` must be `Sync` for the
    /// trait's futures to be `Send`, and a receiver is `Send` but not `Sync`.
    /// The lock is never contended — one receiver, one consumer — it exists to
    /// satisfy that bound.
    release_panic: Option<AsyncMutex<tokio::sync::mpsc::Receiver<()>>>,
}

#[async_trait::async_trait]
impl EventStore<TestEvent> for PanicOnLoadEventStore {
    async fn append(
        &mut self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _expected_version: i64,
        _events: Vec<StoredEvent<TestEvent>>,
    ) -> Result<i64, PersistenceError> {
        Ok(0)
    }

    async fn load(
        &self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<TestEvent>>, PersistenceError> {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(release) = &self.release_panic {
            // Awaited, not blocked on: this runs inside the store's own async
            // method now, and a blocking receive here would park a runtime
            // worker for as long as the test holds the gate.
            let _ = release.lock().await.recv().await;
        }
        panic!("guaranteed_completion_tests: intentional panic during recovery load()");
    }

    async fn list_aggregate_ids(
        &self,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        Ok(Vec::new())
    }
}

/// FR-009 — Scenario: panic during recovery. A caller's command is enqueued
/// while the actor is `Recovering`; recovery itself panics (before `Active`
/// is ever published). The caller must observe a terminal outcome, and the
/// registry must be left with no zombie entry — no separate cleanup path,
/// same `TeardownGuard::drop()` contract as every other exit cause (ADR-005).
#[tokio::test]
async fn panic_during_recovery_answers_enqueued_caller_and_leaves_no_zombie() {
    let load_calls = Arc::new(AtomicUsize::new(0));
    let event_store: Arc<AsyncMutex<dyn EventStore<TestEvent> + Send>> =
        Arc::new(AsyncMutex::new(PanicOnLoadEventStore {
            load_calls: load_calls.clone(),
            release_panic: None,
        }));

    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_event_store(event_store)
        .build();

    let triple = EntityTriple::new("default".to_string(), "probe", "recovery-panic-1");
    let aggregate_id = triple.aggregate_id();

    let entity_ref = runtime
        .entity_ref::<TestCommand, TestState>("probe", "recovery-panic-1", handler())
        .unwrap();

    // Enqueued while Recovering: recover_state() panics before this command
    // is ever popped from the mailbox by process_commands().
    let result: Result<CommandResult<TestEvent, TestState>, EntityError> = tokio::time::timeout(
        Duration::from_secs(5),
        entity_ref.send_command(TestCommand::Increment(1), create_test_context()),
    )
    .await
    .expect("FR-009: the enqueued caller must not hang while recovery panics");

    assert!(
        result.is_err(),
        "a caller enqueued during a recovery-time panic must observe a terminal Err, got {result:?}"
    );
    assert_eq!(
        load_calls.load(Ordering::SeqCst),
        1,
        "recovery must have been attempted exactly once"
    );

    // The guard's Drop runs on the actor task's own (already-unwinding)
    // stack, asynchronously relative to this test task; poll for its
    // eventual effect instead of asserting a fixed delay.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if runtime.registry.lookup(&aggregate_id).is_none() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no zombie registry entry must remain after a recovery-time panic"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        runtime.active_count(),
        0,
        "a recovery-time panic must never be counted as active"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2 / TASK-021 — panic mid-drain answers the undrained remainder
// ---------------------------------------------------------------------------

/// Command with a `Boom` variant that panics `handle_command` on demand —
/// same technique as `actor.rs`'s `PanicOnBoomHandler` (TASK-008), reused
/// here through the public `PersistentEntity` trait since an integration
/// test cannot construct `EntityActor` directly (its fields are
/// `pub(crate)`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum ProbeCommand {
    Noop,
    Boom,
}

#[derive(Debug)]
struct PanicOnBoomHandler;

#[async_trait]
impl PersistentEntity for PanicOnBoomHandler {
    type Command = ProbeCommand;
    type Event = TestEvent;
    type State = TestState;

    fn initial_state(&self) -> TestState {
        TestState::new(0)
    }

    async fn handle_command(
        &self,
        command: &ProbeCommand,
        _state: &TestState,
        _context: &CommandContext,
    ) -> Result<Vec<TestEvent>, EntityError> {
        match command {
            ProbeCommand::Noop => Ok(vec![TestEvent::incremented(1)]),
            ProbeCommand::Boom => {
                panic!("guaranteed_completion_tests: intentional panic mid-drain")
            }
        }
    }

    async fn apply_event(
        &self,
        state: &TestState,
        event: &TestEvent,
    ) -> Result<TestState, EntityError> {
        let value = match event {
            TestEvent::Incremented(v) => state.value + v,
            TestEvent::Decremented(v) => state.value.saturating_sub(*v),
        };
        Ok(TestState {
            value,
            version: state.version + 1,
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
}

/// FR-009 — Scenario: panic mid-drain answers the undrained remainder.
///
/// Proves the property TASK-008 does *not* cover: commands already drained
/// and answered with a REAL, successful result before the panic must keep
/// that result untouched, while commands still queued behind the panic
/// point must still resolve — via the guard's backstop drain — to a
/// terminal `Err`, with no `oneshot` left hanging.
///
/// **Deliberately `#[tokio::test]` (current_thread), not `multi_thread`.**
/// `TokioEntityRef::new()` spawns the actor immediately and does not expose
/// its mailbox until after it returns, so the only way for an integration
/// test to enqueue several hand-built envelopes (with its own `oneshot`
/// pairs, to observe each one's fate) ahead of the actor consuming them is
/// to enqueue them before the actor task ever gets scheduled. On a
/// `current_thread` runtime this is *deterministic*, not racy: this test's
/// own async task never yields (every `entity_ref()`/`mailbox.send()` call
/// below resolves without ever returning `Poll::Pending`), so — with a
/// single OS thread driving the executor — the actor's task provably cannot
/// receive any CPU time until this test task itself yields, which only
/// happens at the first `.await` on a reply channel below. Using
/// `multi_thread` here would turn this into exactly the kind of
/// scheduler-dependent race the framing asked to avoid, since the actor
/// could then run on a different core concurrently with this enqueue burst.
///
/// This does exercise `process_commands()`'s own loop rather than
/// `passivate()`'s — ADR-005 is explicit that the guard's drain is
/// "independent of the normal drain loop" and answers whatever is left in
/// the mailbox's `VecDeque` regardless of which in-body loop (if any) was
/// iterating over it at panic time, so the two loops are interchangeable
/// for what this test proves. Reaching `passivate()`'s specific drain loop
/// with pre-queued items from outside the crate would require racing the
/// real passivation timer against this test's own mailbox producer — a
/// second, *unrelated* scheduler race this test does not need in order to
/// prove the guard's loop-independent backstop.
#[tokio::test]
async fn panic_mid_processing_after_real_successes_answers_the_remainder() {
    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .build();

    let handler: Arc<
        dyn PersistentEntity<Command = ProbeCommand, Event = TestEvent, State = TestState>,
    > = Arc::new(PanicOnBoomHandler);

    let triple = EntityTriple::new("default".to_string(), "probe", "mid-drain-1");
    let aggregate_id = triple.aggregate_id();

    // Spawns the actor; no `.await` occurs in `entity_ref()` itself, so the
    // task is not yet polled by the time this call returns.
    let _entity_ref = runtime
        .entity_ref::<ProbeCommand, TestState>("probe", "mid-drain-1", handler)
        .unwrap();

    let erased = runtime
        .registry
        .lookup(&aggregate_id)
        .expect("fresh entry must exist immediately after entity_ref()");
    let mailbox = erased
        .downcast::<BoundedMailbox<ActorEnvelope<ProbeCommand>>>()
        .expect("must downcast to this test's own command type");

    let mut before_rxs = Vec::new();
    for _ in 0..2 {
        let (tx, rx) = oneshot::channel();
        mailbox
            .send(ActorEnvelope {
                envelope: CommandEnvelope {
                    command: ProbeCommand::Noop,
                    context: create_test_context(),
                },
                reply: tx,
            })
            .await
            .expect("mailbox is fresh and open");
        before_rxs.push(rx);
    }

    let (boom_tx, boom_rx) = oneshot::channel();
    mailbox
        .send(ActorEnvelope {
            envelope: CommandEnvelope {
                command: ProbeCommand::Boom,
                context: create_test_context(),
            },
            reply: boom_tx,
        })
        .await
        .expect("mailbox is fresh and open");

    let mut after_rxs = Vec::new();
    for _ in 0..2 {
        let (tx, rx) = oneshot::channel();
        mailbox
            .send(ActorEnvelope {
                envelope: CommandEnvelope {
                    command: ProbeCommand::Noop,
                    context: create_test_context(),
                },
                reply: tx,
            })
            .await
            .expect("mailbox is fresh and open");
        after_rxs.push(rx);
    }

    // Commands drained (and answered) before the panic must keep their real,
    // successful result — the guard's later drain must never touch them.
    for rx in before_rxs {
        let resolved = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("must not hang")
            .expect("sender must not drop silently without a value");
        let boxed = resolved.expect("a command drained before the panic must keep its real result");
        let result: CommandResult<TestEvent, TestState> = *boxed
            .downcast()
            .expect("result must downcast to CommandResult<TestEvent, TestState>");
        match result {
            CommandResult::Events { events, .. } => {
                assert_eq!(events, vec![TestEvent::incremented(1)]);
            }
            other => panic!("expected Events, got {other:?}"),
        }
    }

    let boom_result = tokio::time::timeout(Duration::from_secs(5), boom_rx)
        .await
        .expect("the in-flight panicking command's reply must not hang");
    assert!(
        boom_result.is_err(),
        "the in-flight command's reply sender must drop on panic-unwind"
    );

    for rx in after_rxs {
        let resolved = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("FR-009: the queued remainder must not hang");
        let terminal = resolved.expect("sender must not drop silently without a value");
        assert!(
            terminal.is_err(),
            "commands still queued behind the panic must resolve to a terminal Err via the guard"
        );
    }
}

// ---------------------------------------------------------------------------
// Scenario 3 / TASK-022 — runtime shutdown while Recovering (FR-009)
// ---------------------------------------------------------------------------

/// FR-009 — Scenario: runtime shutdown while `Recovering`.
///
/// Plain `#[test]`, not `#[tokio::test]`: `TokioEntityRef::new()` does not
/// expose a `JoinHandle` for the actor task it spawns, so the only
/// externally available way to force teardown before the actor is ever
/// polled is to spawn it onto a dedicated, disposable Tokio runtime and drop
/// that runtime outright — the "dropping the whole runtime" option. Per
/// Tokio's own documented `Runtime::drop` contract, tasks spawned via
/// `tokio::spawn` that have not completed are dropped (running destructors)
/// rather than left to finish.
///
/// `entity_ref()` performs no `.await` and the mailbox `send()` below
/// resolves without ever returning `Poll::Pending`, so on this dedicated
/// `current_thread` victim runtime the actor's freshly-spawned task is
/// *never polled* before the runtime is dropped — deterministically still in
/// its initial `Recovering` state (in fact one step earlier: recovery itself
/// never started), not a scheduler-dependent race. The caller's `oneshot`
/// receiver is kept on this test's own separate runtime, independent of the
/// victim runtime's lifetime, so it can be awaited after the drop.
#[test]
fn runtime_shutdown_while_recovering_answers_enqueued_caller() {
    let victim_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("victim runtime must build");

    let triple = EntityTriple::new("default".to_string(), "probe", "shutdown-recovering-1");
    let aggregate_id = triple.aggregate_id();

    let (registry, rx) = victim_rt.block_on(async {
        let runtime = EntityRuntimeBuilder::<TestEvent>::new()
            .passivation_timeout(Duration::from_secs(3600))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build();

        // Spawns the actor's task onto `victim_rt` but does not poll it.
        let _entity_ref = runtime
            .entity_ref::<TestCommand, TestState>("probe", "shutdown-recovering-1", handler())
            .unwrap();

        let erased = runtime
            .registry
            .lookup(&aggregate_id)
            .expect("fresh entry must exist immediately after entity_ref()");
        let mailbox = erased
            .downcast::<BoundedMailbox<ActorEnvelope<TestCommand>>>()
            .expect("must downcast to the real command type");

        let (tx, rx) = oneshot::channel();
        mailbox
            .send(ActorEnvelope {
                envelope: CommandEnvelope {
                    command: TestCommand::Increment(1),
                    context: create_test_context(),
                },
                reply: tx,
            })
            .await
            .expect("mailbox is fresh and open");

        (runtime.registry.clone(), rx)
    });

    // The actor's task was spawned but never polled: dropping the runtime
    // drops that never-started future, running the captured TeardownGuard's
    // Drop exactly as a genuine mid-Recovering shutdown would.
    drop(victim_rt);

    let checker_rt = tokio::runtime::Runtime::new().expect("checker runtime must build");
    // Constructed inside the async block (not as a bare argument expression)
    // so `tokio::time::timeout`'s internal timer registers against
    // `checker_rt`'s own ambient context rather than whatever (possibly
    // absent) context is active on the calling thread.
    let outcome =
        checker_rt.block_on(async { tokio::time::timeout(Duration::from_secs(5), rx).await });

    let resolved =
        outcome.expect("FR-009: the already-enqueued caller must not hang after runtime shutdown");
    let terminal = resolved.expect("reply sender must not drop silently without a value");
    assert!(
        terminal.is_err(),
        "a caller enqueued while Recovering, orphaned by a runtime shutdown, must resolve to a terminal Err"
    );
    assert!(
        registry.lookup(&aggregate_id).is_none(),
        "no zombie registry entry must remain after the runtime shutdown"
    );
}

// ---------------------------------------------------------------------------
// Scenario 4 / TASK-023 — 20-caller probe under a recovery-time panic
// ---------------------------------------------------------------------------

/// FR-009 + NFR-001/NFR-002 — Scenario: 20-caller probe under a
/// recovery-time panic. Multi-threaded (NFR-001): 20 concurrent callers race
/// `entity_ref()` + `send_command()` against one triple whose recovery
/// panics. Asserts exactly one activation attempt (actor-level
/// instrumentation of the real recovery call, not `active_count()`/ID-set
/// cardinality — NFR-002) and that every one of the 20 callers eventually
/// observes a terminal outcome.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn twenty_caller_probe_under_recovery_panic_resolves_all_and_activates_once() {
    let load_calls = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = tokio::sync::mpsc::channel::<()>(1);
    let event_store: Arc<AsyncMutex<dyn EventStore<TestEvent> + Send>> =
        Arc::new(AsyncMutex::new(PanicOnLoadEventStore {
            load_calls: load_calls.clone(),
            release_panic: Some(AsyncMutex::new(release_rx)),
        }));

    let runtime = Arc::new(
        EntityRuntimeBuilder::<TestEvent>::new()
            .passivation_timeout(Duration::from_secs(3600))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .with_event_store(event_store)
            .build(),
    );

    const N: usize = 20;
    let started = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let rt = runtime.clone();
        let h = handler();
        let started = started.clone();
        handles.push(tokio::spawn(async move {
            started.fetch_add(1, Ordering::SeqCst);
            let entity_ref = rt
                .entity_ref::<TestCommand, TestState>("probe", "recovery-panic-20", h)
                .unwrap();
            tokio::time::timeout(
                Duration::from_secs(10),
                entity_ref.send_command(
                    TestCommand::Increment((i + 1) as u64),
                    create_test_context(),
                ),
            )
            .await
            .expect("FR-009: every one of the 20 callers must eventually resolve, not hang")
        }));
    }

    // Wait until every one of the 20 caller tasks has begun executing —
    // bounded poll, not a blind sleep.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if started.load(Ordering::SeqCst) == N {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "all 20 caller tasks must have started within the deadline"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    // Gate the panic until every caller has actually enqueued its command
    // (see `PanicOnLoadEventStore`'s doc comment) — otherwise a straggler
    // racing past the teardown window could legitimately trigger its own
    // second activation attempt, inflating `load_calls` past 1 without any
    // actual bug (this is a test-determinism concern, not a production one).
    // Polling the real mailbox length is a genuine signal; a fixed
    // `yield_now()` budget is a guess that gets less reliable as N grows or
    // the machine is under load (see the 100-caller sibling test).
    let aggregate_id =
        EntityTriple::new("default".to_string(), "probe", "recovery-panic-20").aggregate_id();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let enqueued = runtime
            .registry
            .lookup(&aggregate_id)
            .and_then(|erased| {
                erased
                    .downcast::<BoundedMailbox<ActorEnvelope<TestCommand>>>()
                    .ok()
            })
            .map(|mailbox| mailbox.len())
            .unwrap_or(0);
        if enqueued == N {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "all {N} commands must be enqueued within the deadline, saw {enqueued}"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let _ = release_tx.send(()).await;

    let mut results: Vec<Result<CommandResult<TestEvent, TestState>, EntityError>> =
        Vec::with_capacity(N);
    for handle in handles {
        results.push(handle.await.expect("caller task must not itself panic"));
    }

    assert!(
        results.iter().all(|r| r.is_err()),
        "every one of the 20 callers must observe a terminal Err after the recovery-time panic"
    );
    assert_eq!(
        load_calls.load(Ordering::SeqCst),
        1,
        "NFR-002: exactly one activation attempt (spawn-level instrumentation of the real \
         recovery call) must have occurred for the 20 concurrent callers"
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if runtime.active_count() == 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the registry must end empty for this triple"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

// ---------------------------------------------------------------------------
// Scenario 5 / TASK-024 — poison + Round-3 deadlock regression
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct MismatchState {
    marker: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum MismatchCommand {
    Ping,
}

#[derive(Debug)]
struct MismatchHandler;

#[async_trait]
impl PersistentEntity for MismatchHandler {
    type Command = MismatchCommand;
    type Event = TestEvent;
    type State = MismatchState;

    fn initial_state(&self) -> MismatchState {
        MismatchState { marker: 0 }
    }

    async fn handle_command(
        &self,
        _command: &MismatchCommand,
        _state: &MismatchState,
        _context: &CommandContext,
    ) -> Result<Vec<TestEvent>, EntityError> {
        Ok(vec![])
    }

    async fn apply_event(
        &self,
        state: &MismatchState,
        _event: &TestEvent,
    ) -> Result<MismatchState, EntityError> {
        Ok(state.clone())
    }

    async fn apply_events(
        &self,
        state: &MismatchState,
        _events: &[TestEvent],
    ) -> Result<MismatchState, EntityError> {
        Ok(state.clone())
    }
}

/// Judgment Day CRITICAL 1 regression (ADR-002, FR-001's type-mismatch
/// scenario) — proven through the production entry point, concurrently, in
/// addition to `registry.rs`'s existing sequential unit test
/// (`live_entry_is_unaffected_by_a_mismatched_lookup`, TASK-006): a downcast
/// mismatch against a live entry must fail closed, never spawn a competitor,
/// and — critically for this regression — must never disturb or block any
/// OTHER triple's concurrent activation, since the registry's map lock is
/// process-wide, not per-key.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn downcast_mismatch_never_blocks_or_disturbs_other_triples() {
    let runtime = Arc::new(
        EntityRuntimeBuilder::<TestEvent>::new()
            .passivation_timeout(Duration::from_secs(3600))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .build(),
    );

    let original_ref = runtime
        .entity_ref::<TestCommand, TestState>("probe", "mismatch-1", handler())
        .unwrap();
    let first: CommandResult<TestEvent, TestState> = original_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await
        .unwrap();
    match first {
        CommandResult::Events { new_state, .. } => assert_eq!(new_state.value, 1),
        other => panic!("expected Events, got {other:?}"),
    }

    let mut mismatch_handles = Vec::with_capacity(20);
    for _ in 0..20 {
        let rt = runtime.clone();
        mismatch_handles.push(tokio::spawn(async move {
            rt.entity_ref::<MismatchCommand, MismatchState>(
                "probe",
                "mismatch-1",
                Arc::new(MismatchHandler),
            )
        }));
    }

    let mut other_handles = Vec::with_capacity(20);
    for i in 0..20 {
        let rt = runtime.clone();
        let h = handler();
        let entity_id = format!("mismatch-other-{i}");
        other_handles.push(tokio::spawn(async move {
            let r = rt
                .entity_ref::<TestCommand, TestState>("probe", entity_id, h)
                .unwrap();
            tokio::time::timeout(
                Duration::from_secs(5),
                r.send_command(TestCommand::Increment(1), create_test_context()),
            )
            .await
            .expect("a DIFFERENT triple's activation must never block on a concurrent mismatch")
        }));
    }

    // ADR-002: the downcast-mismatch branch's operative behavior in every
    // build is "return Err, never spawn a competitor" — but it is *also*
    // wrapped in `debug_assert!(false, ..)` specifically so debug/test
    // builds surface the bug loudly instead of silently. Under `cargo test`
    // (a debug build) that means each mismatched call's own task panics
    // rather than returning `Ok(Err(Internal(..)))`; the release-build-only
    // `Err` return is exercised at the registry level by `registry.rs`'s
    // `live_entry_is_unaffected_by_a_mismatched_lookup` (TASK-006), which
    // calls `lookup_or_insert` directly and never reaches this
    // `debug_assert`. What matters for THIS regression (never spawn a
    // competitor, never disturb or block anything else) holds either way.
    for handle in mismatch_handles {
        let joined = handle.await;
        assert!(
            joined.is_err(),
            "a downcast mismatch must fail closed (panicking loudly in this debug build via \
             debug_assert!, per ADR-002) rather than silently spawning a competing actor, got {joined:?}"
        );
    }
    for handle in other_handles {
        let outcome: Result<CommandResult<TestEvent, TestState>, EntityError> = handle
            .await
            .expect("other-triple task must not itself panic");
        assert!(
            outcome.is_ok(),
            "an unrelated triple must activate normally: {outcome:?}"
        );
    }

    let after: CommandResult<TestEvent, TestState> = original_ref
        .send_command(TestCommand::Increment(1), create_test_context())
        .await
        .unwrap();
    match after {
        CommandResult::Events { new_state, .. } => assert_eq!(
            new_state.value, 2,
            "original actor's state must be unaffected by concurrent mismatched lookups"
        ),
        other => panic!("expected Events, got {other:?}"),
    }
}

/// Judgment Day Round-3 CRITICAL regression (ADR-001's self-deadlock fix) —
/// `tokio::spawn` panicking outside a runtime context must never leave the
/// registry's single global map lock held. Ten threads with no Tokio
/// runtime call `entity_ref()` directly (a real precondition violation,
/// panicking inside `tokio::spawn`), concurrently with ten other threads
/// that each build their own runtime and activate a completely different
/// triple normally. Before ADR-001's Round-3 fix, holding the lock across
/// `tokio::spawn` meant a panic there self-deadlocked the *whole* registry —
/// every triple, not just the failing one — so this test would hang and its
/// timeouts would fire; against the fixed code, the "good" threads complete
/// promptly and every "bad" triple's zombie `Recovering` entry is cleaned up.
///
/// **Documented approximation.** This proves "no self-deadlock / no global
/// freeze regression," which is the operationally meaningful property. It
/// does not (and structurally cannot, from outside the crate) prove the map
/// lock is released at the exact nanosecond before `tokio::spawn` — that
/// would require instrumenting the lock itself, which is `pub(crate)`.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn spawn_outside_runtime_panic_never_blocks_other_triples() {
    let registry = Arc::new(EntityRegistry::new());
    let bad_ids: Vec<String> = (0..10)
        .map(|i| {
            EntityTriple::new("default".to_string(), "probe", format!("no-runtime-{i}"))
                .aggregate_id()
        })
        .collect();

    let mut bad_handles = Vec::with_capacity(10);
    for i in 0..10 {
        let registry = registry.clone();
        bad_handles.push(std::thread::spawn(move || {
            let runtime = EntityRuntimeBuilder::<TestEvent>::new()
                .with_registry(registry)
                .passivation_timeout(Duration::from_secs(3600))
                .snapshot_strategy(Arc::new(NoSnapshot))
                .build();
            let entity_id = format!("no-runtime-{i}");
            // No Tokio runtime exists on this OS thread: `tokio::spawn`
            // inside `entity_ref()` panics ("there is no reactor running").
            runtime.entity_ref::<TestCommand, TestState>("probe", entity_id, handler())
        }));
    }

    let mut good_handles = Vec::with_capacity(10);
    for i in 0..10 {
        let registry = registry.clone();
        good_handles.push(std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("good-thread runtime must build");
            rt.block_on(async move {
                let runtime = EntityRuntimeBuilder::<TestEvent>::new()
                    .with_registry(registry)
                    .passivation_timeout(Duration::from_secs(3600))
                    .snapshot_strategy(Arc::new(NoSnapshot))
                    .build();
                let entity_id = format!("with-runtime-{i}");
                let entity_ref = runtime
                    .entity_ref::<TestCommand, TestState>("probe", entity_id, handler())
                    .unwrap();
                tokio::time::timeout(
                    Duration::from_secs(5),
                    entity_ref.send_command(TestCommand::Increment(1), create_test_context()),
                )
                .await
                .expect(
                    "a normally-activated triple must never block on concurrent no-runtime panics elsewhere",
                )
            })
        }));
    }

    for (i, handle) in bad_handles.into_iter().enumerate() {
        let joined = handle.join();
        assert!(
            joined.is_err(),
            "bad thread {i} must have actually panicked (tokio::spawn outside a runtime)"
        );
    }
    for (i, handle) in good_handles.into_iter().enumerate() {
        let outcome: Result<CommandResult<TestEvent, TestState>, EntityError> =
            handle.join().expect("good thread must not itself panic");
        assert!(
            outcome.is_ok(),
            "good thread {i} must activate its triple normally: {outcome:?}"
        );
    }

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    for id in &bad_ids {
        loop {
            if registry.lookup(id).is_none() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "bad triple {id} must not remain as a zombie active entry"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario 6 — capstone: 100-caller probe, recovery panic, then a clean retry
// activates exactly once more (FR-001, FR-005, FR-009, FR-010, NFR-002)
// ---------------------------------------------------------------------------

/// `EventStore` whose `load()` blocks on a gate before its first call, panics
/// exactly once, then behaves like a normal empty store on every subsequent
/// call. Models "the triple's first activation attempt hits a real recovery
/// panic; a later, independent activation attempt for the same triple
/// recovers cleanly" — but the gate matters for a subtler reason: with 100
/// *genuinely* concurrent (`multi_thread`) callers, nothing stops a straggler
/// from calling `entity_ref()` *after* the panicking actor's `TeardownGuard`
/// has already removed the dead entry — that straggler would legitimately
/// find no live entry and spawn its own second, independently-successful
/// activation (FR-010 explicitly permits this; it just isn't the scenario
/// this test wants to isolate from the explicit retry below). The gate
/// blocks the panic itself until the test confirms all 100 callers have
/// already enqueued into the *original* mailbox, so none of them can race
/// past the teardown window on their own.
struct GatedPanicOnceEventStore {
    load_calls: Arc<AtomicUsize>,
    release_panic: AsyncMutex<tokio::sync::mpsc::Receiver<()>>,
}

#[async_trait::async_trait]
impl EventStore<TestEvent> for GatedPanicOnceEventStore {
    async fn append(
        &mut self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _expected_version: i64,
        _events: Vec<StoredEvent<TestEvent>>,
    ) -> Result<i64, PersistenceError> {
        Ok(0)
    }

    async fn load(
        &self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<TestEvent>>, PersistenceError> {
        let call_number = self.load_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call_number == 1 {
            // An explicit wait for the test's release signal — not a sleep, so
            // the gate opens when the test says so rather than after a guessed
            // interval.
            //
            // Awaited, not blocked on. This runs inside the store's own async
            // method, so yielding here returns the worker to the runtime and the
            // 100 caller tasks keep being serviced. A blocking receive would
            // instead park a worker for as long as the test holds the gate —
            // which is what this did while the store bridged async to sync, and
            // what `block_in_place` was compensating for.
            let _ = self.release_panic.lock().await.recv().await;
            panic!(
                "guaranteed_completion_tests: intentional panic on the FIRST recovery attempt only"
            );
        }
        Ok(Vec::new())
    }

    async fn list_aggregate_ids(
        &self,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        Ok(Vec::new())
    }
}

/// FR-001 + FR-005 + FR-009 + FR-010 + NFR-002 — Scenario: the capstone case
/// that exercises every mechanism in this change against one triple, in one
/// scenario, in this order:
///
/// 1. **Concurrent lookup** — 100 callers race `entity_ref()` + `send_command()`
///    against a triple whose recovery panics.
/// 2. **Single activation attempt** — the single-flight lock (ADR-001)
///    coalesces all 100 into exactly one `load()` call (NFR-002: asserted via
///    actor-level instrumentation, not `active_count()`).
/// 3. **Guaranteed completion** — every one of the 100 callers resolves to a
///    terminal `Err` (FR-009); none hangs.
/// 4. **Teardown + epoch + registry cleanup** — the `TeardownGuard` backstop
///    removes the dead entry; the registry ends empty for this triple before
///    the retry begins.
/// 5. **Explicit caller retry** — per ADR-008/FR-010, nothing in this crate
///    retries automatically; this test's own second `entity_ref()` call is
///    the retry. It reaches a brand-new epoch's actor, which recovers
///    cleanly this time (the event store only panics once) and answers with
///    a real, successful result — proving the triple is usable again, not
///    permanently wedged by the earlier panic.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn hundred_caller_probe_then_explicit_retry_activates_exactly_once_more() {
    let load_calls = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = tokio::sync::mpsc::channel::<()>(1);
    let event_store: Arc<AsyncMutex<dyn EventStore<TestEvent> + Send>> =
        Arc::new(AsyncMutex::new(GatedPanicOnceEventStore {
            load_calls: load_calls.clone(),
            release_panic: AsyncMutex::new(release_rx),
        }));

    let runtime = Arc::new(
        EntityRuntimeBuilder::<TestEvent>::new()
            .passivation_timeout(Duration::from_secs(3600))
            .snapshot_strategy(Arc::new(NoSnapshot))
            .with_event_store(event_store)
            .build(),
    );

    let triple = EntityTriple::new("default".to_string(), "probe", "hundred-caller-retry-1");
    let aggregate_id = triple.aggregate_id();

    const N: usize = 100;
    let started = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let rt = runtime.clone();
        let h = handler();
        let started = started.clone();
        handles.push(tokio::spawn(async move {
            started.fetch_add(1, Ordering::SeqCst);
            let entity_ref = rt
                .entity_ref::<TestCommand, TestState>("probe", "hundred-caller-retry-1", h)
                .unwrap();
            tokio::time::timeout(
                Duration::from_secs(10),
                entity_ref.send_command(
                    TestCommand::Increment((i + 1) as u64),
                    create_test_context(),
                ),
            )
            .await
            .expect("FR-009: every one of the 100 callers must eventually resolve, not hang")
        }));
    }

    // Wait until every one of the 100 caller tasks has begun executing —
    // bounded poll, not a blind sleep.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if started.load(Ordering::SeqCst) == N {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "all 100 caller tasks must have started within the deadline"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    // Wait until all 100 commands have actually been enqueued into the
    // mailbox — a real signal, not a fixed `yield_now()` budget guessing how
    // many scheduler turns 100 tasks need under unknown CPU contention. The
    // actor is blocked in recovery (gated on `release_panic`) and hasn't
    // reached `process_commands()` yet, so the mailbox only grows here; once
    // it hits N, every caller has genuinely enqueued and it's safe to
    // release the panic without a straggler racing past teardown into a
    // second, legitimate activation (which would inflate `load_calls` past 1
    // with no actual bug — a test-determinism concern, not a production one).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let enqueued = runtime
            .registry
            .lookup(&aggregate_id)
            .and_then(|erased| {
                erased
                    .downcast::<BoundedMailbox<ActorEnvelope<TestCommand>>>()
                    .ok()
            })
            .map(|mailbox| mailbox.len())
            .unwrap_or(0);
        if enqueued == N {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "all {N} commands must be enqueued within the deadline, saw {enqueued}"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    let _ = release_tx.send(()).await;

    let mut results: Vec<Result<CommandResult<TestEvent, TestState>, EntityError>> =
        Vec::with_capacity(N);
    for handle in handles {
        results.push(handle.await.expect("caller task must not itself panic"));
    }

    assert!(
        results.iter().all(|r| r.is_err()),
        "every one of the 100 callers must observe a terminal Err after the recovery-time panic"
    );
    assert_eq!(
        load_calls.load(Ordering::SeqCst),
        1,
        "NFR-002: the 100-caller burst must coalesce into exactly one activation attempt"
    );

    // Registry cleanup: the triple must end with no live entry before the
    // retry begins — the guard's backstop, not the (never-reached, since the
    // panic preempts it) in-body drain.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if runtime.registry.lookup(&aggregate_id).is_none() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the dead triple must not remain a zombie registry entry before the retry"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Explicit retry (ADR-008/FR-010: the caller retries; nothing in this
    // crate retries automatically). This reaches a fresh epoch's actor.
    let retry_ref = runtime
        .entity_ref::<TestCommand, TestState>("probe", "hundred-caller-retry-1", handler())
        .unwrap();
    let retry_result: CommandResult<TestEvent, TestState> = tokio::time::timeout(
        Duration::from_secs(5),
        retry_ref.send_command(TestCommand::Increment(1), create_test_context()),
    )
    .await
    .expect("the retry must not hang")
    .expect("the retry must reach a newly-activated, healthy actor and succeed");

    match retry_result {
        CommandResult::Events { new_state, .. } => assert_eq!(new_state.value, 1),
        other => panic!("expected Events, got {other:?}"),
    }
    assert_eq!(
        load_calls.load(Ordering::SeqCst),
        2,
        "the explicit retry must trigger exactly one NEW activation attempt — no more, no fewer"
    );
}
