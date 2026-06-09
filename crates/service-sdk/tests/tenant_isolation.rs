//! Tests for tenant isolation functionality.

use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::tenant::TenantId;

#[tokio::test]
async fn test_tenant_isolation() {
    let tenant_a = TenantId {
        id: "tenant-a".to_string(),
    };
    let tenant_b = TenantId {
        id: "tenant-b".to_string(),
    };

    let context_a = ServiceContext::new().with_tenant_id(tenant_a.id.clone());
    let context_b = ServiceContext::new().with_tenant_id(tenant_b.id.clone());

    // Test that different tenants are properly identified
    assert_eq!(context_a.tenant_id(), Some("tenant-a"));
    assert_eq!(context_b.tenant_id(), Some("tenant-b"));

    // Test that cross-tenant access is not allowed by default
    assert!(!context_a.is_cross_tenant_allowed());
    assert!(!context_b.is_cross_tenant_allowed());

    // Test that cross-tenant access can be explicitly allowed
    let context_with_cross_tenant = context_a.allow_cross_tenant();
    assert!(context_with_cross_tenant.is_cross_tenant_allowed());
}
