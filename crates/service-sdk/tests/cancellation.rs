//! Tests for cancellation detection via timeout.

use ego_service_sdk::context::ServiceContext;
use std::time::Duration;

#[tokio::test]
async fn test_deadline_expiry_as_cancellation() {
    let deadline = std::time::SystemTime::now()
        .checked_add(Duration::from_millis(1))
        .unwrap();

    let context = ServiceContext::new().with_deadline(deadline);

    assert!(context.deadline.is_some());

    // Sleep briefly to ensure deadline expires
    tokio::time::sleep(Duration::from_millis(5)).await;

    assert!(context.is_deadline_expired());
}

#[tokio::test]
async fn test_timeout_as_cancellation() {
    let timeout = Duration::from_millis(1);
    let context = ServiceContext::new().with_timeout(timeout);

    assert!(context.timeout.is_some());
    assert_eq!(context.timeout, Some(timeout));
}
