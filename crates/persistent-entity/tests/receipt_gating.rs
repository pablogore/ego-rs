//! The idempotency gate in `EntityActor::execute_command`.
//!
//! These tests assert **interactions**, not only results. A test that checked
//! "the call returned `Replayed`" would pass just as happily if the handler had
//! run and the gate then overwrote its answer — which is the one failure that
//! matters here, because the handler is where side effects come from. So every
//! case counts receipt lookups, handler invocations, and writes.
//!
//! For the cases where the handler must not run at all, the entity is one that
//! **panics if invoked**. That is a structural proof rather than an assertion:
//! there is no arrangement of the gate that both falls through and passes.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ego_domain::context::TenantId;
use ego_domain::operation::{
    AggregateOutcome, OperationFingerprint, OperationIdentity, OperationKey, OperationReceipt,
};
use ego_domain::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::testing::{InMemoryEventStore, TestCommand, TestEvent, TestState};

const ENTITY_TYPE: &str = "gated";
const ENTITY_ID: &str = "e1";
const KEY: &str = "op-key-1";

/// What the store under test was asked to do.
#[derive(Default)]
struct Calls {
    lookups: AtomicUsize,
    begins: AtomicUsize,
    appends: AtomicUsize,
    loads: AtomicUsize,
}

impl Calls {
    fn lookups(&self) -> usize {
        self.lookups.load(Ordering::SeqCst)
    }
    fn begins(&self) -> usize {
        self.begins.load(Ordering::SeqCst)
    }
    fn appends(&self) -> usize {
        self.appends.load(Ordering::SeqCst)
    }
}

/// An event store whose receipt lookup is scripted, and which records every
/// call the actor makes on it.
struct GateStore {
    inner: InMemoryEventStore<TestEvent>,
    receipt: Option<OperationReceipt>,
    lookup_fails: bool,
    calls: Arc<Calls>,
}

impl GateStore {
    fn new(calls: Arc<Calls>) -> Self {
        Self {
            inner: InMemoryEventStore::new(),
            receipt: None,
            lookup_fails: false,
            calls,
        }
    }

    fn with_receipt(mut self, receipt: OperationReceipt) -> Self {
        self.receipt = Some(receipt);
        self
    }

    fn failing_lookup(mut self) -> Self {
        self.lookup_fails = true;
        self
    }
}

#[async_trait::async_trait]
impl EventStore<TestEvent> for GateStore {
    async fn append(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<TestEvent>>,
    ) -> Result<i64, PersistenceError> {
        self.calls.appends.fetch_add(1, Ordering::SeqCst);
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
        self.calls.loads.fetch_add(1, Ordering::SeqCst);
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
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        _operation_key: &str,
    ) -> Result<Option<OperationReceipt>, PersistenceError> {
        self.calls.lookups.fetch_add(1, Ordering::SeqCst);
        if self.lookup_fails {
            return Err(PersistenceError::Internal(
                "the receipt table is unreadable".to_string(),
            ));
        }
        Ok(self.receipt.clone())
    }

    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<TestEvent>>, PersistenceError> {
        self.calls.begins.fetch_add(1, Ordering::SeqCst);
        self.inner.begin().await
    }
}

/// An entity that records how often the command handler ran — and, when armed,
/// refuses to run at all.
#[derive(Debug)]
struct GatedEntity {
    handled: Arc<AtomicUsize>,
    panic_if_called: bool,
}

impl GatedEntity {
    fn counting(handled: Arc<AtomicUsize>) -> Self {
        Self {
            handled,
            panic_if_called: false,
        }
    }

    /// For every case where the gate must return before dispatch. A panic is a
    /// stronger claim than a counter: no fall-through can survive it.
    fn forbidden() -> Self {
        Self {
            handled: Arc::new(AtomicUsize::new(0)),
            panic_if_called: true,
        }
    }
}

