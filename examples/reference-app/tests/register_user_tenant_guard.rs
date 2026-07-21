//! `RegisterUser::register`'s fail-closed write-tenant guard.
//!
//! `#[tenant_scoped]` only proves `ctx` resolves to SOME tenant; it never
//! compares that canonical tenant against the client-controlled
//! `input.tenant_id` request-body field. `RegisterUserImpl::register` adds an
//! explicit guard: the only accepted case is a resolved canonical tenant that
//! equals `input.tenant_id`. This file pins that guard down:
//!
//! - (a) no canonical tenant resolved (`None`, e.g. a direct call that
//!   bypassed the `#[tenant_scoped]` macro proxy) -> `AuthorizationDenied`.
//! - (b) a resolved canonical tenant that disagrees with `input.tenant_id`
//!   -> `AuthorizationDenied`.
//! - (c) a resolved canonical tenant that equals `input.tenant_id` -> `Ok`.
//!
//! Case (a) exercises the impl directly (no proxy, so `enforce_tenant` never
//! runs and `canonical_tenant()` stays `None`). Cases (b)/(c) go through the
//! real guarded proxy path (`ServiceTestFixture` -> `RegisterUserTag`), the
//! only public seam that produces a resolved canonical tenant — there is no
//! public setter for it.

mod support;

use ego_security_sdk::SecurityError;
use ego_service_sdk::context::ServiceContext;
use ego_testkit::{PrincipalBuilder, ServiceTestFixture};
use reference_app::application::{RegisterInput, RegisterUser, RegisterUserError, RegisterUserTag};

fn input(tenant_id: &str) -> RegisterInput {
    RegisterInput {
        user_id: "user-1".to_string(),
        email: "user@example.com".to_string(),
        tenant_id: tenant_id.to_string(),
        org_name: "Acme".to_string(),
    }
}

// (a) No canonical tenant resolved -> denied. A direct call on the impl never
// runs `#[tenant_scoped]`'s `enforce_tenant`, so `canonical_tenant()` is
// `None`; the guard must deny rather than trust the request-body tenant_id.
#[tokio::test]
async fn denies_when_no_canonical_tenant_is_resolved() {
    let service = support::make_register_user(None);

    // A caller-supplied tenant hint, but no resolved canonical tenant.
    let ctx = ServiceContext::new().with_tenant_id("tenant-a");
    let err = service.register(ctx, input("tenant-a")).await.unwrap_err();

    assert!(
        matches!(err, RegisterUserError::Security(SecurityError::AuthorizationDenied { .. })),
        "missing canonical tenant must be denied, got: {err:?}"
    );
}

// (b) Resolved canonical tenant disagrees with input.tenant_id -> denied.
// Principal + hint agree on tenant-a (so `enforce_tenant` resolves and passes),
// but the request body claims tenant-b. The guard inside `register` catches it
// even though `#[tenant_scoped]` was satisfied.
#[tokio::test]
async fn denies_when_resolved_tenant_disagrees_with_input_tenant_id() {
    let principal = PrincipalBuilder::new().tenant("tenant-a").build();
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(support::make_register_user(None))
        .expect("registration succeeds")
        .principal(principal)
        .build();

    let proxy = fixture.resolve::<RegisterUserTag>().expect("registered tag resolves");
    let ctx = fixture.context().with_tenant_id("tenant-a");

    let err = proxy.register(ctx, input("tenant-b")).await.unwrap_err();

    assert!(
        matches!(err, RegisterUserError::Security(SecurityError::AuthorizationDenied { .. })),
        "resolved tenant != input.tenant_id must be denied, got: {err:?}"
    );
}

// (c) Resolved canonical tenant equals input.tenant_id -> Ok.
#[tokio::test]
async fn allows_when_resolved_tenant_matches_input_tenant_id() {
    let principal = PrincipalBuilder::new().tenant("tenant-a").build();
    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(support::make_register_user(None))
        .expect("registration succeeds")
        .principal(principal)
        .build();

    let proxy = fixture.resolve::<RegisterUserTag>().expect("registered tag resolves");
    let ctx = fixture.context().with_tenant_id("tenant-a");

    let output = proxy.register(ctx, input("tenant-a")).await.expect("matching tenant should succeed");

    assert_eq!(output.user_id, "user-1");
    assert_eq!(output.tenant_id, "tenant-a");
}
