//! Tests for TASK-009, TASK-010, TASK-011 — macro-generated proxy types.
//!
//! These are compile + runtime tests that exercise the code `#[service]` generates.
//! Run with: cargo test -p ego-service-sdk proxy_codegen

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::di::{AdapterRef, DepKey, Injectable, ProjectionRef};
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::{ServiceError, ServiceErrorTrait};
use ego_service_sdk::interceptor::{Interceptor, InterceptorChain};
use ego_service_sdk::runtime::{RuntimeBuilder, RuntimeError};
#[allow(unused_imports)]
use ego_service_sdk_macros::operation;
use ego_service_sdk_macros::service;
use std::sync::Arc;

/// Shared fixture: an empty `Runtime` for tests that only need proxy/DI wiring.
fn test_runtime() -> ego_service_sdk::runtime::Runtime {
    RuntimeBuilder::new().build()
}

// ---------------------------------------------------------------------------
// Tag + Ref generation
// ---------------------------------------------------------------------------

/// Domain-specific error with NO From<ServiceError> impl — verifies the proxy does
/// not impose that bound on callers.
#[derive(Debug)]
pub struct OrderError(pub String);

impl ServiceErrorTrait for OrderError {
    fn code(&self) -> &str {
        "ORDER_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

/// Sample trait that uses a domain error type unrelated to ServiceError.
/// This must compile even though OrderError does not implement From<ServiceError>.
#[service(version = "1.0.0")]
pub trait OrderService {
    #[operation]
    async fn place_order(
        &self,
        ctx: ServiceContext,
        product_id: String,
    ) -> Result<String, OrderError>;
}

#[test]
fn service_on_trait_generates_tag_and_ref() {
    // OrderServiceTag must be a public ZST — verify it can be named and used.
    let _tag: OrderServiceTag = OrderServiceTag;

    // OrderServiceRef must exist and have a ::new constructor.
    struct NoopOrderService;

    #[async_trait]
    impl OrderService for NoopOrderService {
        async fn place_order(
            &self,
            _ctx: ServiceContext,
            _product_id: String,
        ) -> Result<String, OrderError> {
            Ok("noop".to_string())
        }
    }

    let inner: Arc<dyn OrderService> = Arc::new(NoopOrderService);
    let chain = Arc::new(InterceptorChain::new());
    let rt = test_runtime();
    let runtime_weak = Arc::downgrade(rt.inner());

    // Must compile: OrderServiceRef::new(inner, chain, runtime_weak)
    let _ref_obj = OrderServiceRef::new(inner, chain, runtime_weak);
}

// ---------------------------------------------------------------------------
// Interceptor chain forwarding
// ---------------------------------------------------------------------------

/// SpyInterceptor records which hooks were called, in which order.
#[derive(Default)]
struct SpyInterceptor {
    calls: std::sync::Mutex<Vec<&'static str>>,
}

#[async_trait]
impl Interceptor for SpyInterceptor {
    async fn on_request(&self, _ctx: &ServiceContext) -> Result<(), ServiceError> {
        self.calls.lock().unwrap().push("on_request");
        Ok(())
    }
    async fn on_response(&self, _ctx: &ServiceContext) -> Result<(), ServiceError> {
        self.calls.lock().unwrap().push("on_response");
        Ok(())
    }
    async fn on_error(
        &self,
        _ctx: &ServiceContext,
        _err: &dyn ServiceErrorTrait,
    ) -> Result<(), ServiceError> {
        self.calls.lock().unwrap().push("on_error");
        Ok(())
    }
}

/// A service used for interceptor ordering tests.
#[service(version = "1.0.0")]
pub trait PaymentService {
    #[operation]
    async fn charge(&self, ctx: ServiceContext, amount: u64) -> Result<String, ServiceError>;

    #[operation]
    async fn refund(&self, ctx: ServiceContext, amount: u64) -> Result<String, ServiceError>;
}

struct FailingPaymentService;

#[async_trait]
impl PaymentService for FailingPaymentService {
    async fn charge(&self, _ctx: ServiceContext, _amount: u64) -> Result<String, ServiceError> {
        Err(ServiceError::internal("payment failed"))
    }
    async fn refund(&self, _ctx: ServiceContext, _amount: u64) -> Result<String, ServiceError> {
        Ok("refunded".to_string())
    }
}

#[tokio::test]
async fn interceptors_fire_in_order_via_generated_ref() {
    let spy = Arc::new(SpyInterceptor::default());
    let mut chain = InterceptorChain::new();
    chain.add_interceptor(spy.clone());

    let inner: Arc<dyn PaymentService> = Arc::new(FailingPaymentService);
    let rt = test_runtime();
    let runtime_weak = Arc::downgrade(rt.inner());
    let proxy = PaymentServiceRef::new(inner, Arc::new(chain), runtime_weak);

    let ctx = ServiceContext::new();

    // charge() returns Err — should fire on_request then on_error.
    let result = proxy.charge(ctx, 100).await;
    assert!(result.is_err(), "charge must return Err");

    let calls = spy.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec!["on_request", "on_error"],
        "interceptor hooks must fire in order: on_request -> on_error for errors"
    );
}

#[tokio::test]
async fn interceptors_fire_on_success_via_generated_ref() {
    let spy = Arc::new(SpyInterceptor::default());
    let mut chain = InterceptorChain::new();
    chain.add_interceptor(spy.clone());

    let inner: Arc<dyn PaymentService> = Arc::new(FailingPaymentService);
    let rt = test_runtime();
    let runtime_weak = Arc::downgrade(rt.inner());
    let proxy = PaymentServiceRef::new(inner, Arc::new(chain), runtime_weak);

    let ctx = ServiceContext::new();

    // refund() returns Ok — should fire on_request then on_response.
    let result = proxy.refund(ctx, 50).await;
    assert!(result.is_ok(), "refund must return Ok");

    let calls = spy.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec!["on_request", "on_response"],
        "interceptor hooks must fire in order: on_request -> on_response for success"
    );
}

