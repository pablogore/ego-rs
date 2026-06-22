use std::sync::Arc;

use ego_security_sdk::{
    context::SecurityContext,
    principal::{Principal, PrincipalKind, SubjectId},
};
use ego_service_sdk::context::ServiceContext;

fn make_security_ctx(subject: &str) -> SecurityContext {
    let sub = SubjectId::new(subject).unwrap();
    let principal = Principal::new(PrincipalKind::User, sub);
    SecurityContext::new(principal)
}

#[test]
fn security_field_defaults_to_none() {
    let ctx = ServiceContext::new();
    assert!(ctx.security().is_none());
}

#[test]
fn security_field_set_via_builder() {
    let sec = Arc::new(make_security_ctx("user:alice"));
    let ctx = ServiceContext::new().with_security(Arc::clone(&sec));
    assert!(ctx.security().is_some());
}

#[test]
fn security_propagates_through_chain() {
    let sec = Arc::new(make_security_ctx("user:42"));
    let ctx = ServiceContext::new().with_security(Arc::clone(&sec));

    // Simulate passing ctx into a handler
    let handle = |service_ctx: &ServiceContext| {
        service_ctx
            .security()
            .unwrap()
            .principal()
            .subject
            .as_str()
            .to_owned()
    };

    assert_eq!(handle(&ctx), "user:42");
}

#[test]
fn existing_construction_sites_compile() {
    use std::collections::HashMap;
    use std::time::{Duration, SystemTime};

    // Every existing builder call must still compile after adding the security field.
    let _ = ServiceContext::new();
    let _ = ServiceContext::new().with_tenant_id("tenant-1");
    let _ = ServiceContext::new().with_correlation_id("corr-abc");
    let _ = ServiceContext::new().with_trace_id("trace-xyz");
    let _ = ServiceContext::new().with_deadline(SystemTime::now());
    let _ = ServiceContext::new().with_timeout(Duration::from_secs(30));
    let _ = ServiceContext::new().with_additional_context(HashMap::new());
    let _ = ServiceContext::new().allow_cross_tenant();
    // Chain combining existing and new builder methods
    let _ = ServiceContext::new()
        .with_tenant_id("t1")
        .with_security(Arc::new(make_security_ctx("user:svc")));
}
