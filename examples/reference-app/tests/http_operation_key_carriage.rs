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

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use ego_domain::operation::OperationKey;
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

/// A router identical to production's except that `RegisterUser` resolves to
/// the recorder. The authentication provider is the real one `build_runtime`
/// constructs, so the request still has to authenticate the way any other does.
fn app_recording(seen: Arc<Mutex<Option<Option<OperationKey>>>>) -> Router {
    let BuiltRuntime {
        authn,
        read_side: read_side_handles,
        ..
    } = build_runtime(&AppConfig::default()).expect("build_runtime succeeds");

    let service: Arc<dyn RegisterUser> = Arc::new(KeyRecordingRegister { seen });
    let runtime = RuntimeBuilder::new()
        // The same declaration production makes, for the same reason: this
        // reference app has no durable reservation store, and pretending
        // otherwise here would be the adoption claim `build_runtime` refuses to
        // make. What is under test is carriage, not enforcement.
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
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
    let response = app_recording(seen.clone())
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
    let response = app_recording(seen.clone())
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
    let response = app_recording(seen.clone())
        .oneshot(post(Some("   ")))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        seen.lock().expect("not poisoned").is_none(),
        "the operation must not have been invoked at all"
    );
}
