//! Integration tests for tenant enforcement via the RuntimeBuilder pipeline.
//!
//! Direct unit tests for `RuntimeInner::enforce_tenant` live in
//! `runtime_builder.rs`'s `#[cfg(test)] mod tests`.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use ego_service_sdk::contract::version::ContractVersion;
use ego_service_sdk::error::ServiceError;
use ego_service_sdk::registry::ServiceRegistry;
use ego_service_sdk::runtime::{ResolvableContainer, RuntimeBuilder};
use ego_service_sdk_macros::service;

// ---------------------------------------------------------------------------
// Shared service for tenant tests
// ---------------------------------------------------------------------------

#[service(version = "1.0.0")]
trait TenantTestService {
    #[operation]
    async fn ping(&self) -> Result<String, ServiceError>;
}

struct PingService;

#[async_trait]
impl TenantTestService for PingService {
    async fn ping(&self) -> Result<String, ServiceError> {
        Ok("pong".to_string())
    }
}

fn register_service(registry: &mut ServiceRegistry) {
    let container = ResolvableContainer(Arc::new(PingService) as Arc<dyn TenantTestService>);
    let erased: Arc<dyn Any + Send + Sync> = Arc::new(container);
    registry
        .register::<TenantTestServiceTag>(ContractVersion::new(1, 0, 0), erased)
        .unwrap();
}

// ---------------------------------------------------------------------------
// Integration tests: RuntimeBuilder with tenant config
// ---------------------------------------------------------------------------

#[tokio::test]
async fn runtime_builder_with_tenant_id() {
    let mut registry = ServiceRegistry::new();
    register_service(&mut registry);

    let runtime = RuntimeBuilder::new()
        .with_registry(registry)
        .with_tenant_id("my-tenant")
        .build()
        .await
        .unwrap();

    let proxy: TenantTestServiceRef = runtime.resolve::<TenantTestServiceTag>().unwrap();
    let result = proxy.ping().await.unwrap();
    assert_eq!(result, "pong");
}

#[tokio::test]
async fn runtime_builder_allow_cross_tenant() {
    let mut registry = ServiceRegistry::new();
    register_service(&mut registry);

    let runtime = RuntimeBuilder::new()
        .with_registry(registry)
        .allow_cross_tenant()
        .build()
        .await
        .unwrap();

    let proxy: TenantTestServiceRef = runtime.resolve::<TenantTestServiceTag>().unwrap();
    let result = proxy.ping().await.unwrap();
    assert_eq!(result, "pong");
}

#[tokio::test]
async fn runtime_enforces_tenant_context() {
    // Runtime has tenant-a; when no context tenant is set, enforcement is skipped.
    let mut registry = ServiceRegistry::new();
    register_service(&mut registry);

    let runtime = RuntimeBuilder::new()
        .with_registry(registry)
        .with_tenant_id("tenant-a")
        .build()
        .await
        .unwrap();

    // Call without any context tenant should succeed (no tenant on ctx → skip).
    let proxy: TenantTestServiceRef = runtime.resolve::<TenantTestServiceTag>().unwrap();
    let result = proxy.ping().await.unwrap();
    assert_eq!(result, "pong");
}
