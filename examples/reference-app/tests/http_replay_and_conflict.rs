//! PROD-012 B6.7 — what a client sees when a reservation answers for it.
//!
//! Two questions this file exists to separate, because from outside the process
//! they can look identical: *did the second identical request replay, or did it
//! re-execute and happen to produce the same answer?* And: *is a permanent
//! conflict distinguishable from a transient one?*
//!
//! # Why response bodies are not the evidence
//!
//! `RegisterOutput` is built by copying `input.user_id` and `input.tenant_id`
//! verbatim (`application.rs`), so two identical requests produce byte-identical
//! responses whether the body ran once, twice or not at all. Comparing them
//! **cannot fail**, which makes it worth nothing.
//!
//! So the replay case has the store return a response the handler *could not
//! have produced from this input*. If that marked value comes back over HTTP,
//! it came from the store. Backed by three counts on every case: how many times
//! the store was asked to reserve, how many times the handler body ran, and how
//! many times a completion was recorded.
//!
//! # Scope
//!
//! B6.7 asks for replay versus conflict. The full six-way refusal mapping
//! landed in #280 with no HTTP-level test at all, so the whole table is closed
//! here rather than leaving public branches uncovered in the same file.
//!
//! One rejection is deliberately absent from *this* file:
//! `RequestNotFingerprintable` cannot be provoked through a store script — it is
//! raised before the store is reached, when an operation's arguments fail to
//! serialise, and `RegisterInput` always does.
//!
//! Its HTTP translation is proven directly against the mapper instead, in
//! `handlers.rs`'s own unit tests
//! (`every_reservation_rejection_maps_to_the_status_its_caller_can_act_on`),
//! which enumerates all six rejections rather than only the unreachable one —
//! a table with one entry proven elsewhere and five assumed is how a mapping
//! drifts.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use ego_domain::operation::{
    FencingToken, Lease, OperationId, OperationReservationStore, OwnerFence, OwnerId,
    ReservationError, ReservationOutcome, ReserveRequest, StoredServiceResponse,
};
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, RuntimeBuilder};
use ego_testkit::ScriptedAuthorizationProvider;
use ego_transport::AppState;
use reference_app::application::{
    RegisterInput, RegisterOutput, RegisterUser, RegisterUserError, RegisterUserTag,
};
use reference_app::ports::http::build_router;
use reference_app::{build_runtime_in_memory, AppConfig, BuiltRuntime};
use support::make_token;
use tower::ServiceExt;

const KEY: &str = "op-under-test";

/// Counts how many times the handler body actually ran. Every "did not execute"
/// row in the matrix reads this, because a replay that re-ran the handler
/// produces a second set of effects — the exact duplicate the reservation
/// exists to prevent — while looking correct from outside.
struct CountingRegister {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl RegisterUser for CountingRegister {
    async fn register(
        &self,
        _ctx: ServiceContext,
        input: RegisterInput,
    ) -> Result<RegisterOutput, RegisterUserError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(RegisterOutput {
            user_id: input.user_id,
            tenant_id: input.tenant_id,
        })
    }
}

/// Answers `reserve` from a script and counts what it was asked to do.
struct ScriptedStore {
    answer: Mutex<Option<Result<ReservationOutcome, ReservationError>>>,
    reserves: AtomicUsize,
    completes: AtomicUsize,
}

impl ScriptedStore {
    fn answering(answer: Result<ReservationOutcome, ReservationError>) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(Some(answer)),
            reserves: AtomicUsize::new(0),
            completes: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl OperationReservationStore for ScriptedStore {
    async fn reserve(&self, _req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        self.reserves.fetch_add(1, Ordering::SeqCst);
        self.answer
            .lock()
            .expect("not poisoned")
            .take()
            .expect("each case reserves exactly once")
    }
    async fn renew(
        &self,
        _f: &OwnerFence,
        _u: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), ReservationError> {
        unreachable!("nothing renews here")
    }
    async fn complete(
        &self,
        _f: &OwnerFence,
        _r: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        self.completes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
        unreachable!("nothing abandons here")
    }
    async fn purge_completed_before(
        &self,
        _c: chrono::DateTime<chrono::Utc>,
        _b: usize,
    ) -> Result<u64, ReservationError> {
        unreachable!("nothing purges here")
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        Ok(())
    }
}

fn lease_for(req_tenant: Option<ego_domain::context::TenantId>) -> Lease {
    Lease {
        operation_id: OperationId::new(
            req_tenant,
            ego_domain::operation::OperationKey::parse(KEY).expect("valid"),
        ),
        owner_id: OwnerId::new("under-test"),
        fencing_token: FencingToken::initial(),
        lease_until: chrono::Utc::now() + chrono::Duration::seconds(30),
    }
}

/// The real router, with `RegisterUser` counted and the reservation store
/// scripted. Everything else — authentication, guards, routing, the generated
/// proxy — is production's.
fn app(store: Arc<ScriptedStore>, calls: Arc<AtomicUsize>) -> Router {
    let BuiltRuntime {
        authn,
        read_side: read_side_handles,
        ..
    } = build_runtime_in_memory(&AppConfig::default()).expect("build_runtime succeeds");

    let service: Arc<dyn RegisterUser> = Arc::new(CountingRegister { calls });
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::MandatoryKey)
        .with_operation_reservation_store(store)
        .with_reservation_owner_id(OwnerId::new("under-test"))
        .with_reservation_lease_duration(Duration::from_secs(30))
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        .with_security(
            authn.clone(),
            Arc::new(ScriptedAuthorizationProvider::allow_all()),
        )
        .build();

