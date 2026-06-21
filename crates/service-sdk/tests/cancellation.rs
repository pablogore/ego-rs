//! Tests for cancellation detection via CancellationToken and deadline.

use ego_service_sdk::context::ServiceContext;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn test_deadline_expiry_as_cancellation() {
    let deadline = std::time::SystemTime::now()
        .checked_add(Duration::from_millis(1))
        .unwrap();

    let context = ServiceContext::new().with_deadline(deadline);
    assert!(context.deadline.is_some());

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

/// REQ-019 / TASK-004 — push-style CancellationToken observed via is_cancelled().
#[tokio::test]
async fn push_style_cancellation_token_observed() {
    let token = CancellationToken::new();
    let ctx = ServiceContext::new().with_cancellation_token(token.clone());

    // Not cancelled yet.
    assert!(!ctx.is_cancelled());

    // Cancel the token.
    token.cancel();

    // Now the context reflects cancellation.
    assert!(ctx.is_cancelled());
}
