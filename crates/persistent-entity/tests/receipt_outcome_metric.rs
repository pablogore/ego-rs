//! `idempotency.receipt.outcome` — the counter AD-10 specifies for what the
//! receipt gate decided.
//!
//! One metric name, three values of an `outcome` attribute, and an
//! `aggregate_type` taken from the entity's registered identity rather than from
//! anything a caller supplied. The name is asserted literally at every call site
//! on purpose: a folded name would still satisfy "a counter was emitted", and
//! folding is precisely what AD-10's table forbids and what the rest of this
//! campaign is migrating away from.
//!
//! The `confirmed` cases assert **ordering as well as occurrence**. A counter
//! incremented before the write would report operations that never landed, so
//! each of those tests pairs its assertion with a negative control in which
//! persistence fails and nothing may be counted.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ego_domain::context::TenantId;
use ego_domain::operation::{
    AggregateOutcome, OperationFingerprint, OperationIdentity, OperationKey, OperationReceipt,
};
use ego_domain::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};
use ego_domain::{Level, MetricKind, MetricObservation, Observability, SemanticEvent};
use ego_testkit::RecordedMetric;
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::snapshot::NoSnapshot;
use persistent_entity::testing::{InMemoryEventStore, TestCommand, TestEvent, TestState};

const METRIC: &str = "idempotency.receipt.outcome";
const ENTITY_TYPE: &str = "invoice";
const OTHER_ENTITY_TYPE: &str = "shipment";
const ENTITY_ID: &str = "e1";

/// Deliberately shaped like something a client would really send, and unique
/// enough that a substring scan cannot match it by accident.
const RAW_KEY: &str = "customer-4417-invoice-2026-03";

// --- The recording double ---------------------------------------------------

/// Records **every** observation — metrics, semantic events, and log lines.
///
/// It keeps the traces and logs it has no assertion of its own for, because the
/// redaction test scans all three. Checking only the metric would leave the raw
/// key free to escape through an event this slice also emits, which is exactly
/// the kind of leak that is invisible until someone reads a production trace.
#[derive(Default)]
struct RecordingObservability {
    metrics: Mutex<Vec<RecordedMetric>>,
    events: Mutex<Vec<SemanticEvent>>,
    logs: Mutex<Vec<String>>,
}

impl RecordingObservability {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Only the observations of the metric under test, in arrival order.
    fn receipt_outcomes(&self) -> Vec<RecordedMetric> {
        self.metrics
            .lock()
            .expect("not poisoned")
            .iter()
            .filter(|m| m.name == METRIC)
            .cloned()
            .collect()
    }

    /// Every metric name recorded, so a folded name shows up as a name rather
    /// than being silently filtered out by `receipt_outcomes`.
    fn metric_names(&self) -> Vec<String> {
        self.metrics
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|m| m.name.clone())
            .collect()
    }

    /// Every piece of text this double ever received, whatever the channel.
    ///
    /// Rendered rather than inspected field by field: the claim is that the raw
    /// key appears **nowhere**, and a scan over the rendering is what makes that
    /// claim total instead of a list of the places someone thought to check.
    fn all_observed_text(&self) -> String {
        let metrics = self.metrics.lock().expect("not poisoned");
        let events = self.events.lock().expect("not poisoned");
        let logs = self.logs.lock().expect("not poisoned");
        format!("{metrics:?}|{events:?}|{logs:?}")
    }
}

impl Observability for RecordingObservability {
    fn trace(&self, event: SemanticEvent) {
        self.events.lock().expect("not poisoned").push(event);
    }

    fn record_metric(&self, observation: MetricObservation<'_>) {
        self.metrics
            .lock()
            .expect("not poisoned")
            .push(RecordedMetric::capture(&observation));
    }

    fn log(&self, _level: Level, message: &str) {
        self.logs
            .lock()
            .expect("not poisoned")
            .push(message.to_string());
    }
}

/// The one observation this metric must produce, spelled out in full.
///
/// Built rather than asserted field by field so every test compares the whole
/// record: a test that checked only `outcome` would pass with the name folded,
/// the kind flattened, or `aggregate_type` dropped.
fn expected(outcome: &str, aggregate_type: &str) -> RecordedMetric {
    RecordedMetric {
        kind: MetricKind::Counter,
        name: METRIC.to_string(),
        value: 1.0,
        attributes: vec![
            ("outcome".to_string(), outcome.to_string()),
            ("aggregate_type".to_string(), aggregate_type.to_string()),
        ],
    }
}

