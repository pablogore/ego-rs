//! Tests for deadline propagation functionality.

use ego_service_sdk::context::ServiceContext;
use std::time::{Duration, SystemTime};

#[tokio::test]
async fn test_deadline_propagation() {
    let deadline = SystemTime::now() + Duration::from_millis(100);
    let context = ServiceContext::new().with_deadline(deadline);

    // Test that deadline is properly set
    assert!(context.deadline.is_some());

    // Test that deadline expiration check works
    assert!(!context.is_deadline_expired());

    // Test that context can be accessed from scope
    let captured_context = context.scope(|| async { ServiceContext::current() }).await;

    assert!(captured_context.is_some());
    let captured = captured_context.unwrap();
    assert!(captured.deadline.is_some());
    assert!(!captured.is_deadline_expired());
}
