//! Tests for interceptor functionality.

use crate::contract::{ServiceDescriptor, ContractVersion};
use crate::error::ServiceError;
use crate::context::ServiceContext;
use crate::interceptor::{Interceptor, InterceptorChain};
use async_trait::async_trait;
use std::sync::Arc;

#[tokio::test]
async fn test_interceptor_chain() {
    #[derive(Debug)]
    struct TestInterceptor {
        call_count: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl Interceptor for TestInterceptor {
        async fn on_request(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
            self.call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        async fn on_response(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
            self.call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        async fn on_error(&self, _context: &ServiceContext, _error: &ServiceError) -> Result<(), ServiceError> {
            self.call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
    }

    let interceptor1 = Arc::new(TestInterceptor {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let interceptor2 = Arc::new(TestInterceptor {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });

    let mut chain = InterceptorChain::new();
    chain.add_interceptor(interceptor1.clone());
    chain.add_interceptor(interceptor2.clone());

    let context = ServiceContext::new();
    
    // Test on_request
    chain.on_request(&context).await.unwrap();
    assert_eq!(interceptor1.call_count.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(interceptor2.call_count.load(std::sync::atomic::Ordering::Relaxed), 1);

    // Test on_response
    chain.on_response(&context).await.unwrap();
    assert_eq!(interceptor1.call_count.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert_eq!(interceptor2.call_count.load(std::sync::atomic::Ordering::Relaxed), 2);

    // Test on_error
    let error = ServiceError::validation("test error");
    chain.on_error(&context, &error).await.unwrap();
    assert_eq!(interceptor1.call_count.load(std::sync::atomic::Ordering::Relaxed), 3);
    assert_eq!(interceptor2.call_count.load(std::sync::atomic::Ordering::Relaxed), 3);
}