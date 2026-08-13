//! AD-10's `idempotency.reserve` span, observed from a real HTTP request —
//! PROD-012 B7.11.
//!
//! # Why this test exists, and where it stops
//!
//! The redaction primitive landed before any instrumentation could use it, so
//! nothing yet showed that a **real** dispatch attaches the token of the key a
//! **real** client presented. Everything before this was either a unit asserting a
//! type's behaviour or a test that built the span attributes itself.
//!
//! Here nothing is constructed by the test except the request. The key arrives in
//! an `Idempotency-Key` header, crosses `resolve_operation_key`, the generated
//! proxy, `#[authorize]`, `#[tenant_scoped]`, and reaches the reservation slot —
//! and what is asserted is the token the runtime handed the `Tracer`, compared
//! against one this test derives from the header value it sent.
//!
//! The chain is:
//!
//! ```text
//! Idempotency-Key: K
//!   → OperationKey(K) → OperationKeyHash::of → SpanAttributes → Tracer  ← here
//!                                                             → exporter ← elsewhere
//! ```
//!
//! **It stops at the `Tracer` port, deliberately.** The port → exporter hop is
//! covered where the adapter that owns it lives, by
//! `the_exported_span_carries_the_correlation_token_and_never_the_raw_key` in
//! `crates/infrastructure/src/tracing_otlp.rs`, against the real `OtlpTracer` and a
//! real in-memory exporter. Reaching the exporter *from here* would mean making
//! `OtlpTracer::from_processor` public and giving this example a dependency on the
//! OpenTelemetry SDK — widening a production API and inverting the layering so an
//! example depends on infrastructure internals, to move an assertion that is
//! already made. The two tests meet at a real seam; that is not a gap in the chain.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Utc};
use ego_domain::operation::{
    FencingToken, Lease, OperationId, OperationKey, OperationKeyHash, OperationReservationStore,
    OwnerFence, OwnerId, ReservationError, ReservationOutcome, ReserveRequest,
    StoredServiceResponse,
};
use ego_domain::{SpanAttributes, SpanId, SpanOutcome, TraceContext, Tracer};
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, RuntimeBuilder};
use ego_testkit::ScriptedAuthorizationProvider;
use ego_transport::AppState;
use reference_app::application::{
    RegisterInput, RegisterOutput, RegisterUser, RegisterUserError, RegisterUserTag,
};
use reference_app::ports::http::build_router;
use reference_app::{build_runtime, AppConfig, BuiltRuntime};
use support::make_token;
use tower::ServiceExt;

/// The client-supplied key. Shaped like a business identifier on purpose: it is
/// exactly the kind of value AD-10 forbids emitting, so a leak would be visible
/// rather than looking like an opaque id.
const KEY: &str = "invoice-2026-03-4417";

// ---------------------------------------------------------------------------
// Doubles: only the reservation store and the tracer. Everything else is the
// reference app's own wiring.
// ---------------------------------------------------------------------------

struct PassthroughRegister;

#[async_trait]
impl RegisterUser for PassthroughRegister {
    async fn register(
        &self,
        _ctx: ServiceContext,
        input: RegisterInput,
    ) -> Result<RegisterOutput, RegisterUserError> {
        Ok(RegisterOutput {
            user_id: input.user_id,
            tenant_id: input.tenant_id,
        })
    }
}

/// Answers one `reserve` with a fresh lease, and records the key it was handed.
///
/// The recorded key is what lets the assertion compare the token against the key
/// the *store* saw, rather than against one the test assumed reached it.
struct RecordingStore {
    keys: Mutex<Vec<OperationKey>>,
}

impl RecordingStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            keys: Mutex::new(Vec::new()),
        })
    }

    fn keys(&self) -> Vec<OperationKey> {
        self.keys.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl OperationReservationStore for RecordingStore {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        self.keys
            .lock()
            .expect("not poisoned")
            .push(req.operation_key.clone());
        Ok(ReservationOutcome::Fresh(Lease {
            operation_id: OperationId::new(req.tenant.clone(), req.operation_key.clone()),
            owner_id: req.owner_id.clone(),
            fencing_token: FencingToken::initial(),
            lease_until: req.lease_until,
        }))
    }
    async fn renew(&self, _f: &OwnerFence, _u: DateTime<Utc>) -> Result<(), ReservationError> {
        Ok(())
    }
    async fn complete(
        &self,
        _f: &OwnerFence,
        _r: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        Ok(())
    }
    async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
        Ok(())
    }
    async fn purge_completed_before(
        &self,
        _c: DateTime<Utc>,
        _b: usize,
    ) -> Result<u64, ReservationError> {
        Ok(0)
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        Ok(())
    }
}

