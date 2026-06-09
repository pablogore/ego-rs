//! Tests for ServiceContext scope functionality.

use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::tenant::TenantId;
use std::collections::HashMap;

#[tokio::test]
async fn test_service_context_scope() {
    // Create a context with some values
    let context = ServiceContext::new()
        .with_tenant_id("tenant1")
        .with_correlation_id("correlation1")
        .with_trace_id("trace1");

    // Test that the context is properly initialized
    assert_eq!(context.tenant_id(), Some("tenant1"));
    assert_eq!(context.correlation_id(), Some("correlation1"));
    assert_eq!(context.trace_id(), Some("trace1"));

    // Test scope functionality by using the scope method
    let result = context.scope(|| async {
        // Inside the scope, we should be able to access the context
        let current_context = ServiceContext::current();
        assert!(current_context.is_some());
        let current = current_context.unwrap();
        assert_eq!(current.tenant_id(), Some("tenant1"));
        assert_eq!(current.correlation_id(), Some("correlation1"));
        assert_eq!(current.trace_id(), Some("trace1"));
        "test_result"
    });

    // The result should be accessible outside the scope
    let final_result = result.await;
    assert_eq!(final_result, "test_result");
}

#[tokio::test]
async fn test_service_context_scope_with_tenant() {
    let tenant_id = TenantId {
        id: "test-tenant".to_string(),
    };
    let context = ServiceContext::new().with_tenant_id(tenant_id.id.clone());

    let result = context.scope(|| async {
        let current_context = ServiceContext::current();
        assert!(current_context.is_some());
        let current = current_context.unwrap();
        assert_eq!(current.tenant_id(), Some("test-tenant"));
        "tenant_test_result"
    });

    let final_result = result.await;
    assert_eq!(final_result, "tenant_test_result");
}

#[tokio::test]
async fn test_service_context_scope_with_additional_context() {
    let mut additional_context = HashMap::new();
    additional_context.insert("key1".to_string(), "value1".to_string());
    additional_context.insert("key2".to_string(), "value2".to_string());

    let context = ServiceContext::new().with_additional_context(additional_context);

    let result = context.scope(|| async {
        let current_context = ServiceContext::current();
        assert!(current_context.is_some());
        let current = current_context.unwrap();
        assert_eq!(current.additional_context.len(), 2);
        assert_eq!(
            current.additional_context.get("key1"),
            Some(&"value1".to_string())
        );
        assert_eq!(
            current.additional_context.get("key2"),
            Some(&"value2".to_string())
        );
        "additional_context_result"
    });

    let final_result = result.await;
    assert_eq!(final_result, "additional_context_result");
}

#[tokio::test]
async fn test_service_context_scope_restores_context() {
    // Set up an initial context
    let initial_context = ServiceContext::new().with_tenant_id("initial_tenant");

    // Set the initial context
    let _scope = initial_context.scope(|| async {
        // Create a new context inside the scope
        let new_context = ServiceContext::new().with_tenant_id("new_tenant");

        let _new_scope = new_context
            .scope(|| async {
                // Inside the nested scope, we should see the new context
                let current_context = ServiceContext::current();
                assert!(current_context.is_some());
                let current = current_context.unwrap();
                assert_eq!(current.tenant_id(), Some("new_tenant"));
                "nested_result"
            })
            .await;

        // After the nested scope, we should be back to the initial context
        let current_context = ServiceContext::current();
        assert!(current_context.is_some());
        let current = current_context.unwrap();
        assert_eq!(current.tenant_id(), Some("initial_tenant"));
        "outer_result"
    });

    // After all scopes, we should be back to no context
    let current_context = ServiceContext::current();
    assert!(current_context.is_none());
}
