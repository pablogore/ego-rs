//! PROD-003 Phase 5 (G2 review fix on PR #216) — proves the HTTP-ingress
//! originated `TraceContext` actually crosses the FULL boundary: HTTP
//! request -> `TraceContextExtractor` -> `ServiceContext::with_trace_context`
//! -> the macro-generated proxy's `chain.on_request` -> `TracingInterceptor`
//! -> `Tracer::start_span`.
//!
//! `ingress_trace_context.rs`'s status==201-only tests do NOT prove this: if
//! someone deleted the handler's `.with_trace_context(trace_context)` call,
//! those tests would still pass (the operation still succeeds; it just never
//! carries a trace). This test captures the FULL `TraceContext` a real spy
//! `Tracer` receives and asserts trace_id/span_id/parent_span_id — it goes
//! red the moment that wiring is removed.

mod support;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use ego_domain::auth::{AuthenticationError, SystemClock};
use ego_domain::{SpanAttributes, SpanId, SpanOutcome, TraceContext, Tracer};
use ego_security_sdk::{
    AccessRequest, AuthenticationProvider, AuthorizationDecision, AuthorizationProvider,
    Credential, Principal, SecurityContext, SecurityError,
};
use ego_service_sdk::runtime::RuntimeBuilder;
use ego_transport::AppState;
use persistent_entity::builder::EntityRuntimeBuilder;
use reference_app::application::{RegisterUserImpl, RegisterUserTag};
use reference_app::ports::http::build_router;
use reference_app::read_side::UsersByTenantStore;
use reference_app::{DEV_SIGNING_KEY, REFERENCE_APP_AUDIENCE};
use security_jwt::{
    Hs256AuthenticationProvider, JwtAlgorithm, JwtProviderConfig, LocalKeyResolver, VerificationKey,
};
use support::make_token;
use tower::ServiceExt;

/// Spy `Tracer`: captures the FULL `TraceContext` every `start_span` call
/// receives (not just a call count), so the test can assert
/// trace_id/span_id/parent_span_id end-to-end.
#[derive(Default)]
struct SpyTracer {
    spans: Mutex<Vec<TraceContext>>,
    names: Mutex<Vec<String>>,
}

impl Tracer for SpyTracer {
    fn start_span(&self, ctx: &TraceContext, name: &str, _attrs: SpanAttributes) {
        self.spans.lock().unwrap().push(*ctx);
        self.names.lock().unwrap().push(name.to_string());
    }

    fn end_span(&self, _span: SpanId, _outcome: SpanOutcome) {}
}

/// Minimal always-allow `AuthorizationProvider` test double. Reference-app's
/// own `ReferenceAllowAllAuthorization` (`lib.rs`) is private, and
/// `ego_security_sdk::AllowAllAuthorizationProvider` lives behind the
/// `dev-providers`/`test-helpers` Cargo feature — reference-app's `lib.rs`
/// already documents why depending on that feature here would unify it into
/// every workspace member's build. This test only proves ingress trace
/// wiring, not authorization policy.
struct AllowAll;

#[async_trait]
impl AuthorizationProvider for AllowAll {
    async fn authorize(
        &self,
        _principal: &Principal,
        _request: &AccessRequest,
        _ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Allow)
    }
}

/// Never exercised: `#[authorize]`'s guard reads only
/// `RuntimeInner::authorization_provider()` (the `authz` half of
/// `RuntimeBuilder::with_security`) — real authentication for this request
/// happens through `AppState::authn` (`AuthenticatedContext`) before the
/// handler runs, never through this runtime-registered `authn` half.
struct UnusedAuthn;

impl AuthenticationProvider for UnusedAuthn {
    fn authenticate(
        &self,
        _credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        unimplemented!(
            "not exercised by this test — real authentication happens via AppState::authn"
        )
    }
}

