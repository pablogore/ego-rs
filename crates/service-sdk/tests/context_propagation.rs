//! Tests for ServiceContext propagation functionality.
//!
//! After CORE-010A, context propagates via explicit ownership transfer,
//! cloning, and parameter passing — never through ambient APIs.
//! This file verifies that fields carry correctly through explicit
//! passing and across spawned task boundaries.

use ego_service_sdk::context::ServiceContext;

#[tokio::test]
async fn test_service_context_explicit_propagation() {
    let ctx = ServiceContext::new()
        .with_tenant_id("tenant-123")
        .with_correlation_id("correlation-456")
        .with_trace_id("trace-789");

    // Explicit passing — no scope, no ambient read
    let ctx2 = ctx.clone();
    assert_eq!(ctx2.tenant_hint(), Some("tenant-123"));
    assert_eq!(ctx2.correlation_id(), Some("correlation-456"));
    assert_eq!(ctx2.trace_id(), Some("trace-789"));
}

#[tokio::test]
async fn test_spawned_task_receives_context_explicitly() {
    let ctx = ServiceContext::new().with_tenant_id("spawn-tenant");
    let ctx_clone = ctx.clone();

    // Explicitly capture context via ownership transfer into spawned task
    let result = tokio::spawn(async move { ctx_clone.tenant_hint().map(|s| s.to_owned()) })
        .await
        .unwrap();

    assert_eq!(result.as_deref(), Some("spawn-tenant"));
}
