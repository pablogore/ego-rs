//! B5.6/B5.7 — the success path writes the receipt in the same unit of work as
//! the events, and a later retry finds it.
//!
//! `receipt_gating.rs` proved the actor *reads* a receipt correctly. Nothing
//! there wrote one, so a fresh command still left no evidence behind and the
//! guarantee was half-built by design. These tests close it, and the decisive
//! one is the full cycle: execute once, retry with the same identity, and the
//! handler must have run **exactly once**.
//!
//! The store double records the order of calls, not only their number. "Both
//! happened" is not the property — `commit` landing between the append and the
//! confirmation would satisfy every counter while making the events durable
//! ahead of the record that says they happened.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ego_domain::operation::{
    AggregateOutcome, OperationFingerprint, OperationKey, OperationReceipt,
};
use ego_domain::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::testing::{InMemoryEventStore, TestCommand, TestEvent, TestState};

const ENTITY_TYPE: &str = "written";
const ENTITY_ID: &str = "w1";
const KEY: &str = "op-write-1";
const FP: &str = "fp-a";

/// Where a step must fail, for the rollback cases.
#[derive(Clone, Copy, PartialEq)]
enum FailAt {
    Nothing,
    Append,
    Confirm,
    Commit,
}

/// Every call the actor made, in order.
type Trace = Arc<Mutex<Vec<&'static str>>>;

fn note(trace: &Trace, step: &'static str) {
    trace
        .lock()
        .expect("the trace lock is never poisoned")
        .push(step);
}

fn steps(trace: &Trace) -> Vec<&'static str> {
    trace
        .lock()
        .expect("the trace lock is never poisoned")
        .clone()
}

fn count(trace: &Trace, step: &str) -> usize {
    steps(trace).iter().filter(|s| **s == step).count()
}

/// A real in-memory store, wrapped so every call is recorded and any single step
/// can be made to fail.
struct RecordingStore {
    inner: InMemoryEventStore<TestEvent>,
    trace: Trace,
    fail_at: FailAt,
}

impl RecordingStore {
    fn new(trace: Trace, fail_at: FailAt) -> Self {
        Self {
            inner: InMemoryEventStore::new(),
            trace,
            fail_at,
        }
    }
}

#[async_trait::async_trait]
impl EventStore<TestEvent> for RecordingStore {
    async fn append(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<TestEvent>>,
    ) -> Result<i64, PersistenceError> {
        note(&self.trace, "direct_append");
        self.inner
            .append(
                aggregate_type,
                aggregate_id,
                tenant_id,
                expected_version,
                events,
            )
            .await
    }

    async fn load(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<TestEvent>>, PersistenceError> {
        self.inner
            .load(aggregate_type, aggregate_id, tenant_id)
            .await
    }

    async fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        self.inner.list_aggregate_ids(tenant_id).await
    }

    async fn find_receipt(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        operation_key: &str,
    ) -> Result<Option<OperationReceipt>, PersistenceError> {
        note(&self.trace, "find_receipt");
        self.inner
            .find_receipt(aggregate_type, aggregate_id, tenant_id, operation_key)
            .await
    }

    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<TestEvent>>, PersistenceError> {
        note(&self.trace, "begin");
        let inner = self.inner.begin().await?;
        Ok(Box::new(RecordingUow {
            inner,
            trace: Arc::clone(&self.trace),
            fail_at: self.fail_at,
        }))
    }
}

struct RecordingUow {
    inner: Box<dyn EventStoreUnitOfWork<TestEvent>>,
    trace: Trace,
    fail_at: FailAt,
}

#[async_trait::async_trait]
impl EventStoreUnitOfWork<TestEvent> for RecordingUow {
    async fn append(
        &mut self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<TestEvent>>,
    ) -> Result<i64, PersistenceError> {
        note(&self.trace, "append");
        if self.fail_at == FailAt::Append {
            return Err(PersistenceError::Internal("append refused".to_string()));
        }
        self.inner
            .append(
                aggregate_type,
                aggregate_id,
                tenant_id,
                expected_version,
                events,
            )
            .await
    }

    async fn confirm_receipt(
        &mut self,
        receipt: &OperationReceipt,
    ) -> Result<(), PersistenceError> {
        note(&self.trace, "confirm");
        if self.fail_at == FailAt::Confirm {
            return Err(PersistenceError::Internal("confirm refused".to_string()));
        }
        self.inner.confirm_receipt(receipt).await
    }

    async fn commit(self: Box<Self>) -> Result<(), PersistenceError> {
        note(&self.trace, "commit");
        if self.fail_at == FailAt::Commit {
            return Err(PersistenceError::Internal("commit refused".to_string()));
        }
        self.inner.commit().await
    }
}

/// Counts handler invocations; refuses to run when a replay is expected.
#[derive(Debug)]
struct Recorded {
    handled: Arc<AtomicUsize>,
    emits: usize,
    forbidden: bool,
}

