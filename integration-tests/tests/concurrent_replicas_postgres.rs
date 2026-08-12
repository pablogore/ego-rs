//! **Guarantee:** two independent runtimes racing the same fresh operation key
//! produce exactly one execution. One is permitted and runs; the other is refused
//! without running, and the reservation they contended for is left holding a
//! single durable answer.
//!
//! **Layers traversed:** HTTP (the production router, real authentication and
//! guards, twice over) → the generated `#[idempotent]` dispatch → two separate
//! `PostgresOperationReservationStore` instances on two separate connection pools
//! → one `INSERT … ON CONFLICT DO NOTHING` against a real PostgreSQL, with real
//! migrations.
//!
//! **Why in-process cannot show this.** Two concurrently released reservation
//! attempts resolving to one durable winner is a database outcome. The two runtimes
//! here share no reservation state whatsoever — separate stores, separate pools,
//! separate owner identities — so the only thing that can decide which of them may
//! execute is the row. An in-memory store shared by two runtimes would decide it
//! with a mutex, which is a different mechanism answering a different question.
//!
//! # What the barriers are, and what they are not
//!
//! Two coordination points, both test scaffolding:
//!
//! 1. **Before `reserve()` delegates**, both replicas meet at a barrier, so both
//!    reservation attempts are released together toward independent pools. Without
//!    it, one request could be answered end to end before the other began, and the
//!    contention would be a hope about scheduling rather than an arrangement.
//! 2. **Inside the winner's body**, execution is held open until the test releases
//!    it. That is what makes the loser's answer deterministic: while the winner is
//!    parked, the row stays `in_progress`, so the loser is refused rather than
//!    served a replay. Both are correct protocol outcomes; only one of them is a
//!    stable thing to assert.
//!
//! Neither barrier decides anything. They fix *when* the two attempts are released
//! and how long the winner holds its lease; **who** may execute is settled entirely
//! by the row. The shared counters are observers for the same reason — they record
//! what happened without participating in it.
//!
//! ## What this does not establish
//!
//! Worth stating, because a barrier invites the stronger reading: releasing both
//! attempts together does **not** prove the two `INSERT` statements overlapped
//! inside PostgreSQL. The runtime scheduler or either pool could still let one
//! statement finish before the other starts, and nothing here observes SQL-level
//! interleaving. The dropped-index mutation does not close that gap either — with
//! no uniqueness, two rows appear whether the inserts overlapped or ran one after
//! the other.
//!
//! What is established is the property the protocol actually promises: two
//! independent runtimes given the opportunity to reserve at the same moment leave
//! **one** row, run **one** body, and answer 201 and 409. Serialised or overlapped,
//! exactly one may execute.
//!
//! # The control case
//!
//! A harness that refused the second request of any pair would satisfy every
//! assertion about the contended key while proving nothing. So the same wiring is
//! run once more with two *different* keys, where both requests must be permitted
//! and both bodies must run.
//!
//! Its scope, precisely: it rules out **global refusal**, not serialisation. A
//! harness that merely serialised the two requests would also permit two distinct
//! keys and also answer 201/201, so this control cannot distinguish that case — and
//! does not need to, since serialisation would not make the single-execution
//! guarantee above any less true.
//!
//! Run: `cargo test --manifest-path integration-tests/Cargo.toml`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Utc};
use ego_domain::operation::{
    OperationReservationStore, OwnerFence, OwnerId, ReservationError, ReservationOutcome,
    ReserveRequest, StoredServiceResponse,
};
use ego_domain::time::SystemClock;
use ego_persistence::postgres::migrations;
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, RuntimeBuilder};
use ego_testkit::{ScriptedAuthorizationProvider, TestJwtBuilder};
use ego_transport::AppState;
use persistent_entity::builder::EntityRuntimeBuilder;
use reference_app::application::{
    RegisterInput, RegisterOutput, RegisterUser, RegisterUserError, RegisterUserImpl,
    RegisterUserTag,
};
use reference_app::ports::http::build_router;
use reference_app::{
    build_runtime, AppConfig, BuiltRuntime, DEV_SIGNING_KEY, REFERENCE_APP_AUDIENCE,
};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::{Barrier, Notify};
use tower::ServiceExt;

const CONTENDED_KEY: &str = "op-two-replicas-one-permit";
const CONTROL_KEY_A: &str = "op-control-replica-a";
const CONTROL_KEY_B: &str = "op-control-replica-b";

/// Bound on every wait in this test.
///
/// Barriers and gates can hang rather than fail, which would turn a real defect
/// into a stalled run. Every await is wrapped so a coordination failure is reported
/// as a failure.
const WAIT_LIMIT: StdDuration = StdDuration::from_secs(30);

