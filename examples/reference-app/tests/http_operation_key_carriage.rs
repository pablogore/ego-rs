//! PROD-012 B6.5 — the `Idempotency-Key` a client sends reaches the operation.
//!
//! The extractor's own tests (`crates/transport`) prove it resolves a key
//! correctly. They cannot prove the handler then *carries* it: an extractor that
//! works perfectly and a handler that ignores its output produce exactly the
//! same passing extractor test, and a request that dispatches with no operation
//! identity at all.
//!
//! So this drives the real router over a real `POST /register` and asserts on
//! what the **service actually received** — the one place a dropped transfer is
//! visible.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use ego_domain::operation::{
    FencingToken, Lease, OperationId, OperationKey, OperationReservationStore, OwnerId,
    ReservationError, ReservationOutcome, ReserveRequest,
};
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

/// Stands in for `RegisterUserImpl` and records the operation key it was
/// invoked with. Recording what the *service* saw is the whole point: it is
/// downstream of the extractor and downstream of the handler, so it is the only
/// vantage point from which a dropped transfer is distinguishable from a
/// working one.
struct KeyRecordingRegister {
    seen: Arc<Mutex<Option<Option<OperationKey>>>>,
}

#[async_trait]
impl RegisterUser for KeyRecordingRegister {
    async fn register(
        &self,
        ctx: ServiceContext,
        input: RegisterInput,
    ) -> Result<RegisterOutput, RegisterUserError> {
        *self.seen.lock().expect("not poisoned") = Some(ctx.operation_key().cloned());
        Ok(RegisterOutput {
            user_id: input.user_id,
            tenant_id: input.tenant_id,
        })
    }
}

/// Counts every reservation attempt. A request refused at the boundary must
/// never reach the runtime's reservation path either — asserting zero here is
/// what distinguishes "rejected before dispatch" from "rejected somewhere after
/// having already started the operation".
#[derive(Default)]
struct CountingReservations {
    reserves: AtomicUsize,
    completes: AtomicUsize,
}