// --- The scripted store -----------------------------------------------------

#[derive(Default)]
struct Calls {
    appends: AtomicUsize,
}

/// An event store whose receipt lookup is scripted and whose write path can be
/// made to fail.
struct OutcomeStore {
    inner: InMemoryEventStore<TestEvent>,
    receipt: Option<OperationReceipt>,
    write_fails: bool,
    calls: Arc<Calls>,
}

impl OutcomeStore {
    fn new() -> Self {
        Self {
            inner: InMemoryEventStore::new(),
            receipt: None,
            write_fails: false,
            calls: Arc::new(Calls::default()),
        }
    }

    fn with_receipt(mut self, receipt: OperationReceipt) -> Self {
        self.receipt = Some(receipt);
        self
    }

    /// Refuses to open a unit of work, so nothing this command produced can be
    /// persisted and no receipt can be confirmed.
    fn failing_writes(mut self) -> Self {
        self.write_fails = true;
        self
    }
}

#[async_trait::async_trait]
impl EventStore<TestEvent> for OutcomeStore {
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
        Ok(self.receipt.clone())
    }

    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<TestEvent>>, PersistenceError> {
        if self.write_fails {
            return Err(PersistenceError::Internal(
                "the unit of work could not be opened".to_string(),
            ));
        }
        self.inner.begin().await
    }
}

// --- The entity -------------------------------------------------------------

/// Emits one event, or none at all — the two shapes that reach the two distinct
/// receipt-confirming call sites in the actor.
#[derive(Debug)]
struct OutcomeEntity {
    emits_events: bool,
}

impl OutcomeEntity {
    fn emitting() -> Self {
        Self { emits_events: true }
    }

    fn silent() -> Self {
        Self {
            emits_events: false,
        }
    }
}

#[async_trait::async_trait]
impl PersistentEntity for OutcomeEntity {
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
        if self.emits_events {
            Ok(vec![TestEvent::Incremented(1)])
        } else {
            Ok(vec![])
        }
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

// --- Drivers ----------------------------------------------------------------

fn receipt_for(entity_type: &str, fingerprint: &str) -> OperationReceipt {
    OperationReceipt::new(
        entity_type,
        ENTITY_ID,
        Some(TenantId::new("default").expect("a non-empty tenant id parses")),
        OperationKey::parse(RAW_KEY).expect("a non-empty key parses"),
        OperationFingerprint::new(fingerprint),
        AggregateOutcome::NoEvents,
    )
}

/// A command carrying no operation identity at all — the pre-existing path,
/// which writes no receipt and therefore has no outcome to report.
fn context_without_identity() -> CommandContext {
    CommandContext::new(ENTITY_TYPE.to_string()).carrying(None)
}

fn context(fingerprint: &str) -> CommandContext {
    CommandContext::new(ENTITY_TYPE.to_string()).carrying(Some(OperationIdentity::new(
        OperationKey::parse(RAW_KEY).expect("a non-empty key parses"),
        OperationFingerprint::new(fingerprint),
    )))
}

/// Drives one command through a real actor, with observability wired in.
async fn send_observed(
    entity_type: &'static str,
    store: OutcomeStore,
    entity: OutcomeEntity,
    ctx: CommandContext,
    obs: Arc<RecordingObservability>,
) -> Result<CommandResult<TestEvent, TestState>, EntityError> {
    let runtime = EntityRuntimeBuilder::<TestEvent>::new()
        .passivation_timeout(Duration::from_secs(3600))
        .snapshot_strategy(Arc::new(NoSnapshot))
        .with_event_store(Arc::new(store))
        .with_observability(obs)
        .build();

    let entity_ref = runtime
        .entity_ref::<TestCommand, TestState>(entity_type, ENTITY_ID, Arc::new(entity))
        .expect("an entity ref must be obtainable");

    entity_ref
        .send_command(TestCommand::Increment(1), ctx)
        .await
}

/// The same drive with no observability at all.
async fn send_unobserved(
    store: OutcomeStore,
    entity: OutcomeEntity,
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

// --- The three outcomes -----------------------------------------------------

/// A receipt whose fingerprint matches: the same request arriving again.
#[tokio::test]
async fn a_replayed_request_counts_already_applied() {
    let obs = RecordingObservability::new();

    let result = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new().with_receipt(receipt_for(ENTITY_TYPE, "fp-1")),
        OutcomeEntity::emitting(),
        context("fp-1"),
        obs.clone(),
    )
    .await;

    assert!(
        matches!(result, Ok(CommandResult::Replayed { .. })),
        "the gate must answer from the receipt"
    );
    assert_eq!(
        obs.receipt_outcomes(),
        vec![expected("already_applied", ENTITY_TYPE)],
        "a replay is one `already_applied` observation carrying the entity's own type"
    );
}

/// A receipt whose fingerprint differs: a different request reusing the key.
#[tokio::test]
async fn a_conflicting_fingerprint_counts_conflict() {
    let obs = RecordingObservability::new();

    let result = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new().with_receipt(receipt_for(ENTITY_TYPE, "fp-stored")),
        OutcomeEntity::emitting(),
        context("fp-different"),
        obs.clone(),
    )
    .await;

