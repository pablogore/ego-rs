//! Tests for interceptor error handling functionality.

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::ServiceError;
use ego_service_sdk::interceptor::{Interceptor, InterceptorChain};
use std::sync::Arc;

#[tokio::test]
async fn test_interceptor_error() {
    #[derive(Debug)]
    struct ErrorInterceptor {
        error_count: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl Interceptor for ErrorInterceptor {
        async fn on_request(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
            Ok(())
        }

        async fn on_response(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
            Ok(())
        }

        async fn on_error(
            &self,
            _context: &ServiceContext,
            _error: &ServiceError,
        ) -> Result<(), ServiceError> {
            self.error_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
    }

    let interceptor = Arc::new(ErrorInterceptor {
        error_count: std::sync::atomic::AtomicUsize::new(0),
    });

    let mut chain = InterceptorChain::new();
    chain.add_interceptor(interceptor.clone());

    let context = ServiceContext::new();
    let error = ServiceError::validation("test error");

    // Test on_error is called when an error occurs
    chain.on_error(&context, &error).await.unwrap();
    assert_eq!(
        interceptor
            .error_count
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}
