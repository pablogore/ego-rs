//! Tests for deadline propagation functionality.
//!
//! After CORE-010A, deadline context is accessed through owned field assertions
//! on the ServiceContext value, not through ambient scope/current APIs.

use ego_service_sdk::context::ServiceContext;
use std::time::{Duration, SystemTime};

#[tokio::test]
async fn test_deadline_propagation() {
    let deadline = SystemTime::now() + Duration::from_millis(100);
    let context = ServiceContext::new().with_deadline(deadline);

    // Test that deadline is properly set on the owned value
    assert!(context.deadline.is_some());

    // Test that deadline expiration check works on the owned value
    assert!(!context.is_deadline_expired());
}