// ---------------------------------------------------------------------------
// The gate that holds the winner's body open
// ---------------------------------------------------------------------------

/// Holds the **first** body to arrive until released; later bodies pass straight
/// through.
///
/// Only the first is held on purpose. If the exclusion breaks and two bodies run,
/// holding both would deadlock and the test would hang instead of failing — the
/// mutation this file exists to catch has to produce a verdict, not a stall.
struct Gate {
    entered: Notify,
    release: Notify,
    arrivals: AtomicUsize,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Notify::new(),
            release: Notify::new(),
            arrivals: AtomicUsize::new(0),
        })
    }

    async fn hold_if_first(&self) {
        self.entered.notify_one();
        if self.arrivals.fetch_add(1, Ordering::SeqCst) == 0 {
            self.release.notified().await;
        }
    }
}

/// Counts executions, and optionally parks the first one.
struct ObservedRegister {
    inner: Arc<dyn RegisterUser>,
    calls: Arc<AtomicUsize>,
    gate: Option<Arc<Gate>>,
}

#[async_trait]
impl RegisterUser for ObservedRegister {
    async fn register(
        &self,
        ctx: ServiceContext,
        input: RegisterInput,
    ) -> Result<RegisterOutput, RegisterUserError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.hold_if_first().await;
        }
        self.inner.register(ctx, input).await
    }
}

// ---------------------------------------------------------------------------
// The store wrapper that lines the two INSERTs up
// ---------------------------------------------------------------------------

/// Delegates to the durable store, having first waited for its counterpart.
///
/// `entered` is incremented before the barrier and `returned` after the delegate
/// answers, so the test can wait on the precise condition "both reservations have
/// been decided" instead of guessing.
struct CoordinatedStore {
    inner: PostgresOperationReservationStore,
    start_line: Arc<Barrier>,
    entered: Arc<AtomicUsize>,
    returned: Arc<AtomicUsize>,
    completes: Arc<AtomicUsize>,
}

#[async_trait]
impl OperationReservationStore for CoordinatedStore {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        // Both attempts are released together, toward independent pools. This does
        // not assert that the two INSERTs overlap inside PostgreSQL — nothing here
        // observes that — only that neither request was answered before the other
        // was allowed to start.
        self.start_line.wait().await;
        let outcome = self.inner.reserve(req).await;
        self.returned.fetch_add(1, Ordering::SeqCst);
        outcome
    }
    async fn renew(&self, f: &OwnerFence, until: DateTime<Utc>) -> Result<(), ReservationError> {
        self.inner.renew(f, until).await
    }
    async fn complete(
        &self,
        f: &OwnerFence,
        r: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        self.completes.fetch_add(1, Ordering::SeqCst);
        self.inner.complete(f, r).await
    }
    async fn abandon(&self, f: &OwnerFence) -> Result<(), ReservationError> {
        self.inner.abandon(f).await
    }
    async fn purge_completed_before(
        &self,
        cutoff: DateTime<Utc>,
        batch: usize,
    ) -> Result<u64, ReservationError> {
        self.inner.purge_completed_before(cutoff, batch).await
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        self.inner.probe().await
    }
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

fn make_token(sub: &str, tenant_id: &str) -> String {
    TestJwtBuilder::new(DEV_SIGNING_KEY.to_vec())
        .subject(sub)
        .tenant_id(tenant_id)
        .claim("aud", Value::from(REFERENCE_APP_AUDIENCE))
        .build()
}

/// Everything one replica needs, shared with nothing except the database and the
/// test's observers.
struct Replica {
    router: Router,
}

#[allow(clippy::too_many_arguments)]
fn replica(
    owner: &str,
    pool: PgPool,
    start_line: Arc<Barrier>,
    entered: Arc<AtomicUsize>,
    returned: Arc<AtomicUsize>,
    completes: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
    gate: Option<Arc<Gate>>,
) -> Replica {
    let BuiltRuntime {
        authn,
        read_side: read_side_handles,
        ..
    } = build_runtime(&AppConfig::default()).expect("build_runtime succeeds");

    let real: Arc<dyn RegisterUser> = Arc::new(RegisterUserImpl::new(
        Arc::new(EntityRuntimeBuilder::new().build()),
        Arc::new(EntityRuntimeBuilder::new().build()),
        None,
    ));
    let observed: Arc<dyn RegisterUser> = Arc::new(ObservedRegister {
        inner: real,
        calls,
        gate,
    });

    let store = Arc::new(CoordinatedStore {
        inner: PostgresOperationReservationStore::new(pool, Arc::new(SystemClock)),
        start_line,
        entered,
        returned,
        completes,
    });

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::MandatoryKey)
        .with_operation_reservation_store(store)
        .with_reservation_owner_id(OwnerId::new(owner))
        .with_reservation_lease_duration(StdDuration::from_secs(30))
        .with_service::<RegisterUserTag>(observed)
        .expect("registration succeeds")
        .with_security(
            authn.clone(),
            Arc::new(ScriptedAuthorizationProvider::allow_all()),
        )
        .build();

    Replica {
        router: build_router(
            AppState::new(runtime.resolver(), authn),
            read_side_handles.query.clone(),
        ),
    }
}