/// Records every span with the attributes it was opened with.
struct SpanRecordingTracer {
    started: Mutex<Vec<(String, SpanAttributes)>>,
    ended: AtomicUsize,
}

impl SpanRecordingTracer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            started: Mutex::new(Vec::new()),
            ended: AtomicUsize::new(0),
        })
    }

    fn started(&self) -> Vec<(String, SpanAttributes)> {
        self.started.lock().expect("not poisoned").clone()
    }
}

impl Tracer for SpanRecordingTracer {
    fn start_span(&self, _ctx: &TraceContext, name: &str, attrs: SpanAttributes) {
        self.started
            .lock()
            .expect("not poisoned")
            .push((name.to_string(), attrs));
    }
    fn end_span(&self, _span: SpanId, _outcome: SpanOutcome) {
        self.ended.fetch_add(1, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// The app, and one request
// ---------------------------------------------------------------------------

fn app(store: Arc<RecordingStore>, tracer: Arc<SpanRecordingTracer>) -> Router {
    let BuiltRuntime {
        authn,
        read_side: read_side_handles,
        ..
    } = build_runtime(&AppConfig::default()).expect("build_runtime succeeds");

    let service: Arc<dyn RegisterUser> = Arc::new(PassthroughRegister);
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
        .with_tracer(tracer as Arc<dyn Tracer>)
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

// ---------------------------------------------------------------------------
// The assertions
// ---------------------------------------------------------------------------

/// A real request produces `idempotency.reserve`, carrying the token of the key
/// the client sent — and the raw key nowhere.
///
/// Three independent things have to hold, and each fails on a different mutation:
///
/// - **the span exists, under its documented name** — dies if the emission is
///   removed or renamed;
/// - **its token is the one derived from the header value**, compared against a
///   token this test computes from `KEY` *and* against the key the store recorded
///   — dies if the wrong value is hashed, or a constant is;
/// - **the raw key appears in no attribute** — dies if the emission ever carries
///   the key instead of its digest.
#[tokio::test]
async fn a_real_request_emits_the_reserve_span_with_the_presented_keys_token() {
    let store = RecordingStore::new();
    let tracer = SpanRecordingTracer::new();

    let response = app(store.clone(), tracer.clone())
        .oneshot(post())
        .await
        .expect("the request is served");
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the request must succeed, or the span under test was never reached"
    );

    // The key genuinely crossed the whole chain and reached the store.
    assert_eq!(
        store.keys(),
        vec![OperationKey::parse(KEY).expect("valid key")],
        "the store must have been asked to reserve the header's key"
    );

    let spans = tracer.started();
    let reserve: Vec<_> = spans
        .iter()
        .filter(|(name, _)| name == "idempotency.reserve")
        .collect();
    assert_eq!(
        reserve.len(),
        1,
        "exactly one reserve span for one request, got {spans:?}"
    );

    let expected = OperationKeyHash::of(&OperationKey::parse(KEY).expect("valid key"));
    assert_eq!(
        reserve[0].1.operation_key_hash(),
        Some(expected.as_str()),
        "the span must carry the token of the key the client presented — not of \
         some other value, and not a constant"
    );

    // And nowhere on that span is the key itself. Swept over the whole rendered
    // attribute set rather than the one field expected to be wrong, because a leak
    // worth catching would most likely surface somewhere nobody thought to check.
    let rendered = format!("{:?}", reserve[0].1);
    assert!(
        !rendered.contains(KEY),
        "the client-supplied key must appear in no attribute: {rendered}"
    );

    assert_eq!(
        tracer.ended.load(Ordering::Relaxed),
        spans.len(),
        "every span opened must be closed, or the adapter's table leaks one per request"
    );
}

/// The same request against a runtime with no tracer is served identically.
///
/// The negative control. Instrumentation must not become a precondition for
/// dispatch, and the untraced configuration is the one most deployments run.
#[tokio::test]
async fn the_same_request_without_a_tracer_is_served_the_same_way() {
    let store = RecordingStore::new();

    let BuiltRuntime {
        authn,
        read_side: read_side_handles,
        ..
    } = build_runtime(&AppConfig::default()).expect("build_runtime succeeds");
    let service: Arc<dyn RegisterUser> = Arc::new(PassthroughRegister);
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::MandatoryKey)
        .with_operation_reservation_store(store.clone())
        .with_reservation_owner_id(OwnerId::new("under-test"))
        .with_reservation_lease_duration(Duration::from_secs(30))
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        .with_security(
            authn.clone(),
            Arc::new(ScriptedAuthorizationProvider::allow_all()),
        )
        .build();
    let router = build_router(
        AppState::new(runtime.resolver(), authn),
        read_side_handles.query.clone(),
    );

    let response = router.oneshot(post()).await.expect("the request is served");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        store.keys().len(),
        1,
        "the reservation still happens without tracing"
    );
}
