//! TASK-007 (RED): integration test for `AuthenticatedContext`'s
//! `FromRequestParts` impl, wired to a real `Hs256AuthenticationProvider`
//! (reused from `security-jwt`, not reinvented).

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::Request;
use ego_domain::auth::SystemClock;
use ego_security_sdk::AuthenticationProvider;
use ego_service_sdk::runtime::RuntimeBuilder;
use ego_testkit::TestJwtBuilder;
use ego_transport::security::AuthenticatedContext;
use ego_transport::state::AppState;
use security_jwt::{
    Hs256AuthenticationProvider, JwtAlgorithm, JwtProviderConfig, LocalKeyResolver, VerificationKey,
};

/// NIST SP 800-107 minimum HMAC secret length is 32 bytes.
fn secret() -> Vec<u8> {
    b"integration-test-secret-32-bytes!".to_vec()
}

fn make_state() -> AppState {
    let resolver = Arc::new(LocalKeyResolver::new(
        JwtAlgorithm::Hs256,
        VerificationKey::Hmac(secret()),
    ));
    let config = JwtProviderConfig {
        // `expected_aud` is required (audience-confusion / token-reuse defense),
        // so a bare `default()` is rejected by `try_new`. Tokens minted below
        // carry a matching `aud`.
        expected_aud: Some(vec![TEST_AUD.to_string()]),
        ..JwtProviderConfig::default()
    };
    let provider: Arc<dyn AuthenticationProvider> = Arc::new(
        Hs256AuthenticationProvider::try_new(config, resolver, Arc::new(SystemClock))
            .expect("valid JWT provider config"),
    );
    let rt = RuntimeBuilder::new().build();
    AppState::new(rt.resolver(), provider)
}

/// Audience the provider's `expected_aud` requires and every minted token carries.
const TEST_AUD: &str = "transport-test-audience";

fn make_token(sub: &str, tenant_id: Option<&str>) -> String {
    let mut builder = TestJwtBuilder::new(secret())
        .subject(sub)
        .claim("aud", serde_json::Value::from(TEST_AUD));
    if let Some(t) = tenant_id {
        builder = builder.tenant_id(t);
    }
    builder.build()
}

fn parts_with_authorization(value: Option<&str>) -> axum::http::request::Parts {
    let mut builder = Request::builder().method("POST").uri("/register");
    if let Some(v) = value {
        builder = builder.header("authorization", v);
    }
    let (parts, ()) = builder.body(()).unwrap().into_parts();
    parts
}

#[tokio::test]
async fn missing_authorization_header_is_rejected_before_handler_runs() {
    let state = make_state();
    let mut parts = parts_with_authorization(None);
    let result = AuthenticatedContext::from_request_parts(&mut parts, &state).await;
    assert!(result.is_err(), "missing credentials must be rejected");
}

#[tokio::test]
async fn malformed_bearer_header_is_rejected() {
    let state = make_state();
    let mut parts = parts_with_authorization(Some("Bearer  double-space-token"));
    let result = AuthenticatedContext::from_request_parts(&mut parts, &state).await;
    assert!(result.is_err(), "malformed Bearer header must be rejected");
}

#[tokio::test]
async fn valid_jwt_produces_security_context_with_matching_claims() {
    let state = make_state();
    let token = make_token("user-42", Some("tenant-9"));
    let header_value = format!("Bearer {token}");
    let mut parts = parts_with_authorization(Some(&header_value));

    let AuthenticatedContext(ctx) = AuthenticatedContext::from_request_parts(&mut parts, &state)
        .await
        .expect("valid token must authenticate");

    assert_eq!(ctx.principal().subject_id.as_str(), "user-42");
    assert_eq!(
        ctx.principal().tenant_id.as_ref().map(|t| t.as_str()),
        Some("tenant-9")
    );
}
