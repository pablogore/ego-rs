use ego_service_sdk::context::ServiceContext;

#[test]
fn is_cross_tenant_allowed_defaults_to_false() {
    let ctx = ServiceContext::new();
    assert!(!ctx.is_cross_tenant_allowed());
}

// If these tests fail — either after a signature change on with_cross_tenant_access or after a
// Rust toolchain upgrade that changes compiler error wording — regenerate the fixture files with:
//   TRYBUILD=overwrite cargo test --test cross_tenant_access_contract
#[test]
fn cross_tenant_api_visibility_contract() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/with_cross_tenant_access_external.rs");
    t.compile_fail("tests/compile_fail/allow_cross_tenant_field.rs");
    t.compile_fail("tests/compile_fail/cross_tenant_permit_foreign_construction.rs");
    t.compile_fail("tests/compile_fail/cross_tenant_permit_new_external.rs");
}
