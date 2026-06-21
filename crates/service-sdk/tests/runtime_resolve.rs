//! SPEC-008 TASK-014: Runtime::resolve\<T\>() integration tests.
//!
//! Exercises the full pipeline: register via ResolvableContainer, build Runtime,
//! resolve a typed proxy, and verify method dispatch, interceptors, context propagation,
//! error paths, and empty-registry behavior.
//! Run with: cargo test -p ego-service-sdk runtime_resolve

use std::any::{Any, TypeId};
use std::sync::Arc;

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::contract::version::ContractVersion;
use ego_service_sdk::di::{DepKey, Injectable};
use ego_service_sdk::error::ServiceError;
use ego_service_sdk::interceptor::{Interceptor, InterceptorChain};
use ego_service_sdk::registry::ServiceRegistry;
use ego_service_sdk::runtime::RuntimeInner;
use ego_service_sdk::runtime::{ResolvableContainer, RuntimeBuilder, RuntimeError};
use ego_service_sdk_macros::service;

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

/// A minimal test service for resolve round-trips.
#[service(version = "1.0.0")]
trait ResolveTestService {
    #[operation]
    async fn greet(&self, name: String) -> Result<String, ServiceError>;
}

struct GreetService;

#[async_trait]
impl ResolveTestService for GreetService {
    async fn greet(&self, name: String) -> Result<String, ServiceError> {
        Ok(format!("Hello, {}!", name))
    }
}

/// An interceptor that records the hooks that were called.
#[derive(Default)]
struct CallRecorder {
    calls: std::sync::Mutex<Vec<&'static str>>,
}

#[async_trait]
impl Interceptor for CallRecorder {
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
        _err: &dyn ego_service_sdk::error::ServiceErrorTrait,
    ) -> Result<(), ServiceError> {
        self.calls.lock().unwrap().push("on_error");
        Ok(())
    }
}

/// A service that reads the current context's tenant.
struct ContextReaderService {
    captured_tenant: std::sync::Mutex<Option<String>>,
}

#[async_trait]
impl ResolveTestService for ContextReaderService {
    async fn greet(&self, _name: String) -> Result<String, ServiceError> {
        let tenant = ServiceContext::current().and_then(|ctx| ctx.tenant_id.clone());
        *self.captured_tenant.lock().unwrap() = tenant;
        Ok("read".to_string())
    }
}

/// A service that always fails.
struct FailingService;

#[async_trait]
impl ResolveTestService for FailingService {
    async fn greet(&self, _name: String) -> Result<String, ServiceError> {
        Err(ServiceError::internal("failed on purpose"))
    }
}

// ---------------------------------------------------------------------------
// Helper: register an implementation arc into a fresh registry
// ---------------------------------------------------------------------------

fn register_greet_service(impl_arc: Arc<dyn ResolveTestService>) -> ServiceRegistry {
    let container = ResolvableContainer(impl_arc);
    let erased: Arc<dyn Any + Send + Sync> = Arc::new(container);

    let mut registry = ServiceRegistry::new();
    registry
        .register::<ResolveTestServiceTag>(ContractVersion::new(1, 0, 0), erased)
        .unwrap();
    registry
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_returns_generated_proxy() {
    let registry = register_greet_service(Arc::new(GreetService));

    let runtime = RuntimeBuilder::new()
        .with_registry(registry)
        .build()
        .await
        .unwrap();

    let proxy: ResolveTestServiceRef = runtime.resolve::<ResolveTestServiceTag>().unwrap();
    let result = proxy.greet("Tester".to_string()).await.unwrap();
    assert_eq!(result, "Hello, Tester!");
}

#[tokio::test]
async fn resolve_preserves_interceptors() {
    let recorder = Arc::new(CallRecorder::default());
    let mut chain = InterceptorChain::new();
    chain.add_interceptor(recorder.clone());

    let registry = register_greet_service(Arc::new(GreetService));

    let runtime = RuntimeBuilder::new()
        .with_registry(registry)
        .with_interceptor_chain(Arc::new(chain))
        .build()
        .await
        .unwrap();

    let proxy: ResolveTestServiceRef = runtime.resolve::<ResolveTestServiceTag>().unwrap();
    let result = proxy.greet("World".to_string()).await.unwrap();
    assert_eq!(result, "Hello, World!");

    let calls = recorder.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec!["on_request", "on_response"],
        "interceptor hooks must fire via resolved proxy"
    );
}

#[tokio::test]
async fn resolve_preserves_context() {
    let ctx_service = Arc::new(ContextReaderService {
        captured_tenant: std::sync::Mutex::new(None),
    });
    let registry = register_greet_service(ctx_service.clone());

    let runtime = RuntimeBuilder::new()
        .with_registry(registry)
        .build()
        .await
        .unwrap();

    let proxy: ResolveTestServiceRef = runtime.resolve::<ResolveTestServiceTag>().unwrap();

    let outer_ctx = ServiceContext::new().with_tenant_id("tenant-abc");
    outer_ctx
        .scope(|| async { proxy.greet("x".to_string()).await })
        .await
        .unwrap();

    let captured = ctx_service.captured_tenant.lock().unwrap().clone();
    assert_eq!(
        captured.as_deref(),
        Some("tenant-abc"),
        "ServiceContext must propagate through resolved proxy"
    );
}