fn post(key: &str) -> Request<Body> {
    let token = make_token("user-1", "tenant-a");
    Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", key)
        .body(Body::from(
            serde_json::json!({
                "user_id": "user-1",
                "email": "user@example.com",
                "tenant_id": "tenant-a",
                "org_name": "Acme",
            })
            .to_string(),
        ))
        .unwrap()
}

async fn send(router: Router, key: &str) -> StatusCode {
    router
        .oneshot(post(key))
        .await
        .expect("the router answers")
        .status()
}

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the container accepts connections")
}

/// Waits for a counter to reach `target`, or fails at the deadline.
///
/// The condition is the exit, and the deadline is an assertion — the same shape as
/// the fencing test's lock poll, and for the same reason: a wait that gives up
/// quietly would let the test proceed on an arrangement it never achieved.
async fn wait_for(counter: &AtomicUsize, target: usize, what: &str) {
    let started = Instant::now();
    while counter.load(Ordering::SeqCst) < target {
        assert!(
            started.elapsed() < WAIT_LIMIT,
            "{what}: expected {target}, still {} after {WAIT_LIMIT:?}",
            counter.load(Ordering::SeqCst)
        );
        tokio::time::sleep(StdDuration::from_millis(5)).await;
    }
}

async fn rows_for(pool: &PgPool, key: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM operation_reservations WHERE operation_key = $1")
        .bind(key)
        .fetch_one(pool)
        .await
        .expect("the count comes back")
}

async fn state_and_response(pool: &PgPool, key: &str) -> (String, Option<Vec<u8>>) {
    sqlx::query_as("SELECT state, response FROM operation_reservations WHERE operation_key = $1")
        .bind(key)
        .fetch_one(pool)
        .await
        .expect("exactly one reservation row for this key")
}