impl Recorded {
    fn emitting(handled: Arc<AtomicUsize>, emits: usize) -> Self {
        Self {
            handled,
            emits,
            forbidden: false,
        }
    }
}

#[async_trait::async_trait]
impl PersistentEntity for Recorded {
    type Command = TestCommand;
    type Event = TestEvent;
    type State = TestState;

    fn initial_state(&self) -> Self::State {
        TestState {
            value: 0,
            version: 0,
        }
    }

    async fn handle_command(
        &self,
        _command: &Self::Command,
        _state: &Self::State,
        _context: &CommandContext,
    ) -> Result<Vec<Self::Event>, EntityError> {
        if self.forbidden {
            panic!("handle_command ran on a retry: the receipt written by the first execution was not found");
        }
        self.handled.fetch_add(1, Ordering::SeqCst);
        Ok((0..self.emits).map(|_| TestEvent::Incremented(1)).collect())
    }

    async fn apply_event(
        &self,
        state: &Self::State,
        event: &Self::Event,
    ) -> Result<Self::State, EntityError> {
        let mut next = state.clone();
        if let TestEvent::Incremented(v) = event {
            next.value += v;
            next.version += 1;
        }
        Ok(next)
    }

    async fn apply_events(
        &self,
        state: &Self::State,
        events: &[Self::Event],
    ) -> Result<Self::State, EntityError> {
        let mut next = state.clone();
        for event in events {
            next = self.apply_event(&next, event).await?;
        }
        Ok(next)
    }
}

fn context(key: Option<&str>, fingerprint: Option<&str>) -> CommandContext {
    let mut ctx = CommandContext::new(ENTITY_TYPE.to_string());
    ctx.operation_key = key.map(|k| OperationKey::parse(k).expect("a non-empty key parses"));
    ctx.fingerprint = fingerprint.map(OperationFingerprint::new);
    ctx
}

/// One runtime over one store, so two commands in a row hit the same state —
/// which is the whole point of the cycle tests.
fn runtime_over(store: RecordingStore) -> persistent_entity::runtime::EntityRuntime<TestEvent> {
    EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_event_store(Arc::new(store))
        .build()
}

async fn send(
    runtime: &persistent_entity::runtime::EntityRuntime<TestEvent>,
    entity: Recorded,
    ctx: CommandContext,
) -> Result<CommandResult<TestEvent, TestState>, EntityError> {
    let entity_ref = runtime
        .entity_ref::<TestCommand, TestState>(ENTITY_TYPE, ENTITY_ID, Arc::new(entity))
        .expect("an entity ref must be obtainable");
    entity_ref
        .send_command(TestCommand::Increment(1), ctx)
        .await
}

// --- The decisive test: execute once, retry, handler ran exactly once ------

#[tokio::test]
async fn events_are_written_with_their_receipt_and_a_retry_replays() {
    let trace: Trace = Arc::new(Mutex::new(Vec::new()));
    let handled = Arc::new(AtomicUsize::new(0));
    let runtime = runtime_over(RecordingStore::new(Arc::clone(&trace), FailAt::Nothing));

    let first = send(
        &runtime,
        Recorded::emitting(Arc::clone(&handled), 2),
        context(Some(KEY), Some(FP)),
    )
    .await
    .expect("a first execution must succeed");
    assert!(matches!(first, CommandResult::Events { .. }));

    // Order, not just presence. A commit landing between the append and the
    // confirmation satisfies every counter while making the events durable ahead
    // of the record that says they happened.
    assert_eq!(
        steps(&trace),
        vec!["find_receipt", "begin", "append", "confirm", "commit"],
        "the success path must look up, then open one unit of work and append, \
         confirm and commit inside it, in that order"
    );
    assert_eq!(
        count(&trace, "direct_append"),
        0,
        "the direct append path must not be used when a receipt is written"
    );

    let second = send(
        &runtime,
        Recorded {
            handled: Arc::clone(&handled),
            emits: 2,
            forbidden: true,
        },
        context(Some(KEY), Some(FP)),
    )
    .await
    .expect("a retry must succeed as a replay");

    match second {
        CommandResult::Replayed { outcome } => assert_eq!(
            outcome,
            AggregateOutcome::events(1, 2).expect("an ascending inclusive range is valid"),
            "the replay must report the inclusive range the first execution wrote"
        ),
        other => panic!("expected Replayed, got {other:?}"),
    }

    assert_eq!(
        handled.load(Ordering::SeqCst),
        1,
        "the handler must have run exactly once across both commands"
    );
}