#[async_trait::async_trait]
impl PersistentEntity for GatedEntity {
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
        if self.panic_if_called {
            panic!(
                "handle_command was invoked, but the gate should have answered before \
                 dispatch — a receipt hit, a conflicting fingerprint, and a failed lookup \
                 must never reach the handler"
            );
        }
        self.handled.fetch_add(1, Ordering::SeqCst);
        Ok(vec![TestEvent::Incremented(1)])
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

fn receipt(fingerprint: &str, outcome: AggregateOutcome) -> OperationReceipt {
    OperationReceipt::new(
        ENTITY_TYPE,
        ENTITY_ID,
        Some(TenantId::new("default").expect("a non-empty tenant id parses")),
        OperationKey::parse(KEY).expect("a non-empty key parses"),
        OperationFingerprint::new(fingerprint),
        outcome,
    )
}

/// A context carrying an operation identity, or none at all.
///
/// There is no third case. A key without a fingerprint, or the reverse, is not
/// constructible — see the compile-fail fixture
/// `operation_identity_half_constructed.rs`, which replaced the two runtime
/// tests that used to assert the gate ignored those halves.
fn context(identity: Option<(&str, &str)>) -> CommandContext {
    CommandContext::new(ENTITY_TYPE.to_string()).carrying(identity.map(|(key, fingerprint)| {
        OperationIdentity::new(
            OperationKey::parse(key).expect("a non-empty key parses"),
            OperationFingerprint::new(fingerprint),
        )
    }))
}

/// Drives one command through a real actor over the scripted store.
async fn send(
    store: GateStore,
    entity: GatedEntity,
    ctx: CommandContext,
) -> Result<CommandResult<TestEvent, TestState>, EntityError> {
    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_event_store(Arc::new(store))
        .build();

    let entity_ref = runtime
        .entity_ref::<TestCommand, TestState>(ENTITY_TYPE, ENTITY_ID, Arc::new(entity))
        .expect("an entity ref must be obtainable");

    entity_ref
        .send_command(TestCommand::Increment(1), ctx)
        .await
}

// --- The three shapes that must not consult a receipt at all ---------------
//
// Both halves of the identity are required. `operation_key` says which
// operation this is; `fingerprint` says which *request* it came from. With only
// one, a retry cannot be told apart from a different command reusing the key,
// so the honest answer is to leave the pre-existing path alone.

#[tokio::test]
async fn a_command_with_neither_key_nor_fingerprint_takes_the_previous_path() {
    let calls = Arc::new(Calls::default());
    let handled = Arc::new(AtomicUsize::new(0));

    let result = send(
        GateStore::new(calls.clone()),
        GatedEntity::counting(handled.clone()),
        context(None),
    )
    .await
    .expect("a non-idempotent command must still succeed");

    assert!(matches!(result, CommandResult::Events { .. }));
    assert_eq!(calls.lookups(), 0, "no identity, no lookup");
    assert_eq!(
        handled.load(Ordering::SeqCst),
        1,
        "the handler must run once"
    );
}

// --- Miss: the ordinary first execution ------------------------------------

#[tokio::test]
async fn a_miss_runs_the_command_exactly_once() {
    let calls = Arc::new(Calls::default());
    let handled = Arc::new(AtomicUsize::new(0));

    let result = send(
        GateStore::new(calls.clone()),
        GatedEntity::counting(handled.clone()),
        context(Some((KEY, "fp-a"))),
    )
    .await
    .expect("a first execution must succeed");

    assert!(matches!(result, CommandResult::Events { .. }));
    assert_eq!(calls.lookups(), 1, "the gate must consult exactly once");
    assert_eq!(
        handled.load(Ordering::SeqCst),
        1,
        "a miss is the ordinary first-execution case and must run the handler"
    );
}

// --- Hits: nothing runs, nothing is written --------------------------------

#[tokio::test]
async fn a_hit_on_a_no_events_receipt_replays_without_dispatching() {
    let calls = Arc::new(Calls::default());

    let result = send(
        GateStore::new(calls.clone()).with_receipt(receipt("fp-a", AggregateOutcome::NoEvents)),
        GatedEntity::forbidden(),
        context(Some((KEY, "fp-a"))),
    )
    .await
    .expect("a replay is a success, not an error");

    match result {
        CommandResult::Replayed { outcome } => assert_eq!(
            outcome,
            AggregateOutcome::NoEvents,
            "the replay must carry exactly the stored outcome"
        ),
        other => panic!("expected Replayed, got {other:?}"),
    }

    assert_eq!(calls.lookups(), 1);
    assert_eq!(calls.begins(), 0, "a replay must not open a unit of work");
    assert_eq!(calls.appends(), 0, "a replay must not persist anything");
}

#[tokio::test]
async fn a_hit_on_an_events_receipt_replays_without_dispatching() {
    let calls = Arc::new(Calls::default());
    let stored = AggregateOutcome::events(1, 3).expect("an ascending inclusive range is valid");

    let result = send(
        GateStore::new(calls.clone()).with_receipt(receipt("fp-a", stored.clone())),
        GatedEntity::forbidden(),
        context(Some((KEY, "fp-a"))),
    )
    .await
    .expect("a replay is a success, not an error");

    match result {
        CommandResult::Replayed { outcome } => assert_eq!(
            outcome, stored,
            "the replay must carry exactly the stored range, not a recomputed one"
        ),
        other => panic!("expected Replayed, got {other:?}"),
    }

    assert_eq!(calls.lookups(), 1);
    assert_eq!(calls.begins(), 0, "a replay must not open a unit of work");
    assert_eq!(calls.appends(), 0, "a replay must not persist anything");
}

// --- Conflict: a different request wearing the same key --------------------

#[tokio::test]
async fn a_conflicting_fingerprint_is_a_permanent_operation_conflict() {
    let calls = Arc::new(Calls::default());

    let error = send(
        GateStore::new(calls.clone()).with_receipt(receipt("fp-a", AggregateOutcome::NoEvents)),
        GatedEntity::forbidden(),
        context(Some((KEY, "fp-b"))),
    )
    .await
    .expect_err("a different request reusing an operation key must be refused");

    match &error {
        EntityError::OperationConflict { operation_key } => {
            assert_eq!(
                operation_key, KEY,
                "the error must name the conflicting key so a caller can act on it"
            );
        }
        other => panic!("expected OperationConflict, got {other:?}"),
    }

    // A version conflict is transient and invites a reload-and-retry. This one
    // never resolves, so reporting it as one would send the caller into a loop
    // that cannot terminate.
    assert!(
        !matches!(error, EntityError::VersionConflict { .. }),
        "an idempotency conflict must never be reported as stream concurrency"
    );

    let rendered = error.to_string();
    assert!(
        !rendered.contains("fp-a") && !rendered.contains("fp-b"),
        "the error must not leak fingerprints: they are request digests, and the \
         caller's own key is the only part it can act on — got {rendered:?}"
    );

    assert_eq!(calls.lookups(), 1);
    assert_eq!(calls.begins(), 0);
    assert_eq!(calls.appends(), 0);
}

// --- A failed lookup is not a miss -----------------------------------------

#[tokio::test]
async fn a_failed_lookup_is_reported_and_never_falls_through_to_the_handler() {
    let calls = Arc::new(Calls::default());

    let error = send(
        GateStore::new(calls.clone()).failing_lookup(),
        GatedEntity::forbidden(),
        context(Some((KEY, "fp-a"))),
    )
    .await
    .expect_err("an unreadable receipt table must surface as an error");

    assert!(
        matches!(error, EntityError::PersistenceError(_)),
        "expected a persistence error, got {error:?}"
    );

    // The entity would have panicked, but state the intent as well: a miss means
    // "run the command", so softening a read failure into a miss would
    // re-execute an operation that may already have completed.
    assert_eq!(calls.lookups(), 1);
    assert_eq!(calls.begins(), 0);
    assert_eq!(calls.appends(), 0);
}