#[tokio::test]
async fn two_replicas_racing_one_key_yield_exactly_one_execution() {
    let container = Postgres::default()
        .start()
        .await
        .expect("a PostgreSQL container starts");
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.expect("a host"),
        container
            .get_host_port_ipv4(5432)
            .await
            .expect("the mapped port"),
    );

    // Separate pools, so the two replicas share no connection either. Migrations
    // run once for the whole test, through the observer pool.
    let pool_a = connect(&url).await;
    let pool_b = connect(&url).await;
    let observer = connect(&url).await;
    migrations::run(&observer)
        .await
        .expect("the real migrations apply");

    // -----------------------------------------------------------------------
    // The contended key
    // -----------------------------------------------------------------------
    let start_line = Arc::new(Barrier::new(2));
    let entered = Arc::new(AtomicUsize::new(0));
    let returned = Arc::new(AtomicUsize::new(0));
    let completes = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let gate = Gate::new();

    let a = replica(
        "replica-a",
        pool_a.clone(),
        start_line.clone(),
        entered.clone(),
        returned.clone(),
        completes.clone(),
        calls.clone(),
        Some(gate.clone()),
    );
    let b = replica(
        "replica-b",
        pool_b.clone(),
        start_line.clone(),
        entered.clone(),
        returned.clone(),
        completes.clone(),
        calls.clone(),
        Some(gate.clone()),
    );

    let task_a = tokio::spawn(async move { send(a.router, CONTENDED_KEY).await });
    let task_b = tokio::spawn(async move { send(b.router, CONTENDED_KEY).await });

    // Both reservations have been decided by the database. Waited for explicitly
    // rather than inferred, so what follows cannot race with a reservation that
    // has not happened yet.
    wait_for(&returned, 2, "both reservations decided").await;

    // The winner is inside its body and parked there.
    tokio::time::timeout(WAIT_LIMIT, gate.entered.notified())
        .await
        .expect("a body must have started — one replica was permitted");

    // --- The durable state while the winner is still holding its lease -------
    assert_eq!(
        rows_for(&observer, CONTENDED_KEY).await,
        1,
        "exactly one reservation row for the contended key. Two rows means both \
         inserts were accepted, so nothing was excluded and the statuses below \
         would be describing something else"
    );
    let (state_while_held, response_while_held) =
        state_and_response(&observer, CONTENDED_KEY).await;
    assert_eq!(
        state_while_held, "in_progress",
        "the reservation is held, not yet answered — this is the window in which \
         the loser was refused"
    );
    assert!(
        response_while_held.is_none(),
        "and nothing is recorded yet, which the table's own CHECK also requires"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "exactly one body has run. This is the invariant; the statuses are its \
         observable consequence"
    );
    assert_eq!(
        completes.load(Ordering::SeqCst),
        0,
        "the winner has not finished, so no answer has been recorded"
    );

    // --- Release the winner and collect both answers -------------------------
    gate.release.notify_one();

    let status_a = tokio::time::timeout(WAIT_LIMIT, task_a)
        .await
        .expect("replica A answered within the limit")
        .expect("replica A's task completed");
    let status_b = tokio::time::timeout(WAIT_LIMIT, task_b)
        .await
        .expect("replica B answered within the limit")
        .expect("replica B's task completed");

    // Which replica wins is genuinely undecided — that is what makes it a race.
    // The pair is what the protocol promises, not the assignment.
    let mut statuses = [status_a, status_b];
    statuses.sort_by_key(|s| s.as_u16());
    assert_eq!(
        statuses,
        [StatusCode::CREATED, StatusCode::CONFLICT],
        "one replica is permitted and answers 201; the other is refused with 409. \
         Got A={status_a}, B={status_b}"
    );

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "still exactly one execution after both requests finished"
    );
    assert_eq!(
        completes.load(Ordering::SeqCst),
        1,
        "and exactly one answer was recorded — the refused replica had no permit \
         to complete under"
    );

    // --- The settled durable state ------------------------------------------
    assert_eq!(rows_for(&observer, CONTENDED_KEY).await, 1);
    let (final_state, final_response) = state_and_response(&observer, CONTENDED_KEY).await;
    assert_eq!(
        final_state, "completed",
        "the reservation the two replicas contended for now holds one answer"
    );
    assert!(
        final_response.is_some(),
        "and that answer is durable, not just returned"
    );

    // -----------------------------------------------------------------------
    // The control: different keys must both execute
    // -----------------------------------------------------------------------
    //
    // Without this, every assertion above is also satisfied by a harness that
    // refuses the second request of any pair. Same wiring, same barrier, no gate —
    // both bodies must run to completion.
    //
    // It does NOT rule out a harness that merely serialises: that would permit two
    // distinct keys too, and answer 201/201 just the same. Scope stated rather than
    // implied, and it costs nothing here — serialisation would not weaken the
    // single-execution guarantee.
    let control_calls = Arc::new(AtomicUsize::new(0));
    let control_completes = Arc::new(AtomicUsize::new(0));
    let control_returned = Arc::new(AtomicUsize::new(0));
    let control_line = Arc::new(Barrier::new(2));

    let ca = replica(
        "replica-a",
        pool_a.clone(),
        control_line.clone(),
        Arc::new(AtomicUsize::new(0)),
        control_returned.clone(),
        control_completes.clone(),
        control_calls.clone(),
        None,
    );
    let cb = replica(
        "replica-b",
        pool_b.clone(),
        control_line.clone(),
        Arc::new(AtomicUsize::new(0)),
        control_returned.clone(),
        control_completes.clone(),
        control_calls.clone(),
        None,
    );

    let ct_a = tokio::spawn(async move { send(ca.router, CONTROL_KEY_A).await });
    let ct_b = tokio::spawn(async move { send(cb.router, CONTROL_KEY_B).await });

    let control_status_a = tokio::time::timeout(WAIT_LIMIT, ct_a)
        .await
        .expect("control A answered")
        .expect("control A's task completed");
    let control_status_b = tokio::time::timeout(WAIT_LIMIT, ct_b)
        .await
        .expect("control B answered")
        .expect("control B's task completed");

    assert_eq!(
        (control_status_a, control_status_b),
        (StatusCode::CREATED, StatusCode::CREATED),
        "two distinct keys are not in contention, so both must be permitted. If \
         either were refused, the 409 above would be an artefact of this harness \
         rather than of the reservation"
    );
    assert_eq!(
        control_calls.load(Ordering::SeqCst),
        2,
        "both bodies ran — the exclusion above is specific to the shared key, not a \
         harness that refuses whatever arrives second"
    );
    assert_eq!(control_completes.load(Ordering::SeqCst), 2);
    assert_eq!(rows_for(&observer, CONTROL_KEY_A).await, 1);
    assert_eq!(rows_for(&observer, CONTROL_KEY_B).await, 1);
}
