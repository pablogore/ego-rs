//! Tests for tenant isolation functionality.

use ego_service_sdk::context::ServiceContext;

#[tokio::test]
async fn test_tenant_isolation() {
    let context_a = ServiceContext::new().with_tenant_id("tenant-a");
    let context_b = ServiceContext::new().with_tenant_id("tenant-b");

    assert_eq!(context_a.tenant_id(), Some("tenant-a"));
    assert_eq!(context_b.tenant_id(), Some("tenant-b"));

    // Cross-tenant access is not allowed by default.
    assert!(!context_a.is_cross_tenant_allowed());
    assert!(!context_b.is_cross_tenant_allowed());

    // Cross-tenant access can be explicitly allowed.
    let context_with_cross_tenant = context_a.allow_cross_tenant();
    assert!(context_with_cross_tenant.is_cross_tenant_allowed());
}