#[tokio::test]
async fn context_propagates_via_explicit_param() {
    struct ContextCapturingService {
        captured_tenant: std::sync::Mutex<Option<String>>,
    }

    #[async_trait]
    impl PaymentService for ContextCapturingService {
        async fn charge(&self, ctx: ServiceContext, _amount: u64) -> Result<String, ServiceError> {
            *self.captured_tenant.lock().unwrap() = ctx.tenant_id.clone();
            Ok("charged".to_string())
        }
        async fn refund(&self, _ctx: ServiceContext, _amount: u64) -> Result<String, ServiceError> {
            Ok("refunded".to_string())
        }
    }

    let capturing = Arc::new(ContextCapturingService {
        captured_tenant: std::sync::Mutex::new(None),
    });
    let inner: Arc<dyn PaymentService> = capturing.clone();
    let chain = Arc::new(InterceptorChain::new());
    let rt = test_runtime();
    let runtime_weak = Arc::downgrade(rt.inner());
    let proxy = PaymentServiceRef::new(inner, chain, runtime_weak);

    let ctx = ServiceContext::new().with_tenant_id("tenant-abc");
    proxy.charge(ctx, 42).await.unwrap();

    let captured = capturing.captured_tenant.lock().unwrap().clone();
    assert_eq!(
        captured.as_deref(),
        Some("tenant-abc"),
        "Context explicitly passed to proxy must arrive at impl through parameter"
    );
}

// ---------------------------------------------------------------------------
// Injectable field detection
// ---------------------------------------------------------------------------

/// Dummy types used as DI targets.
struct MyProjection;
struct MyAdapter;

/// A service struct with mixed field types:
/// - one `ProjectionRef<P>` → must appear in `dependencies()`
/// - one `AdapterRef<A>`    → must appear in `dependencies()`
/// - one plain `String`     → must NOT appear in `dependencies()`
///
/// NOTE: `EntityRef<T>` is NOT detected here — it lives in entity-sdk (INV-008).
#[allow(dead_code)]
#[service]
struct InjectableServiceImpl {
    projection: ProjectionRef<MyProjection>,
    adapter: AdapterRef<MyAdapter>,
    name: String,
}

#[test]
fn service_on_struct_detects_fields() {
    let deps = InjectableServiceImpl::dependencies();
    assert_eq!(
        deps.len(),
        2,
        "dependencies() must return 2 items (ProjectionRef + AdapterRef), not 3"
    );

    let has_projection = deps.iter().any(|d| matches!(d, DepKey::Projection(_, _)));
    let has_adapter = deps.iter().any(|d| matches!(d, DepKey::Adapter(_, _)));

    assert!(
        has_projection,
        "dependencies() must include a Projection DepKey"
    );
    assert!(has_adapter, "dependencies() must include an Adapter DepKey");
}

#[test]
fn injectable_build_returns_dependency_not_found_for_di_fields() {
    // InjectableServiceImpl has DI fields (ProjectionRef, AdapterRef).
    // build() calls rt.resolve_projection / rt.resolve_adapter which return
    // DependencyNotFound when no instance is registered.
    let rt = test_runtime();
    let result = InjectableServiceImpl::build(rt.inner());
    assert!(
        matches!(result, Err(RuntimeError::DependencyNotFound { .. })),
        "build() must return DependencyNotFound when resolvers are missing"
    );
}

/// A struct with only plain fields — build() should succeed via Default.
#[allow(dead_code)]
#[service]
struct PlainServiceImpl {
    name: String,
    count: u32,
}

#[test]
fn injectable_build_succeeds_for_plain_fields() {
    let rt = test_runtime();
    let result = PlainServiceImpl::build(rt.inner());
    assert!(
        result.is_ok(),
        "build() must succeed when all fields use Default"
    );
    let instance = result.unwrap();
    assert_eq!(instance.name, String::default());
    assert_eq!(instance.count, u32::default());
}
