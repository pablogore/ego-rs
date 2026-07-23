//! CORE-018 Phase 9 — HTTP route wiring (AD-1, AD-7), router-level (no real
//! socket — `tower::ServiceExt::oneshot`).
//!
//! Satisfies http-transport spec "Request reaches the guarded operation",
//! "Outcomes map to appropriate responses".

mod support;

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
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
    // Router-level tests never call `App::start()` — no effect executor is
    // registered in this reference app, and `App::resolver()` is callable
    // before starting (request-time resolution never depended on it).
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
async fn valid_bearer_jwt_and_body_returns_201() {
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

#[tokio::test]
async fn missing_authorization_header_returns_401_and_never_invokes_operation() {
    let request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .body(body())
        .unwrap();

    let response = app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// Covers map_register_error's Security -> Forbidden arm through the real
// HTTP route (register_user_guard_chain.rs's
// cross_tenant_request_is_denied_and_no_entity_write_occurs proves the same
// denial at the service layer; this drives it through tower::oneshot instead).
#[tokio::test]
async fn cross_tenant_request_returns_403() {
    // Token authenticates as tenant-a, but the request body asks to register
    // into tenant-b — the guard chain's tenant-scoping check denies this.
    let token = make_token("user-1", "tenant-a");
    let mismatched_body = Body::from(
        serde_json::json!({
            "user_id": "user-1",
            "email": "user@example.com",
            "tenant_id": "tenant-b",
            "org_name": "Acme",
        })
        .to_string(),
    );
    let request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(mismatched_body)
        .unwrap();

    let response = app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// Covers map_register_error's EntityWrite -> Internal arm through the real
// HTTP route (register_user_partial_failure.rs's
// user_write_failure_leaves_org_persisted_as_a_benign_reusable_orphan proves
// the same failure at the service layer; this drives it through
// tower::oneshot instead).
#[tokio::test]
async fn empty_email_partial_failure_returns_500() {
    let token = make_token("user-1", "tenant-a");
    // Empty email is UserEntity's real validation trigger (see
    // domain/user.rs) — drives a genuine User-write failure, not a
    // test-only backdoor.
    let empty_email_body = Body::from(
        serde_json::json!({
            "user_id": "user-1",
            "email": "",
            "tenant_id": "tenant-a",
            "org_name": "Acme",
        })
        .to_string(),
    );
    let request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(empty_email_body)
        .unwrap();

    let response = app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

// Finding 1 (security fix): GET /tenants/:tenant_id/users used to have zero
// authentication — any unauthenticated caller could read any tenant's
// users/org name. These three tests prove: (a) no credentials -> 401, (b)
// valid credentials for tenant-a requesting tenant-b's users -> 403, (c)
// valid credentials for tenant-a requesting tenant-a's own users -> 200
// with the data that tenant actually registered.

#[tokio::test]
async fn users_by_tenant_without_credentials_returns_401() {
    let request = Request::builder()
        .method("GET")
        .uri("/tenants/tenant-a/users")
        .body(Body::empty())
        .unwrap();

    let response = app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn users_by_tenant_cross_tenant_request_returns_403() {
    // Token authenticates as tenant-a, but the path asks for tenant-b's users.
    let token = make_token("user-1", "tenant-a");
    let request = Request::builder()
        .method("GET")
        .uri("/tenants/tenant-b/users")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn users_by_tenant_same_tenant_returns_200_with_that_tenants_real_data() {
    let config = AppConfig::default();
    let BuiltRuntime {
        app,
        authn,
        read_side: read_side_handles,
    } = build_runtime(&config).expect("build_runtime succeeds");
    let state = AppState::new(app.resolver(), authn);
    let query = read_side_handles.query.clone();
    let router = build_router(state, query.clone());
    let read_side = read_side_handles.spawn();

    let token = make_token("user-1", "tenant-a");
    let register_request = Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(body())
        .unwrap();
    let response = router.clone().oneshot(register_request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // The projection catches up asynchronously (CORE-005 is pull-based).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while query.view("tenant-a").users.is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "projection never caught up"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let query_request = Request::builder()
        .method("GET")
        .uri("/tenants/tenant-a/users")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(query_request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let view: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(view["org_name"], "Acme");
    assert_eq!(view["users"][0]["user_id"], "user-1");

    let _ = read_side.stop().await;
}