    build_router(
        AppState::new(runtime.resolver(), authn),
        read_side_handles.query.clone(),
    )
}

fn post() -> Request<Body> {
    let token = make_token("user-1", "tenant-a");
    Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", KEY)
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

/// Drives one request and returns the status, the body, and the three counts.
async fn run(
    answer: Result<ReservationOutcome, ReservationError>,
) -> (StatusCode, Vec<u8>, usize, usize, usize) {
    let store = ScriptedStore::answering(answer);
    let calls = Arc::new(AtomicUsize::new(0));
    let response = app(store.clone(), calls.clone())
        .oneshot(post())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a readable body")
        .to_vec();
    (
        status,
        body,
        store.reserves.load(Ordering::SeqCst),
        calls.load(Ordering::SeqCst),
        store.completes.load(Ordering::SeqCst),
    )
}

// ---------------------------------------------------------------------------
// Replay — the answer comes from the store, and nothing runs
// ---------------------------------------------------------------------------

/// The central case. The store holds a response **the handler could not have
/// produced from this input**, so its arrival over HTTP is proof the answer was
/// replayed rather than recomputed. A test comparing two identical responses
/// could not tell the difference and would pass either way.
#[tokio::test]
async fn a_replay_returns_the_stored_response_and_runs_nothing() {
    let marked = ego_service_sdk::runtime::encode_stored_response(&RegisterOutput {
        user_id: "REPLAYED-FROM-THE-STORE".to_string(),
        tenant_id: "tenant-a".to_string(),
    })
    .expect("encodes");

    let (status, body, reserves, calls, completes) =
        run(Ok(ReservationOutcome::Succeeded(marked))).await;

    assert_eq!(status, StatusCode::CREATED);
    let answer: RegisterOutput = serde_json::from_slice(&body).expect("a RegisterOutput");
    assert_eq!(
        answer.user_id, "REPLAYED-FROM-THE-STORE",
        "the recorded answer, not one rebuilt from the request — the handler had \
         no way to produce this value"
    );

    assert_eq!(reserves, 1);
    assert_eq!(
        calls, 0,
        "a replay that re-runs the handler produces a second set of effects, \
         which is the exact bug this whole change exists to close"
    );
    // Guarded by the type rather than by this assertion, and worth saying so:
    // `ReservationDecision::Replay` carries a `StoredServiceResponse` and no
    // permit, so there is no fence to complete under. No mutation can make this
    // non-zero without inventing one, which is exactly the shape that split was
    // introduced to prevent. The assertion stays as a regression tripwire for
    // anyone who later folds the two variants back together.
    assert_eq!(
        completes, 0,
        "a replay produced no new response; recording one would overwrite a \
         durable answer under a fence this dispatch never held"
    );
}

// ---------------------------------------------------------------------------
// The refusals, each mapped to what a caller can act on
// ---------------------------------------------------------------------------

/// The five store-answerable refusals, asserted by status **and** by the three
/// counts. A status alone would not show whether the operation ran first.
///
/// The split is by what the caller can do: 409 says this key is taken — by this
/// request or another; 503 says the machinery could not answer and later may
/// work; 500 says only an operator can clear it.
#[tokio::test]
async fn every_refusal_maps_to_a_status_a_caller_can_act_on() {
    let cases: Vec<(
        &str,
        Result<ReservationOutcome, ReservationError>,
        StatusCode,
    )> = vec![
        (
            "the same key already carries a different request — permanent",
            Ok(ReservationOutcome::Conflict),
            StatusCode::CONFLICT,
        ),
        (
            "this runtime already holds the lease",
            Ok(ReservationOutcome::OwnedInProgress(lease_for(None))),
            StatusCode::CONFLICT,
        ),
        (
            "another owner holds the lease",
            Ok(ReservationOutcome::OtherInProgress),
            StatusCode::CONFLICT,
        ),
        (
            "the store could not answer — transient",
            Err(ReservationError::Backend("down".to_string())),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            "it completed, but this build cannot read what it stored",
            Ok(ReservationOutcome::Succeeded(StoredServiceResponse::new(
                b"not an envelope this build writes".to_vec(),
            ))),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ];

    for (what, answer, expected) in cases {
        let (status, _body, reserves, calls, completes) = run(answer).await;

        assert_eq!(status, expected, "{what}");
        assert_eq!(reserves, 1, "{what}: reserved once");
        assert_eq!(
            calls, 0,
            "{what}: a refused reservation must not reach the handler"
        );
        assert_eq!(
            completes, 0,
            "{what}: nothing ran, so there is no answer to record"
        );
    }
}

// ---------------------------------------------------------------------------
// The control: the same wiring, permitted
// ---------------------------------------------------------------------------

/// Without this, every assertion above could pass on a router that refuses
/// everything. A permitted reservation runs the handler exactly once and records
/// its answer — so the zeros elsewhere are attributable to the refusal rather
/// than to the fixture.
#[tokio::test]
async fn a_permitted_reservation_runs_the_handler_once_and_records_its_answer() {
    let (status, body, reserves, calls, completes) =
        run(Ok(ReservationOutcome::Fresh(lease_for(None)))).await;

    assert_eq!(status, StatusCode::CREATED);
    let answer: RegisterOutput = serde_json::from_slice(&body).expect("a RegisterOutput");
    assert_eq!(
        answer.user_id, "user-1",
        "this one *is* rebuilt from the request, which is what makes the marked \
         value in the replay case meaningful"
    );

    assert_eq!(reserves, 1);
    assert_eq!(calls, 1);
    assert_eq!(completes, 1);
}
