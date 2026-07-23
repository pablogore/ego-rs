//! PROD-003 Phase 5 (TASK-014a/b) — HTTP ingress trace-context origination,
//! router-level (no real socket — `tower::ServiceExt::oneshot`, mirrors
//! `http_route.rs`).
//!
//! `crates/transport/src/propagation.rs`'s `originate_trace_context` unit
//! tests already prove the exact parent-linkage/root/malformed-fallback
//! semantics in isolation; these tests prove the real `/register` ingress
//! point (`ports/http/handlers.rs::register_handler`) is actually wired to
//! it end-to-end and that a malformed inbound `traceparent` degrades to a
//! fresh root trace instead of failing the request (service-sdk spec:
//! "Trace-Context Originates At HTTP Ingress").

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use ego_domain::TraceContext;
use ego_transport::AppState;
use reference_app::ports::http::build_router;
use reference_app::{build_runtime, AppConfig, BuiltRuntime};
use support::make_token;
use tower::ServiceExt;

fn app() -> Router {
    let config = AppConfig::default();
    let BuiltRuntime {
        app,
        authn,
        read_side: read_side_handles,
    } = build_runtime(&config).expect("build_runtime succeeds");
    let state = AppState::new(app.resolver(), authn);
    build_router(state, read_side_handles.query.clone())
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
async fn valid_inbound_traceparent_still_reaches_the_operation() {
    let token = make_token("user-1", "tenant-a");
    let inbound = TraceContext::root().to_traceparent();
    let request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("traceparent", inbound)
        .body(body())
        .unwrap();

    let response = app().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn no_inbound_traceparent_still_reaches_the_operation() {
    let token = make_token("user-1", "tenant-a");
    let request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(body())
        .unwrap();

    let response = app().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

// Resilience (W3C: invalid traceparent treated as absent): a malformed
// inbound header MUST NOT fail the request.
#[tokio::test]
async fn malformed_inbound_traceparent_does_not_fail_the_request() {
    let token = make_token("user-1", "tenant-a");
    let request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("traceparent", "not-a-traceparent")
        .body(body())
        .unwrap();

    let response = app().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}
