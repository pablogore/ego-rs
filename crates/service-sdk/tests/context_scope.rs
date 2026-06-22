//! Tests for ServiceContext scope functionality.
//!
//! After CORE-010A, ServiceContext has no ambient APIs (scope, current).
//! Context values are propagated via explicit ownership, cloning, and
//! parameter passing. These tests verify that fields are preserved
//! through explicit operations rather than ambient side effects.

use ego_service_sdk::context::ServiceContext;
use std::collections::HashMap;

#[tokio::test]
async fn test_service_context_explicit_field_carry() {
    let ctx = ServiceContext::new()
        .with_tenant_id("tenant1")
        .with_correlation_id("correlation1")
        .with_trace_id("trace1");

    // Clone and assert fields on the owned value directly
    let ctx2 = ctx.clone();
    assert_eq!(ctx2.tenant_id(), Some("tenant1"));
    assert_eq!(ctx2.correlation_id(), Some("correlation1"));
    assert_eq!(ctx2.trace_id(), Some("trace1"));
}

#[tokio::test]
async fn test_service_context_explicit_tenant_carry() {
    let ctx = ServiceContext::new().with_tenant_id("test-tenant");

    // Clone preserves the tenant field
    let ctx2 = ctx.clone();
    assert_eq!(ctx2.tenant_id(), Some("test-tenant"));

    // The original is still intact
    assert_eq!(ctx.tenant_id(), Some("test-tenant"));
}

#[tokio::test]
async fn test_service_context_explicit_additional_context() {
    let mut additional = HashMap::new();
    additional.insert("key1".to_string(), "value1".to_string());
    additional.insert("key2".to_string(), "value2".to_string());

    let ctx = ServiceContext::new().with_additional_context(additional);

    // Clone and assert fields on owned value
    let ctx2 = ctx.clone();
    assert_eq!(ctx2.additional_context.len(), 2);
    assert_eq!(
        ctx2.additional_context.get("key1"),
        Some(&"value1".to_string())
    );
    assert_eq!(
        ctx2.additional_context.get("key2"),
        Some(&"value2".to_string())
    );
}

#[tokio::test]
async fn test_service_context_explicit_independence() {
    // Two separate owned values are independent — no ambient side effects
    let ctx_a = ServiceContext::new().with_tenant_id("tenant_a");
    let ctx_b = ServiceContext::new().with_tenant_id("tenant_b");

    assert_eq!(ctx_a.tenant_id(), Some("tenant_a"));
    assert_eq!(ctx_b.tenant_id(), Some("tenant_b"));

    // Mutating ctx_a does NOT affect ctx_b
    let ctx_a_clone = ctx_a.clone();
    assert_eq!(ctx_a_clone.tenant_id(), Some("tenant_a"));
    assert_eq!(ctx_b.tenant_id(), Some("tenant_b"));
}