/// Builds the real `/register` router, wired to `tracer` via
/// `RuntimeBuilder::with_tracer` — the same wiring path Phase 4 already
/// proved drives `TracingInterceptor` (`with_tracer_wires_a_tracing_interceptor_into_the_chain`
/// in `crates/service-sdk/src/runtime/builder.rs`), now exercised through a
/// real HTTP request instead of a direct `on_request` call.
fn app_with_tracer(tracer: Arc<SpyTracer>) -> Router {
    let org_runtime = Arc::new(EntityRuntimeBuilder::new().build());
    let user_runtime = Arc::new(EntityRuntimeBuilder::new().build());
    let register_user = Arc::new(RegisterUserImpl::new(org_runtime, user_runtime, None));

    let rt = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .with_security(Arc::new(UnusedAuthn), Arc::new(AllowAll))
        .with_tracer(tracer as Arc<dyn Tracer>)
        .with_service::<RegisterUserTag>(register_user)
        .expect("registers cleanly")
        .build();

    let resolver = Arc::new(LocalKeyResolver::new(
        JwtAlgorithm::Hs256,
        VerificationKey::Hmac(DEV_SIGNING_KEY.to_vec()),
    ));
    let jwt_config = JwtProviderConfig {
        expected_aud: Some(vec![REFERENCE_APP_AUDIENCE.to_string()]),
        ..JwtProviderConfig::default()
    };
    let authn: Arc<dyn AuthenticationProvider> = Arc::new(
        Hs256AuthenticationProvider::try_new(jwt_config, resolver, Arc::new(SystemClock))
            .expect("valid JWT provider config"),
    );

    let state = AppState::new(rt.resolver(), authn);
    build_router(state, UsersByTenantStore::default())
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

#[tokio::test]
async fn valid_inbound_traceparent_crosses_the_full_boundary_to_the_tracer() {
    let tracer = Arc::new(SpyTracer::default());
    let router = app_with_tracer(tracer.clone());

    let remote = TraceContext::root();
    let inbound = remote.to_traceparent();
    let token = make_token("user-1", "tenant-a");
    let request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("traceparent", inbound)
        .body(body())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let spans = tracer.spans.lock().unwrap();
    assert_eq!(
        spans.len(),
        1,
        "TracingInterceptor must start exactly one span for this request"
    );
    let captured = spans[0];
    assert_eq!(
        captured.trace_id(),
        remote.trace_id(),
        "the originated trace must continue the inbound trace"
    );
    assert_eq!(
        captured.parent_span_id(),
        Some(remote.span_id()),
        "the originated span's parent must be the inbound remote span"
    );
    assert_ne!(
        captured.span_id(),
        remote.span_id(),
        "the originated span must be a FRESH local span, not the remote one"
    );

    // #212 (PROD-003 follow-up): the generated proxy stamps the dispatched
    // operation name onto the context, so the span is named after the method
    // (`register`) end-to-end rather than the generic fallback.
    let names = tracer.names.lock().unwrap();
    assert_eq!(
        names.as_slice(),
        &["register".to_string()],
        "the request-boundary span must be named after the dispatched operation"
    );
}

#[tokio::test]
async fn absent_inbound_traceparent_crosses_the_boundary_as_a_fresh_root() {
    let tracer = Arc::new(SpyTracer::default());
    let router = app_with_tracer(tracer.clone());

    let token = make_token("user-1", "tenant-a");
    let request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(body())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let spans = tracer.spans.lock().unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(
        spans[0].parent_span_id(),
        None,
        "no inbound traceparent must originate a fresh root trace that reaches the tracer"
    );
}

#[tokio::test]
async fn malformed_inbound_traceparent_crosses_the_boundary_as_a_fresh_root() {
    let tracer = Arc::new(SpyTracer::default());
    let router = app_with_tracer(tracer.clone());

    let token = make_token("user-1", "tenant-a");
    let request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("traceparent", "not-a-traceparent")
        .body(body())
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let spans = tracer.spans.lock().unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(
        spans[0].parent_span_id(),
        None,
        "a malformed traceparent must degrade to a fresh root trace that reaches the tracer, never error"
    );
}
