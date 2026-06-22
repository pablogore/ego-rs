use std::sync::Arc;

use ego_security_sdk::{
    context::SecurityContext,
    principal::{Principal, PrincipalKind, SubjectId},
};
use ego_service_sdk::context::ServiceContext;

fn make_principal(subject: &str) -> Principal {
    let sub = SubjectId::new(subject).unwrap();
    Principal::new(PrincipalKind::User, sub)
}

#[test]
fn security_context_carried_via_explicit_passing() {
    let sec_ctx = Arc::new(SecurityContext::new(make_principal("user:alice")));
    let svc_ctx = ServiceContext::new().with_security(Arc::clone(&sec_ctx));

    let handler = |ctx: &ServiceContext| {
        ctx.security
            .as_ref()
            .unwrap()
            .principal()
            .subject
            .as_str()
            .to_owned()
    };

    assert_eq!(handler(&svc_ctx), "user:alice");
}

#[test]
fn two_independent_contexts_do_not_share_state() {
    let ctx_a = ServiceContext::new()
        .with_security(Arc::new(SecurityContext::new(make_principal("user:alice"))));
    let ctx_b = ServiceContext::new()
        .with_security(Arc::new(SecurityContext::new(make_principal("user:bob"))));

    let sub_a = ctx_a
        .security
        .as_ref()
        .unwrap()
        .principal()
        .subject
        .as_str()
        .to_owned();
    let sub_b = ctx_b
        .security
        .as_ref()
        .unwrap()
        .principal()
        .subject
        .as_str()
        .to_owned();

    assert_eq!(sub_a, "user:alice");
    assert_eq!(sub_b, "user:bob");
    assert_ne!(sub_a, sub_b);
}

#[test]
fn inv_007_clone_preserves_security_field() {
    let arc = Arc::new(SecurityContext::new(make_principal("user:alice")));
    let original = ServiceContext::new().with_security(Arc::clone(&arc));
    let cloned = original.clone();

    assert!(cloned.security.is_some());
    assert!(Arc::ptr_eq(
        original.security.as_ref().unwrap(),
        cloned.security.as_ref().unwrap()
    ));
}
