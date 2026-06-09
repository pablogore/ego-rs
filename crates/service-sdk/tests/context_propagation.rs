//! Tests for ServiceContext propagation functionality.

use ego_service_sdk::context::ServiceContext;

#[tokio::test]
async fn test_service_context_propagation() {
    let context = ServiceContext::new()
        .with_tenant_id("tenant-123")
        .with_correlation_id("correlation-456")
        .with_trace_id("trace-789");

    // Test that context can be created with values
    assert_eq!(context.tenant_id(), Some("tenant-123"));
    assert_eq!(context.correlation_id(), Some("correlation-456"));
    assert_eq!(context.trace_id(), Some("trace-789"));

    // Test that context can be accessed from scope
    let captured_context = context.scope(|| async { ServiceContext::current() }).await;

    assert!(captured_context.is_some());
    let captured = captured_context.unwrap();
    assert_eq!(captured.tenant_id(), Some("tenant-123"));
    assert_eq!(captured.correlation_id(), Some("correlation-456"));
    assert_eq!(captured.trace_id(), Some("trace-789"));
}
