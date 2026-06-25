use ego_service_sdk::context::ServiceContext;

#[test]
fn is_cross_tenant_allowed_defaults_to_false() {
    let ctx = ServiceContext::new();
    assert!(!ctx.is_cross_tenant_allowed());
}

#[test]
fn cross_tenant_escalation_is_rejected_at_compile_time() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/with_cross_tenant_access_external.rs");
    t.compile_fail("tests/compile_fail/allow_cross_tenant_field.rs");
}