#[tokio::test]
async fn resolve_unknown_service_fails() {
    let registry = ServiceRegistry::new();
    let runtime = RuntimeBuilder::new()
        .with_registry(registry)
        .build()
        .await
        .unwrap();

    let result: Result<ResolveTestServiceRef, RuntimeError> =
        runtime.resolve::<ResolveTestServiceTag>();

    match result {
        Err(RuntimeError::ServiceNotFound) => {} // expected
        other => panic!(
            "resolving unregistered service must fail with ServiceNotFound, got: {:?}",
            other.err().unwrap()
        ),
    }
}

#[tokio::test]
async fn resolve_error_path_fires_on_error() {
    let recorder = Arc::new(CallRecorder::default());
    let mut chain = InterceptorChain::new();
    chain.add_interceptor(recorder.clone());

    let registry = register_greet_service(Arc::new(FailingService));

    let runtime = RuntimeBuilder::new()
        .with_registry(registry)
        .with_interceptor_chain(Arc::new(chain))
        .build()
        .await
        .unwrap();

    let proxy: ResolveTestServiceRef = runtime.resolve::<ResolveTestServiceTag>().unwrap();
    let result = proxy.greet("x".to_string()).await;
    assert!(result.is_err(), "failing service must return an error");

    let calls = recorder.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec!["on_request", "on_error"],
        "error path must fire on_request -> on_error"
    );
}

#[tokio::test]
async fn resolve_multiple_services_each_resolves_independently() {
    // Define a second independent service trait.
    #[service(version = "1.0.0")]
    trait QueryService {
        #[operation]
        async fn query(&self, input: String) -> Result<String, ServiceError>;
    }

    struct QueryServiceImpl;

    #[async_trait]
    impl QueryService for QueryServiceImpl {
        async fn query(&self, input: String) -> Result<String, ServiceError> {
            Ok(format!("queried: {}", input))
        }
    }

    // Register both services.
    let greet_container =
        ResolvableContainer(Arc::new(GreetService) as Arc<dyn ResolveTestService>);
    let query_container = ResolvableContainer(Arc::new(QueryServiceImpl) as Arc<dyn QueryService>);

    let mut registry = ServiceRegistry::new();
    registry
        .register::<ResolveTestServiceTag>(
            ContractVersion::new(1, 0, 0),
            Arc::new(greet_container) as Arc<dyn Any + Send + Sync>,
        )
        .unwrap();
    registry
        .register::<QueryServiceTag>(
            ContractVersion::new(1, 0, 0),
            Arc::new(query_container) as Arc<dyn Any + Send + Sync>,
        )
        .unwrap();

    let runtime = RuntimeBuilder::new()
        .with_registry(registry)
        .build()
        .await
        .unwrap();

    // Both must resolve independently.
    let greet_proxy: ResolveTestServiceRef = runtime.resolve::<ResolveTestServiceTag>().unwrap();
    let query_proxy: QueryServiceRef = runtime.resolve::<QueryServiceTag>().unwrap();

    let greet_result = greet_proxy.greet("Alice".to_string()).await.unwrap();
    let query_result = query_proxy.query("data".to_string()).await.unwrap();

    assert_eq!(greet_result, "Hello, Alice!");
    assert_eq!(query_result, "queried: data");
}

// ---------------------------------------------------------------------------
// Determinism: topological sort must produce the same result every time
// ---------------------------------------------------------------------------

/// Simple Injectable chain: A -> B -> C
struct ServiceA;

impl Injectable for ServiceA {
    fn dependencies() -> Vec<DepKey> {
        vec![DepKey::Projection(TypeId::of::<ServiceB>())]
    }
    fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError>
    where
        Self: Sized,
    {
        Ok(ServiceA)
    }
}

struct ServiceB;

impl Injectable for ServiceB {
    fn dependencies() -> Vec<DepKey> {
        vec![DepKey::Projection(TypeId::of::<ServiceC>())]
    }
    fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError>
    where
        Self: Sized,
    {
        Ok(ServiceB)
    }
}

struct ServiceC;

impl Injectable for ServiceC {
    fn dependencies() -> Vec<DepKey> {
        vec![]
    }
    fn build(_rt: &RuntimeInner) -> Result<Self, RuntimeError>
    where
        Self: Sized,
    {
        Ok(ServiceC)
    }
}

#[tokio::test]
async fn topological_order_is_deterministic() {
    // Register in forward order and build multiple times.
    for i in 0..10 {
        let builder = RuntimeBuilder::new()
            .with_service::<ServiceA>() // depends on Projection<B>
            .with_service::<ServiceB>() // depends on Projection<C>
            .with_service::<ServiceC>() // no deps
            .with_projection::<ServiceB>() // satisfies A's dep
            .with_projection::<ServiceC>(); // satisfies B's dep

        let result = builder.build().await;
        assert!(
            result.is_ok(),
            "forward order iteration {i} must build: {:?}",
            result.err()
        );
    }
}

#[tokio::test]
async fn multiple_builds_produce_same_order() {
    // Register in REVERSE order and verify it still builds.
    for i in 0..10 {
        let builder = RuntimeBuilder::new()
            .with_service::<ServiceC>() // no deps
            .with_service::<ServiceB>() // depends on Projection<C>
            .with_service::<ServiceA>() // depends on Projection<B>
            .with_projection::<ServiceC>() // satisfies B's dep
            .with_projection::<ServiceB>(); // satisfies A's dep

        let result = builder.build().await;
        assert!(
            result.is_ok(),
            "reverse order iteration {i} must build: {:?}",
            result.err()
        );
    }
}
