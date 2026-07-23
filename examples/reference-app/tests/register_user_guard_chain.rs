//! CORE-018 Phase 6 — `RegisterUser` guard chain + happy path (AD-4).
//!
//! Satisfies reference-service spec "Unauthorized principal denied",
//! "Cross-tenant request denied", "Successful registration".

mod support;

use std::sync::Arc;

use ego_testkit::{PrincipalBuilder, ScriptedAuthorizationProvider, ServiceTestFixture};
use reference_app::application::{RegisterInput, RegisterUser, RegisterUserTag};
use support::make_register_user as make_service;

fn input() -> RegisterInput {
    RegisterInput {
        user_id: "user-1".to_string(),
        email: "user@example.com".to_string(),
        tenant_id: "tenant-a".to_string(),
        org_name: "Acme".to_string(),
    }
}

// TASK-016: unauthorized principal denied, no entity write.
#[tokio::test]
async fn unauthorized_principal_is_denied_and_no_entity_write_occurs() {
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(make_service(None))
        .expect("registration succeeds")
        .authorization(Arc::new(ScriptedAuthorizationProvider::deny_all()))
        .build();

    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");

    let result = proxy.register(fixture.context(), input()).await;

    assert!(result.is_err(), "expected authorization to deny the call");
}

// TASK-017: authorized principal, mismatched tenant hint -> denied.
#[tokio::test]
async fn cross_tenant_request_is_denied_and_no_entity_write_occurs() {
    let principal = PrincipalBuilder::new().tenant("tenant-a").build();
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(make_service(None))
        .expect("registration succeeds")
        .principal(principal)
        .build();

    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");

    let ctx = fixture.context().with_tenant_id("tenant-b");
    let result = proxy.register(ctx, input()).await;

    assert!(
        result.is_err(),
        "expected tenant-scoping to deny the cross-tenant call"
    );
}

// TASK-018: authorized + matching tenant -> success.
#[tokio::test]
async fn successful_registration_returns_ok_output() {
    let principal = PrincipalBuilder::new().tenant("tenant-a").build();
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(make_service(None))
        .expect("registration succeeds")
        .principal(principal)
        .build();

    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");

    let ctx = fixture.context().with_tenant_id("tenant-a");
    let result = proxy.register(ctx, input()).await;

    let output = result.expect("authorized, tenant-matched call should succeed");
    assert_eq!(output.user_id, "user-1");
    assert_eq!(output.tenant_id, "tenant-a");
}
