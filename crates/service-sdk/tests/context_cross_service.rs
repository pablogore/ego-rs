//! Tests for cross-service context propagation functionality.
//!
//! After CORE-010A, context crossing a service boundary is explicit:
//! the caller passes `ctx` as a parameter or via clone, and the callee
//! receives it as an owned value with no ambient read.

use ego_service_sdk::context::ServiceContext;

#[tokio::test]
async fn test_context_cross_service_explicit() {
    let ctx = ServiceContext::new()
        .with_tenant_id("tenant-123")
        .with_correlation_id("correlation-456");

    // Simulate service boundary: clone and pass explicitly
    async fn service_b(ctx: ServiceContext) -> (Option<String>, Option<String>) {
        (
            ctx.tenant_id().map(|s| s.to_owned()),
            ctx.correlation_id().map(|s| s.to_owned()),
        )
    }

    let (tenant, correlation) = service_b(ctx.clone()).await;
    assert_eq!(tenant.as_deref(), Some("tenant-123"));
    assert_eq!(correlation.as_deref(), Some("correlation-456"));
}

#[tokio::test]
async fn test_context_cross_service_multi_tenant() {
    // Verify each service call gets the correct context independently
    async fn service_b(ctx: ServiceContext) -> Option<String> {
        ctx.tenant_id().map(|s| s.to_owned())
    }

    let ctx_a = ServiceContext::new().with_tenant_id("tenant-alpha");
    let ctx_b = ServiceContext::new().with_tenant_id("tenant-beta");

    let (result_a, result_b) = tokio::join!(service_b(ctx_a), service_b(ctx_b),);

    assert_eq!(result_a.as_deref(), Some("tenant-alpha"));
    assert_eq!(result_b.as_deref(), Some("tenant-beta"));
}