    assert!(
        matches!(result, Err(EntityError::OperationConflict { .. })),
        "a reused key with a different request is refused"
    );
    assert_eq!(
        obs.receipt_outcomes(),
        vec![expected("conflict", ENTITY_TYPE)],
        "a conflict is counted as `conflict`, never collapsed into `already_applied`"
    );
}

/// The first confirming call site: a command that produced events.
#[tokio::test]
async fn a_fresh_request_producing_events_counts_confirmed() {
    let obs = RecordingObservability::new();

    let result = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new(),
        OutcomeEntity::emitting(),
        context("fp-1"),
        obs.clone(),
    )
    .await;

    assert!(result.is_ok(), "the command must succeed: {result:?}");
    assert_eq!(
        obs.receipt_outcomes(),
        vec![expected("confirmed", ENTITY_TYPE)],
        "a confirmed receipt on the events path is counted once"
    );
}

/// The second confirming call site: a command that produced none.
///
/// This is the case the receipt exists for — with no event in the stream, the
/// receipt is the only evidence the command ran — so it has its own write path
/// in the actor and its own test here. One test covering only the events path
/// would leave this one uncounted and look complete.
#[tokio::test]
async fn a_fresh_request_producing_no_events_counts_confirmed() {
    let obs = RecordingObservability::new();

    let result = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new(),
        OutcomeEntity::silent(),
        context("fp-1"),
        obs.clone(),
    )
    .await;

    assert!(
        matches!(result, Ok(CommandResult::NoEvents { .. })),
        "the command must succeed having emitted nothing: {result:?}"
    );
    assert_eq!(
        obs.receipt_outcomes(),
        vec![expected("confirmed", ENTITY_TYPE)],
        "the no-events path confirms a receipt too, and must count it"
    );
}

// --- The properties the values alone do not establish -----------------------

/// Two entity types produce two observations of **one** series.
///
/// This is the property folding `aggregate_type` into the name would destroy:
/// the dimension varies, the name does not, and nothing downstream can aggregate
/// across types unless that holds.
#[tokio::test]
async fn two_aggregate_types_share_one_metric_name() {
    let obs = RecordingObservability::new();

    for entity_type in [ENTITY_TYPE, OTHER_ENTITY_TYPE] {
        let _ = send_observed(
            entity_type,
            OutcomeStore::new().with_receipt(receipt_for(entity_type, "fp-1")),
            OutcomeEntity::emitting(),
            context("fp-1"),
            obs.clone(),
        )
        .await;
    }

    assert_eq!(
        obs.receipt_outcomes(),
        vec![
            expected("already_applied", ENTITY_TYPE),
            expected("already_applied", OTHER_ENTITY_TYPE),
        ],
        "the aggregate type varies as an attribute; the series name does not"
    );
    assert_eq!(
        obs.metric_names(),
        vec![METRIC.to_string(), METRIC.to_string()],
        "no name may carry the type: {:?}",
        obs.metric_names()
    );
}

/// The negative control: a write that fails counts nothing.
///
/// Without this, a counter incremented before the write would pass every test
/// above. `confirmed` must mean the receipt is durable, and the only way to
/// assert that is to break the write and require silence.
#[tokio::test]
async fn a_failed_write_counts_no_confirmation() {
    let obs = RecordingObservability::new();

    let result = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new().failing_writes(),
        OutcomeEntity::emitting(),
        context("fp-1"),
        obs.clone(),
    )
    .await;

    assert!(
        result.is_err(),
        "the command must fail when its receipt cannot be written: {result:?}"
    );
    assert_eq!(
        obs.receipt_outcomes(),
        Vec::new(),
        "nothing was persisted, so nothing may be counted as confirmed: {:?}",
        obs.receipt_outcomes()
    );
}

