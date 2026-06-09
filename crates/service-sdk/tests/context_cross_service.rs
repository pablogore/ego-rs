//! Tests for cross-service context propagation functionality.

use ego_service_sdk::context::ServiceContext;

#[tokio::test]
async fn test_context_cross_service() {
    let context = ServiceContext::new()
        .with_tenant_id("tenant-123")
        .with_correlation_id("correlation-456");

    // Test that context can be propagated across service calls
    let captured_context = context.scope(|| async { ServiceContext::current() }).await;

    assert!(captured_context.is_some());
    let captured = captured_context.unwrap();
    assert_eq!(captured.tenant_id(), Some("tenant-123"));
    assert_eq!(captured.correlation_id(), Some("correlation-456"));
}
