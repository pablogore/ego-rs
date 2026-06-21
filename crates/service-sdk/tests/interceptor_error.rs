//! Tests for interceptor error handling using ServiceErrorTrait.

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::{ErrorCategory, ServiceError, ServiceErrorTrait};
use ego_service_sdk::interceptor::{Interceptor, InterceptorChain};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Spy interceptor that captures &dyn ServiceErrorTrait calls
// ---------------------------------------------------------------------------

struct SpyInterceptor {
    captured_code: Mutex<Option<String>>,
    captured_category: Mutex<Option<ErrorCategory>>,
}

impl SpyInterceptor {
    fn new() -> Self {
        Self {
            captured_code: Mutex::new(None),
            captured_category: Mutex::new(None),
        }
    }
}

#[async_trait]
impl Interceptor for SpyInterceptor {
    async fn on_request(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
        Ok(())
    }

    async fn on_response(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
        Ok(())
    }

    /// REQ-020 — on_error receives &dyn ServiceErrorTrait, not &ServiceError.
    async fn on_error(
        &self,
        _context: &ServiceContext,
        error: &dyn ServiceErrorTrait,
    ) -> Result<(), ServiceError> {
        *self.captured_code.lock().unwrap() = Some(error.code().to_string());
        *self.captured_category.lock().unwrap() = Some(error.category());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TASK-006 test
// ---------------------------------------------------------------------------

/// REQ-020 / TASK-006 — on_error receives &dyn ServiceErrorTrait.
/// The spy interceptor can call .code() and .category() on the trait object.
/// The caller still receives the original typed ServiceError unchanged.
#[tokio::test]
async fn on_error_receives_service_error_trait() {
    let spy = Arc::new(SpyInterceptor::new());
    let mut chain = InterceptorChain::new();
    chain.add_interceptor(spy.clone());

    let ctx = ServiceContext::new();
    let err = ServiceError::validation("bad input");

    // Simulate the chain on_error call.
    chain.on_error(&ctx, &err).await.unwrap();

    // The spy must have captured code and category via the trait object.
    let code = spy.captured_code.lock().unwrap().clone().unwrap();
    let category = spy.captured_category.lock().unwrap().clone().unwrap();

    assert_eq!(code, "VALIDATION");
    assert_eq!(category, ErrorCategory::Validation);
}

/// Existing test still compiles — basic interceptor error count.
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
            _error: &dyn ServiceErrorTrait,
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

    chain.on_error(&context, &error).await.unwrap();
    assert_eq!(
        interceptor
            .error_count
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}
