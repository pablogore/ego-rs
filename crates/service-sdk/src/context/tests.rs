//! Tests for service context propagation.

use crate::context::ServiceContext;

#[tokio::test]
async fn test_service_context_propagation() {
    let context = ServiceContext::new()
        .with_tenant_id("tenant-123")
        .with_correlation_id("correlation-456")
        .with_trace_id("trace-789");
    
    // Test that context is properly set
    assert_eq!(context.tenant_id(), Some("tenant-123"));
    assert_eq!(context.correlation_id(), Some("correlation-456"));
    assert_eq!(context.trace_id(), Some("trace-789"));
    
    // Test that context can be set to None
    let context2 = ServiceContext::new();
    assert_eq!(context2.tenant_id(), None);
    assert_eq!(context2.correlation_id(), None);
    assert_eq!(context2.trace_id(), None);
}