#[tokio::test]
async fn a_zero_event_success_is_written_through_a_real_unit_of_work_and_replays() {
    let trace: Trace = Arc::new(Mutex::new(Vec::new()));
    let handled = Arc::new(AtomicUsize::new(0));
    let runtime = runtime_over(RecordingStore::new(Arc::clone(&trace), FailAt::Nothing));

    let first = send(
        &runtime,
        Recorded::emitting(Arc::clone(&handled), 0),
        context(Some(KEY), Some(FP)),
    )
    .await
    .expect("a zero-event success must succeed");
    assert!(matches!(first, CommandResult::NoEvents { .. }));

    assert_eq!(
        steps(&trace),
        vec!["find_receipt", "begin", "confirm", "commit"],
        "a zero-event success must still open a real unit of work and commit its \
         receipt — with no append, because there is nothing to append"
    );

    let second = send(
        &runtime,
        Recorded {
            handled: Arc::clone(&handled),
            emits: 0,
            forbidden: true,
        },
        context(Some(KEY), Some(FP)),
    )
    .await
    .expect("a retry must succeed as a replay");

    match second {
        CommandResult::Replayed { outcome } => assert_eq!(
            outcome,
            AggregateOutcome::NoEvents,
            "a replayed zero-event success reports NoEvents"
        ),
        other => panic!("expected Replayed, got {other:?}"),
    }

    assert_eq!(
        handled.load(Ordering::SeqCst),
        1,
        "the handler must have run exactly once: without a durable receipt, this \
         is the branch that silently re-executes, because no event exists to \
         suggest anything happened"
    );
}

// --- No identity: the previous path, untouched ----------------------------

#[tokio::test]
async fn a_command_without_the_identity_keeps_the_direct_path() {
    let trace: Trace = Arc::new(Mutex::new(Vec::new()));
    let handled = Arc::new(AtomicUsize::new(0));
    let runtime = runtime_over(RecordingStore::new(Arc::clone(&trace), FailAt::Nothing));

    send(
        &runtime,
        Recorded::emitting(Arc::clone(&handled), 1),
        context(None, None),
    )
    .await
    .expect("a non-idempotent command must still succeed");

    assert_eq!(
        count(&trace, "begin"),
        0,
        "a command with no receipt to write must not open a unit of work it does \
         not need, and must keep depending on the same store method as before"
    );
    assert_eq!(count(&trace, "direct_append"), 1);
    assert_eq!(count(&trace, "find_receipt"), 0);
}

// --- Failures: neither the events nor the receipt become visible -----------

async fn nothing_is_visible_after(fail_at: FailAt, expected_tail: &[&str]) {
    let trace: Trace = Arc::new(Mutex::new(Vec::new()));
    let handled = Arc::new(AtomicUsize::new(0));

    // The store is held here, not handed away, so the assertions below read the
    // very store the runtime writes through. An earlier version of this test
    // created a second, unrelated `InMemoryEventStore` and claimed in a comment
    // to be reading the runtime's — which made the whole rollback check
    // vacuous.
    let store = Arc::new(RecordingStore::new(Arc::clone(&trace), fail_at));
    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_event_store(Arc::clone(&store) as Arc<dyn EventStore<TestEvent> + Send + Sync>)
        .build();

    let outcome = send(
        &runtime,
        Recorded::emitting(Arc::clone(&handled), 2),
        context(Some(KEY), Some(FP)),
    )
    .await;
    assert!(outcome.is_err(), "a failing step must surface as an error");

    assert_eq!(
        steps(&trace),
        expected_tail,
        "the sequence must stop at the failing step: nothing after it may run"
    );

    // Both halves of the rollback, read directly from the store the runtime
    // used. Asserting only the receipt's absence would pass against an
    // implementation that made the events durable and lost the receipt — the
    // retry would find no receipt, re-execute, and look correct while the
    // aggregate had silently advanced.
    let events = store
        .load(ENTITY_TYPE, ENTITY_ID, Some("default"))
        .await
        .unwrap_or_default();
    assert!(
        events.is_empty(),
        "a failed step must leave no event durable, and {} were found",
        events.len()
    );

    let key = OperationKey::parse(KEY).expect("a non-empty key parses");
    assert_eq!(
        store
            .find_receipt(ENTITY_TYPE, ENTITY_ID, Some("default"), key.as_str())
            .await
            .expect("a receipt lookup must not fail"),
        None,
        "a failed step must leave no receipt durable either"
    );
}

#[tokio::test]
async fn a_failed_append_leaves_neither_events_nor_receipt() {
    nothing_is_visible_after(FailAt::Append, &["find_receipt", "begin", "append"]).await;
}

#[tokio::test]
async fn a_failed_confirmation_leaves_neither_events_nor_receipt() {
    nothing_is_visible_after(
        FailAt::Confirm,
        &["find_receipt", "begin", "append", "confirm"],
    )
    .await;
}

#[tokio::test]
async fn a_failed_commit_leaves_neither_events_nor_receipt() {
    nothing_is_visible_after(
        FailAt::Commit,
        &["find_receipt", "begin", "append", "confirm", "commit"],
    )
    .await;
}
