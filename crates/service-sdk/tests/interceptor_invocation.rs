//! Tests for interceptor invocation functionality.

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::{ServiceError, ServiceErrorTrait};
use ego_service_sdk::interceptor::{Interceptor, InterceptorChain};
use std::sync::Arc;

#[tokio::test]
async fn test_interceptor_invocation() {
    #[derive(Debug)]
    struct CountingInterceptor {
        request_count: std::sync::atomic::AtomicUsize,
        response_count: std::sync::atomic::AtomicUsize,
        error_count: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl Interceptor for CountingInterceptor {
        async fn on_request(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
            self.request_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        async fn on_response(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
            self.response_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        async fn on_error(
            &self,
            _context: &ServiceContext,
            _error: &dyn ServiceErrorTrait,
        ) -> Result<(), ServiceError> {
            self.error_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
    }

    let interceptor = Arc::new(CountingInterceptor {
        request_count: std::sync::atomic::AtomicUsize::new(0),
        response_count: std::sync::atomic::AtomicUsize::new(0),
        error_count: std::sync::atomic::AtomicUsize::new(0),
    });

    let mut chain = InterceptorChain::new();
    chain.add_interceptor(interceptor.clone());

    let context = ServiceContext::new();

    // Test on_request
    chain.on_request(&context).await.unwrap();
    assert_eq!(
        interceptor
            .request_count
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    // Test on_response
    chain.on_response(&context).await.unwrap();
    assert_eq!(
        interceptor
            .response_count
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    // Test on_error
    let error = ServiceError::validation("test error");
    chain.on_error(&context, &error).await.unwrap();
    assert_eq!(
        interceptor
            .error_count
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}