#[async_trait]
impl OperationReservationStore for CountingReservations {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        self.reserves.fetch_add(1, Ordering::SeqCst);
        // Permits the operation. A store that refused would make every
        // dispatched request fail for a reason that has nothing to do with the
        // header, and the zero-reserve assertion elsewhere would then pass even
        // if the boundary had let the request through.
        Ok(ReservationOutcome::Fresh(Lease {
            operation_id: OperationId::new(req.tenant.clone(), req.operation_key.clone()),
            owner_id: req.owner_id.clone(),
            fencing_token: FencingToken::initial(),
            lease_until: req.lease_until,
        }))
    }
    async fn renew(
        &self,
        _f: &ego_domain::operation::OwnerFence,
        _u: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), ReservationError> {
        unreachable!("nothing renews here")
    }
    /// Reached by B6.8's epilogue when an operation dispatched through this
    /// router succeeds — which is the whole chain working, from the header the
    /// client sent to the answer recorded for its replay.
    async fn complete(
        &self,
        _f: &ego_domain::operation::OwnerFence,
        _r: ego_domain::operation::StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        self.completes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn abandon(
        &self,
        _f: &ego_domain::operation::OwnerFence,
    ) -> Result<(), ReservationError> {
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

/// A router identical to production's except that `RegisterUser` resolves to
/// the recorder. The authentication provider is the real one `build_runtime`
/// constructs, so the request still has to authenticate the way any other does.
///
/// `mode` is the runtime's, and the extractor reads it from there — which is
/// why parameterising it here exercises the whole chain rather than the
/// extractor alone.
fn app_recording_under(
    mode: IdempotencyEnforcementMode,
    seen: Arc<Mutex<Option<Option<OperationKey>>>>,
    reservations: Arc<CountingReservations>,
) -> Router {
    let BuiltRuntime {
        authn,
        read_side: read_side_handles,
        ..
    } = build_runtime(&AppConfig::default()).expect("build_runtime succeeds");

    let service: Arc<dyn RegisterUser> = Arc::new(KeyRecordingRegister { seen });
    let mut builder = RuntimeBuilder::new().with_idempotency_enforcement_mode(mode);
    if matches!(mode, IdempotencyEnforcementMode::MandatoryKey) {
        // The builder refuses `MandatoryKey` with nowhere to reserve. Registering
        // a store is what makes that mode *buildable* — it is not what makes the
        // header required, which is why the store below expects to be called
        // zero times.
        builder = builder
            .with_operation_reservation_store(reservations.clone())
            .with_reservation_owner_id(OwnerId::new("under-test"))
            .with_reservation_lease_duration(Duration::from_secs(30));
    }
    let runtime = builder
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        // `register` is guarded by `#[authorize]` and `#[tenant_scoped]`, so a
        // runtime with no security capability refuses it before the key is ever
        // looked at. Authenticating for real and allowing every permission keeps
        // those guards on the path without making this a security test.
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

fn body() -> Body {
    Body::from(
        serde_json::json!({
            "user_id": "user-1",
            "email": "user@example.com",
            "tenant_id": "tenant-a",
            "org_name": "Acme",
        })
        .to_string(),
    )
}

fn post(key: Option<&str>) -> Request<Body> {
    let token = make_token("user-1", "tenant-a");
    let mut request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = key {
        request = request.header("Idempotency-Key", key);
    }
    request.body(body()).unwrap()
}

/// The assertion the whole boundary exists for: the key the client put on the
/// wire is the key the operation runs under — parsed, and otherwise untouched.
///
/// **This is the test a dropped `.with_operation_key(..)` fails.** Every
/// extractor test still passes in that case, because the extractor is not what
/// broke.
#[tokio::test]
async fn the_clients_idempotency_key_reaches_the_operation() {
    let seen = Arc::new(Mutex::new(None));
    let response = app_recording_under(
        IdempotencyEnforcementMode::Compatibility,
        seen.clone(),
        Arc::new(CountingReservations::default()),
    )
    .oneshot(post(Some("op-from-the-wire")))
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let observed = seen
        .lock()
        .expect("not poisoned")
        .clone()
        .expect("the operation was invoked");
    assert_eq!(
        observed.expect("it must carry a key").as_str(),
        "op-from-the-wire",
        "the value the client sent, not one regenerated, normalised or minted \
         along the way"
    );
}

/// The negative control, which is what keeps the assertion above from passing
/// for the wrong reason. Under this deployment's `Compatibility` declaration a
/// keyless request still dispatches — and arrives carrying **no** key, rather
/// than one invented to fill the gap.
#[tokio::test]
async fn a_request_without_the_header_reaches_the_operation_carrying_no_key() {
    let seen = Arc::new(Mutex::new(None));
    let response = app_recording_under(
        IdempotencyEnforcementMode::Compatibility,
        seen.clone(),
        Arc::new(CountingReservations::default()),
    )
    .oneshot(post(None))
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let observed = seen
        .lock()
        .expect("not poisoned")
        .clone()
        .expect("the operation was invoked");
    assert!(
        observed.is_none(),
        "a key the caller never supplied must not be manufactured for them"
    );
}

/// A supplied-but-unusable key is refused at the boundary under every mode, so
/// the operation is never reached at all. Asserted through the recorder rather
/// than the status code alone: a 400 returned *after* dispatching would look
/// identical from outside.
#[tokio::test]
async fn an_unusable_key_is_refused_without_reaching_the_operation() {
    let seen = Arc::new(Mutex::new(None));
    let response = app_recording_under(
        IdempotencyEnforcementMode::Compatibility,
        seen.clone(),
        Arc::new(CountingReservations::default()),
    )
    .oneshot(post(Some("   ")))
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        seen.lock().expect("not poisoned").is_none(),
        "the operation must not have been invoked at all"
    );
}

// ---------------------------------------------------------------------------
// The promise B6.5 exists for, through the real router
// ---------------------------------------------------------------------------

/// **A deployment that requires an operation key does not dispatch a request
/// without one.** This is B6.5's central claim, and until now it was only shown
/// by calling the extractor directly — which proves the extractor's policy, not
/// the router's behaviour, and says nothing about whether the operation ran.
///
/// That is the same gap that hid a dropped handler transfer: a component tested
/// in isolation can be perfect while the chain it sits in does the wrong thing.
///
/// Three observations, because the status code alone is not enough. A 400
/// returned *after* dispatching, or after reserving, would look identical from
/// outside the process.
#[tokio::test]
async fn a_missing_key_is_refused_by_the_router_under_mandatory_key() {
    let seen = Arc::new(Mutex::new(None));
    let reservations = Arc::new(CountingReservations::default());

    let response = app_recording_under(
        IdempotencyEnforcementMode::MandatoryKey,
        seen.clone(),
        reservations.clone(),
    )
    .oneshot(post(None))
    .await
    .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a runtime that requires a key must refuse a request that carries none"
    );
    assert!(
        seen.lock().expect("not poisoned").is_none(),
        "the operation must never have been invoked — this is the assertion that \
         separates 'refused before dispatch' from 'refused after running'"
    );
    assert_eq!(
        reservations.reserves.load(Ordering::SeqCst),
        0,
        "and nothing may have been reserved either: a refusal at the boundary \
         leaves no lease behind for a legitimate retry to contend with"
    );
    assert_eq!(reservations.completes.load(Ordering::SeqCst), 0);
}

/// The control that keeps the case above from passing for the wrong reason: the
/// *same* runtime, the *same* store, and a key that is present. It dispatches,
/// carries the key, and does reach the reservation path — so the refusal above
/// is attributable to the missing header rather than to the enforcing runtime
/// rejecting everything.
#[tokio::test]
async fn the_same_mandatory_runtime_dispatches_a_request_that_carries_a_key() {
    let seen = Arc::new(Mutex::new(None));
    let reservations = Arc::new(CountingReservations::default());

    let _response = app_recording_under(
        IdempotencyEnforcementMode::MandatoryKey,
        seen.clone(),
        reservations.clone(),
    )
    .oneshot(post(Some("op-mandatory-ok")))
    .await
    .unwrap();

    let observed = seen
        .lock()
        .expect("not poisoned")
        .clone()
        .expect("the operation was invoked");
    assert_eq!(
        observed.expect("carrying a key").as_str(),
        "op-mandatory-ok",
        "the enforcing runtime admits a well-formed key and passes it through"
    );
    assert_eq!(
        reservations.reserves.load(Ordering::SeqCst),
        1,
        "and it reserves under it — which is what makes the zero above meaningful"
    );
    assert_eq!(
        reservations.completes.load(Ordering::SeqCst),
        1,
        "and records the answer for a later replay: the header a client sent \
         reaches the reservation, the operation, and the completion that makes \
         the next identical request replayable — the whole chain, over HTTP"
    );
}
