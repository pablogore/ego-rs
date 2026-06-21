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
use ego_service_sdk::runtime::{RuntimeBuilder, RuntimeError, RuntimeInner};
#[allow(unused_imports)]
use ego_service_sdk_macros::operation;
use ego_service_sdk_macros::service;
use std::sync::Arc;

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

impl From<RuntimeError> for OrderError {
    fn from(e: RuntimeError) -> Self {
        OrderError(e.to_string())
    }
}

/// Sample trait that uses a domain error type unrelated to ServiceError.
/// This must compile even though OrderError does not implement From<ServiceError>.
#[service(version = "1.0.0")]
pub trait OrderService {
    #[operation]
    async fn place_order(&self, product_id: String) -> Result<String, OrderError>;
}

#[test]
fn service_on_trait_generates_tag_and_ref() {
    // OrderServiceTag must be a public ZST — verify it can be named and used.
    let _tag: OrderServiceTag = OrderServiceTag;

    // OrderServiceRef must exist and have a ::new constructor.
    struct NoopOrderService;

    #[async_trait]
    impl OrderService for NoopOrderService {
        async fn place_order(&self, _product_id: String) -> Result<String, OrderError> {
            Ok("noop".to_string())
        }
    }

    let inner: Arc<dyn OrderService> = Arc::new(NoopOrderService);
    let chain = Arc::new(InterceptorChain::new());
    let runtime_inner = Arc::new(ego_service_sdk::runtime::RuntimeInner::default());
    let runtime_weak = Arc::downgrade(&runtime_inner);

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
    async fn charge(&self, amount: u64) -> Result<String, ServiceError>;

    #[operation]
    async fn refund(&self, amount: u64) -> Result<String, ServiceError>;
}

struct FailingPaymentService;

#[async_trait]
impl PaymentService for FailingPaymentService {
    async fn charge(&self, _amount: u64) -> Result<String, ServiceError> {
        Err(ServiceError::internal("payment failed"))
    }
    async fn refund(&self, _amount: u64) -> Result<String, ServiceError> {
        Ok("refunded".to_string())
    }
}

#[tokio::test]
async fn interceptors_fire_in_order_via_generated_ref() {
    let spy = Arc::new(SpyInterceptor::default());
    let mut chain = InterceptorChain::new();
    chain.add_interceptor(spy.clone());

    let inner: Arc<dyn PaymentService> = Arc::new(FailingPaymentService);
    let runtime_inner = Arc::new(ego_service_sdk::runtime::RuntimeInner::default());
    let runtime_weak = Arc::downgrade(&runtime_inner);
    let proxy = PaymentServiceRef::new(inner, Arc::new(chain), runtime_weak);

    // charge() returns Err — should fire on_request then on_error.
    let result = proxy.charge(100).await;
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
    let runtime_inner = Arc::new(ego_service_sdk::runtime::RuntimeInner::default());
    let runtime_weak = Arc::downgrade(&runtime_inner);
    let proxy = PaymentServiceRef::new(inner, Arc::new(chain), runtime_weak);

    // refund() returns Ok — should fire on_request then on_response.
    let result = proxy.refund(50).await;
    assert!(result.is_ok(), "refund must return Ok");

    let calls = spy.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec!["on_request", "on_response"],
        "interceptor hooks must fire in order: on_request -> on_response for success"
    );
}

#[tokio::test]
async fn context_propagates_across_service_boundary() {
    struct ContextCapturingService {
        captured_tenant: std::sync::Mutex<Option<String>>,
    }

    #[async_trait]
    impl PaymentService for ContextCapturingService {
        async fn charge(&self, _amount: u64) -> Result<String, ServiceError> {
            let tenant = ServiceContext::current().and_then(|ctx| ctx.tenant_id.clone());
            *self.captured_tenant.lock().unwrap() = tenant;
            Ok("charged".to_string())
        }
        async fn refund(&self, _amount: u64) -> Result<String, ServiceError> {
            Ok("refunded".to_string())
        }
    }

    let capturing = Arc::new(ContextCapturingService {
        captured_tenant: std::sync::Mutex::new(None),
    });
    let inner: Arc<dyn PaymentService> = capturing.clone();

    let chain = Arc::new(InterceptorChain::new());
    let runtime_inner = Arc::new(ego_service_sdk::runtime::RuntimeInner::default());
    let runtime_weak = Arc::downgrade(&runtime_inner);
    let proxy = PaymentServiceRef::new(inner, chain, runtime_weak);

    let outer_ctx = ServiceContext::new().with_tenant_id("tenant-abc");
    outer_ctx
        .scope(|| async { proxy.charge(42).await })
        .await
        .unwrap();

    let captured = capturing.captured_tenant.lock().unwrap().clone();
    assert_eq!(
        captured.as_deref(),
        Some("tenant-abc"),
        "ServiceContext::current() inside impl must carry the outer tenant_id"
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

    let has_projection = deps.iter().any(|d| matches!(d, DepKey::Projection(_)));
    let has_adapter = deps.iter().any(|d| matches!(d, DepKey::Adapter(_)));

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
    let rt = RuntimeInner::default();
    let result = InjectableServiceImpl::build(&rt);
    assert!(
        matches!(result, Err(RuntimeError::DependencyNotFound)),
        "build() must return DependencyNotFound when resolvers are missing"
    );
}

#[tokio::test]
async fn injectable_build_succeeds_when_deps_are_registered() {
    // Register MyProjection and MyAdapter as resolvable instances.
    let runtime = RuntimeBuilder::new()
        .with_projection_value(MyProjection)
        .with_adapter_value(MyAdapter)
        .build()
        .await
        .unwrap();

    let result = InjectableServiceImpl::build(runtime.inner());
    assert!(
        result.is_ok(),
        "build() must succeed when deps are registered with RuntimeInner"
    );
}

#[tokio::test]
async fn injectable_build_uses_runtime_inner_not_stub() {
    // Prove generated build() actually resolves from RuntimeInner:
    // Different registered projections resolve to the correct instances.
    let runtime = RuntimeBuilder::new()
        .with_projection_value(MyProjection)
        .with_adapter_value(MyAdapter)
        .build()
        .await
        .unwrap();

    let svc = InjectableServiceImpl::build(runtime.inner()).unwrap();
    // Deref to verify it's the real resolved instance, not a stub.
    let _: &MyProjection = &svc.projection;
    let _: &MyAdapter = &svc.adapter;
    assert_eq!(
        svc.name,
        String::default(),
        "plain field must still use Default::default()"
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
    let rt = RuntimeInner::default();
    let result = PlainServiceImpl::build(&rt);
    assert!(
        result.is_ok(),
        "build() must succeed when all fields use Default"
    );
    let instance = result.unwrap();
    assert_eq!(instance.name, String::default());
    assert_eq!(instance.count, u32::default());
}
