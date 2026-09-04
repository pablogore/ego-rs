//! PROD-P1.1 — real-wire acceptance for `GET /health`/`GET /ready`, over a
//! real TCP socket via `ego_transport::serve` (the same CORE-018 chain
//! `wire_register_postgres.rs` proves for `/register`), never
//! `tower::ServiceExt::oneshot`.
//!
//! Three cases:
//! - Test 1: a healthy running app — `/health` returns 200.
//! - Test 2: the same healthy app — `/ready` returns 200 too (no contributor
//!   is registered in this in-memory composition, so this is a lighter
//!   sibling of the Postgres-backed Required Test 2 in
//!   `integration-tests/tests/infrastructure/wire_health_readiness_postgres.rs`).
//! - Test 3: a deterministic not-ready transition, proven with the existing
//!   `ego_testkit::StaticHealthContributor` test double registered on a bare
//!   `RuntimeBuilder` via `.with_lifecycle_component(...)` — the health
//!   model's own established mechanism for this, not a brittle
//!   container-killing test. Also proves `/health` stays 200 while `/ready`
//!   is 503: liveness is unaffected by readiness.

use std::future::pending;
use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use ego_domain::health::{DependencyRequirement, HealthStatus};
use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::credential::Credential;
use ego_security_sdk::AuthenticationError;
use ego_service_sdk::implementation::LifecycleManaged;
use ego_service_sdk::runtime::{IdempotencyEnforcementMode, RuntimeBuilder};
use ego_testkit::StaticHealthContributor;
use ego_transport::AppState;
use reference_app::ports::http::{health_handler, ready_handler};
use reference_app::{build_runtime_in_memory, AppConfig, BuiltRuntime};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Never actually called — `/health` and `/ready` require no credentials.
struct UnusedAuthn;
impl AuthenticationProvider for UnusedAuthn {
    fn authenticate(&self, _: &Credential) -> Result<SecurityContext, AuthenticationError> {
        Err(AuthenticationError::InvalidToken("unused in this test".into()))
    }
}

async fn spawn(router: Router) -> (std::net::SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binds a real, ephemeral TCP socket");
    let addr = listener
        .local_addr()
        .expect("a bound listener reports a local address");
    let handle = tokio::spawn(async move {
        let _ = ego_transport::serve(listener, router, pending()).await;
    });
    (addr, handle)
}

#[tokio::test(flavor = "multi_thread")]
async fn healthy_running_app_returns_200_on_health_and_ready() {
    let BuiltRuntime {
        app,
        authn,
        read_side,
        ..
    } = build_runtime_in_memory(&AppConfig::default()).expect("build_runtime succeeds");
    let state = AppState::new(app.resolver(), authn);
    let router = reference_app::ports::http::build_router(state, read_side.query.clone());
    let (addr, server) = spawn(router).await;

    let client = reqwest::Client::new();

    let health = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("a real HTTP client reaches the bound socket");
    assert_eq!(health.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = health.json().await.expect("valid JSON body");
    assert_eq!(body["status"], "healthy");

    let ready = client
        .get(format!("http://{addr}/ready"))
        .send()
        .await
        .expect("a real HTTP client reaches the bound socket");
    assert_eq!(ready.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = ready.json().await.expect("valid JSON body");
    assert_eq!(body["status"], "ready");

    server.abort();
}

/// An always-unhealthy component, wired the same way a real infra
/// contributor would be — through `LifecycleManaged::health_contributors()`
/// — proving the not-ready transition through the actual aggregation path,
/// not a stand-in.
struct AlwaysUnhealthy;

#[async_trait::async_trait]
impl LifecycleManaged for AlwaysUnhealthy {
    fn health_contributors(&self) -> Vec<Arc<dyn ego_domain::health::HealthContributor>> {
        vec![Arc::new(StaticHealthContributor::new(
            HealthStatus::Unhealthy,
            DependencyRequirement::Required,
        ))]
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn required_dependency_unhealthy_makes_ready_503_while_health_stays_200() {
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
        .with_lifecycle_component(Arc::new(AlwaysUnhealthy))
        .build();
    let state = AppState::new(runtime.resolver(), Arc::new(UnusedAuthn));
    let router = Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .with_state(state);
    let (addr, server) = spawn(router).await;

    let client = reqwest::Client::new();

    let health = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("a real HTTP client reaches the bound socket");
    assert_eq!(
        health.status(),
        reqwest::StatusCode::OK,
        "liveness must not be affected by a failing dependency"
    );

    let ready = client
        .get(format!("http://{addr}/ready"))
        .send()
        .await
        .expect("a real HTTP client reaches the bound socket");
    assert_eq!(ready.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = ready.json().await.expect("valid JSON body");
    assert_eq!(body["status"], "not_ready");

    server.abort();
}
