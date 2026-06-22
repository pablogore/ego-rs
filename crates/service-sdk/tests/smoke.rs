//! End-to-end smoke test for the Service SDK.

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::contract::{ContractVersion, OperationDescriptor, ServiceDescriptor};
use ego_service_sdk::error::{ServiceError, ServiceErrorTrait};
use ego_service_sdk::implementation::{LifecycleManaged, Service};
use ego_service_sdk::interceptor::{Interceptor, InterceptorChain};
use ego_service_sdk::registry::ServiceRegistry;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test service implementation
// ---------------------------------------------------------------------------

struct TestServiceImpl {
    descriptor: ServiceDescriptor,
}

impl TestServiceImpl {
    fn new(name: &str) -> Self {
        Self {
            descriptor: ServiceDescriptor {
                name: name.to_string(),
                version: ContractVersion::new(1, 0, 0),
                operations: vec![OperationDescriptor {
                    name: "test_op".to_string(),
                    input: vec!["String".to_string()],
                    output: "String".to_string(),
                    errors: vec![],
                    description: None,
                    metadata: std::collections::HashMap::new(),
                    idempotent: false,
                    mutating: true,
                }],
                description: None,
                metadata: std::collections::HashMap::new(),
            },
        }
    }
}

#[async_trait]
impl Service for TestServiceImpl {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }
}

// ---------------------------------------------------------------------------
// Test interceptor
// ---------------------------------------------------------------------------

struct CountingInterceptor {
    request_count: std::sync::atomic::AtomicUsize,
    response_count: std::sync::atomic::AtomicUsize,
}

impl CountingInterceptor {
    fn new() -> Self {
        Self {
            request_count: std::sync::atomic::AtomicUsize::new(0),
            response_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
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
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_service_descriptor() {
    let descriptor = ServiceDescriptor {
        name: "MyService".to_string(),
        version: ContractVersion::new(1, 0, 0),
        operations: vec![OperationDescriptor {
            name: "do_thing".to_string(),
            input: vec!["Input".to_string()],
            output: "Output".to_string(),
            errors: vec![],
            description: None,
            metadata: std::collections::HashMap::new(),
            idempotent: false,
            mutating: true,
        }],
        description: None,
        metadata: std::collections::HashMap::new(),
    };
    assert_eq!(descriptor.name, "MyService");
    assert_eq!(descriptor.version, ContractVersion::new(1, 0, 0));
    assert_eq!(descriptor.operations.len(), 1);
    assert_eq!(descriptor.operations[0].name, "do_thing");
}

#[tokio::test]
async fn test_contract_version() {
    let v = ContractVersion::new(2, 1, 0);
    assert_eq!(v.major, 2);
    assert_eq!(v.minor, 1);
    assert_eq!(v.patch, 0);
    assert_eq!(v.to_string(), "2.1.0");

    let v_parsed: ContractVersion = "2.1.0".parse().unwrap();
    assert_eq!(v_parsed, v);
}

#[tokio::test]
async fn test_service_implementation() {
    let service = TestServiceImpl::new("TestSvc");
    assert_eq!(service.name(), "TestSvc");
    assert_eq!(service.version(), &ContractVersion::new(1, 0, 0));
}

#[tokio::test]
async fn test_service_registry() {
    let registry = ServiceRegistry::new();
    assert!(registry.is_empty());

    let default_registry: ServiceRegistry = Default::default();
    assert!(default_registry.is_empty());
}

#[tokio::test]
async fn test_interceptor_chain() {
    let chain = InterceptorChain::new();
    let ctx = ServiceContext::new();
    assert!(chain.on_request(&ctx).await.is_ok());
    assert!(chain.on_response(&ctx).await.is_ok());

    let error = ServiceError::validation("test");
    assert!(chain.on_error(&ctx, &error).await.is_ok());
}

#[tokio::test]
async fn test_interceptor_with_interceptors() {
    let mut chain = InterceptorChain::new();
    let counter = Arc::new(CountingInterceptor::new());
    chain.add_interceptor(counter.clone());

    let ctx = ServiceContext::new();
    chain.on_request(&ctx).await.unwrap();
    assert_eq!(
        counter
            .request_count
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    chain.on_response(&ctx).await.unwrap();
    assert_eq!(
        counter
            .response_count
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[tokio::test]
async fn test_service_context() {
    let ctx = ServiceContext::new()
        .with_tenant_id("tenant-1")
        .with_correlation_id("corr-1")
        .with_trace_id("trace-1");

    assert_eq!(ctx.tenant_id(), Some("tenant-1"));
    assert_eq!(ctx.correlation_id(), Some("corr-1"));
    assert_eq!(ctx.trace_id(), Some("trace-1"));
}

#[tokio::test]
async fn test_context_explicit_carry() {
    let ctx = ServiceContext::new().with_tenant_id("scoped-tenant");
    let ctx2 = ctx.clone();
    assert_eq!(ctx2.tenant_id(), Some("scoped-tenant"));
    // No scope() call; no current() call.
}

#[tokio::test]
async fn test_tenant_isolation() {
    let a = ServiceContext::new().with_tenant_id("tenant-a");
    let b = ServiceContext::new().with_tenant_id("tenant-b");

    assert_eq!(a.tenant_id(), Some("tenant-a"));
    assert_eq!(b.tenant_id(), Some("tenant-b"));
    assert!(!a.is_cross_tenant_allowed());
    assert!(a.allow_cross_tenant().is_cross_tenant_allowed());
}

#[tokio::test]
async fn test_deadline() {
    let ctx = ServiceContext::new()
        .with_deadline(std::time::SystemTime::now() + std::time::Duration::from_millis(100));
    assert!(ctx.deadline.is_some());
    assert!(!ctx.is_deadline_expired());
}

/// REQ-017 / TASK-005 — A struct implementing only Service must compile without lifecycle hooks.
/// The lifecycle hooks (initialize, shutdown) live exclusively on LifecycleManaged.
#[tokio::test]
async fn service_trait_has_no_lifecycle_hooks() {
    struct NoLifecycleService {
        descriptor: ServiceDescriptor,
    }

    // Service must compile without any initialize/shutdown methods.
    #[async_trait]
    impl Service for NoLifecycleService {
        fn descriptor(&self) -> &ServiceDescriptor {
            &self.descriptor
        }
    }

    let svc = NoLifecycleService {
        descriptor: ServiceDescriptor {
            name: "NoLifecycle".to_string(),
            version: ContractVersion::new(1, 0, 0),
            operations: vec![],
            description: None,
            metadata: std::collections::HashMap::new(),
        },
    };
    assert_eq!(svc.name(), "NoLifecycle");
}

/// REQ-017 / TASK-005 — A struct implementing LifecycleManaged exposes initialize/shutdown.
#[tokio::test]
async fn lifecycle_managed_hooks_are_callable() {
    struct ManagedService;

    #[async_trait]
    impl LifecycleManaged for ManagedService {
        // Default no-op implementations are sufficient.
    }

    let svc = ManagedService;
    // Default implementations must return Ok(()).
    assert!(svc.initialize().await.is_ok());
    assert!(svc.shutdown().await.is_ok());
}