/// The same, for the no-events path — the one whose only evidence is the receipt.
#[tokio::test]
async fn a_failed_write_on_the_no_events_path_counts_no_confirmation() {
    let obs = RecordingObservability::new();

    let result = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new().failing_writes(),
        OutcomeEntity::silent(),
        context("fp-1"),
        obs.clone(),
    )
    .await;

    assert!(
        result.is_err(),
        "the command must fail when its receipt cannot be written: {result:?}"
    );
    assert_eq!(
        obs.receipt_outcomes(),
        Vec::new(),
        "no receipt was written, so no confirmation may be counted: {:?}",
        obs.receipt_outcomes()
    );
}

/// A command with no operation identity is counted nowhere.
///
/// This is the guard on the events path, and it is not covered by any test that
/// supplies an identity: the success arm sees one `persist_result` for both the
/// receipt-writing branch and the plain-append branch. Removing the guard makes
/// every unidentified command report a receipt it never wrote, and without this
/// test that mutation survives — it did, before this test existed.
#[tokio::test]
async fn a_command_carrying_no_operation_identity_counts_nothing() {
    let obs = RecordingObservability::new();

    let with_events = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new(),
        OutcomeEntity::emitting(),
        context_without_identity(),
        obs.clone(),
    )
    .await;
    assert!(
        with_events.is_ok(),
        "the pre-existing path still succeeds: {with_events:?}"
    );

    let without_events = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new(),
        OutcomeEntity::silent(),
        context_without_identity(),
        obs.clone(),
    )
    .await;
    assert!(
        without_events.is_ok(),
        "and so does its no-events variant: {without_events:?}"
    );

    assert_eq!(
        obs.receipt_outcomes(),
        Vec::new(),
        "no receipt was written on either path, so no outcome may be reported: {:?}",
        obs.receipt_outcomes()
    );
}

/// The raw operation key appears in nothing this slice emits.
///
/// Scanned over every observation of every channel rather than over one known
/// field. The key is client-supplied and unbounded, so as a metric dimension it
/// would multiply time series without limit — and as of the typed-metric port
/// that is a rule to keep, not something the types make unrepresentable.
#[tokio::test]
async fn no_observation_carries_the_raw_operation_key() {
    let obs = RecordingObservability::new();

    // Every path, so no single arm can be the one that leaks.
    let _ = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new(),
        OutcomeEntity::emitting(),
        context("fp-1"),
        obs.clone(),
    )
    .await;
    let _ = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new().with_receipt(receipt_for(ENTITY_TYPE, "fp-1")),
        OutcomeEntity::emitting(),
        context("fp-1"),
        obs.clone(),
    )
    .await;
    let _ = send_observed(
        ENTITY_TYPE,
        OutcomeStore::new().with_receipt(receipt_for(ENTITY_TYPE, "fp-stored")),
        OutcomeEntity::emitting(),
        context("fp-different"),
        obs.clone(),
    )
    .await;

    let observed = obs.all_observed_text();
    assert!(
        !observed.contains(RAW_KEY),
        "the client-supplied key must appear in no name, no attribute key or value, and no \
         event or log this slice produces: {observed}"
    );
    assert!(
        !obs.receipt_outcomes().is_empty(),
        "the scan is only meaningful if something was actually emitted"
    );
}

/// Without observability the operation behaves exactly as before.
#[tokio::test]
async fn an_operation_without_observability_behaves_identically() {
    let replayed = send_unobserved(
        OutcomeStore::new().with_receipt(receipt_for(ENTITY_TYPE, "fp-1")),
        OutcomeEntity::emitting(),
        context("fp-1"),
    )
    .await;
    assert!(
        matches!(replayed, Ok(CommandResult::Replayed { .. })),
        "an unobserved replay still answers from the receipt: {replayed:?}"
    );

    let conflict = send_unobserved(
        OutcomeStore::new().with_receipt(receipt_for(ENTITY_TYPE, "fp-stored")),
        OutcomeEntity::emitting(),
        context("fp-different"),
    )
    .await;
    assert!(
        matches!(conflict, Err(EntityError::OperationConflict { .. })),
        "an unobserved conflict is still refused: {conflict:?}"
    );

    let fresh = send_unobserved(
        OutcomeStore::new(),
        OutcomeEntity::emitting(),
        context("fp-1"),
    )
    .await;
    assert!(
        fresh.is_ok(),
        "an unobserved fresh command still succeeds: {fresh:?}"
    );
}
