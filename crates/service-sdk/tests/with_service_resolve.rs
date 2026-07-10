//! Tests for CORE-025 TASK-013 (`RuntimeBuilder::with_service`) and
//! TASK-014 (`Runtime::resolve`).
//!
//! Compile + runtime tests exercising the real `#[service]`/`#[operation]`/
//! `#[tenant_scoped]` macros — no real DB/broker/HTTP I/O, only in-memory
//! runtime state — mirroring `proxy_codegen.rs`'s and
//! `tenant_scoped_codegen.rs`'s style and placement (a crate-local `tests/`
//! compile-test file, same as those two; this repo has no
//! `crates/integration-tests/` and none of these tests need one).
//!
//! Run with: cargo test -p ego-service-sdk --test with_service_resolve

use std::sync::Arc;

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::{ServiceError, ServiceErrorTrait};
use ego_service_sdk::registry::RegistryError;
use ego_service_sdk::runtime::{RuntimeBuilder, RuntimeError};
use ego_service_sdk::security::SecurityError;
#[allow(unused_imports)]
use ego_service_sdk_macros::{operation, tenant_scoped};
use ego_service_sdk_macros::service;

// ---------------------------------------------------------------------------
// TASK-013: RuntimeBuilder::with_service registration
// ---------------------------------------------------------------------------

#[service(version = "1.0.0")]
pub trait HelloService {
    #[operation]
    async fn greet(&self, ctx: ServiceContext, name: String) -> Result<String, ServiceError>;
}

struct HelloServiceImpl;

#[async_trait]
impl HelloService for HelloServiceImpl {
    async fn greet(&self, _ctx: ServiceContext, name: String) -> Result<String, ServiceError> {
        Ok(format!("hello, {name}"))
    }
}

struct OtherHelloServiceImpl;

#[async_trait]
impl HelloService for OtherHelloServiceImpl {
    async fn greet(&self, _ctx: ServiceContext, _name: String) -> Result<String, ServiceError> {
        Ok("other".to_string())
    }
}

#[test]
fn first_registration_for_a_tag_succeeds() {
    let inner: Arc<dyn HelloService> = Arc::new(HelloServiceImpl);
    let result = RuntimeBuilder::new().with_service::<HelloServiceTag>(inner);
    assert!(result.is_ok(), "first registration for a fresh tag must succeed");
}

#[test]
fn duplicate_registration_is_rejected_not_silently_replaced() {
    let first: Arc<dyn HelloService> = Arc::new(HelloServiceImpl);
    let second: Arc<dyn HelloService> = Arc::new(OtherHelloServiceImpl);

    // The registry's own `register` (verified at that layer in
    // `registry.rs::register_rejects_duplicate`) early-returns before
    // pushing, so a rejected duplicate never overwrites the original entry.
    let result = RuntimeBuilder::new()
        .with_service::<HelloServiceTag>(first)
        .expect("first registration must succeed")
        .with_service::<HelloServiceTag>(second);

    assert!(
        matches!(result, Err(RegistryError::DuplicateService { .. })),
        "second registration under the same tag must be rejected, not silently replace the original"
    );
}

// ---------------------------------------------------------------------------
// TASK-014: Runtime::resolve
// ---------------------------------------------------------------------------

#[tokio::test]
async fn registered_tag_resolves_to_a_fully_guarded_invokable_proxy() {
    let inner: Arc<dyn HelloService> = Arc::new(HelloServiceImpl);
    let rt = RuntimeBuilder::new()
        .with_service::<HelloServiceTag>(inner)
        .expect("registration succeeds")
        .build();

    let proxy = rt
        .resolve::<HelloServiceTag>()
        .expect("registered tag must resolve to Ok(HelloServiceRef)");

    let out = proxy
        .greet(ServiceContext::new(), "world".to_string())
        .await
        .expect("invocation succeeds exactly as the hand-rolled HelloServiceRef::new path would");
    assert_eq!(out, "hello, world");
}

#[test]
fn unregistered_tag_resolves_to_service_not_found_not_a_panic() {
    let rt = RuntimeBuilder::new().build();
    let result = rt.resolve::<HelloServiceTag>();
    assert!(matches!(result, Err(RuntimeError::ServiceNotFound)));
}

/// Domain error with `From<SecurityError>` — required for `#[tenant_scoped]`
/// codegen's fallible `enforce_tenant(..)?` call site (mirrors
/// `tenant_scoped_codegen.rs::TenantTestError`).
#[derive(Debug)]
pub struct TenantHelloError(String);

impl From<SecurityError> for TenantHelloError {
    fn from(e: SecurityError) -> Self {
        TenantHelloError(e.to_string())
    }
}

impl ServiceErrorTrait for TenantHelloError {
    fn code(&self) -> &str {
        "TENANT_HELLO_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[service(version = "1.0.0")]
pub trait TenantScopedHello {
    #[operation]
    #[tenant_scoped]
    async fn greet(&self, ctx: ServiceContext) -> Result<String, TenantHelloError>;
}

struct TenantScopedHelloImpl;

#[async_trait]
impl TenantScopedHello for TenantScopedHelloImpl {
    async fn greet(&self, _ctx: ServiceContext) -> Result<String, TenantHelloError> {
        Ok("hello".to_string())
    }
}

#[tokio::test]
async fn tenant_scoped_operation_resolved_via_resolve_still_fails_closed() {
    let inner: Arc<dyn TenantScopedHello> = Arc::new(TenantScopedHelloImpl);
    let rt = RuntimeBuilder::new()
        .with_service::<TenantScopedHelloTag>(inner)
        .expect("registration succeeds")
        .build();

    let proxy = rt
        .resolve::<TenantScopedHelloTag>()
        .expect("registered tenant-scoped tag must resolve");

    // No security attached -> unresolvable tenant under the default
    // AuthenticatedOnly enforcement mode -> fails closed, same guard order
    // the hand-rolled path (tenant_scoped_codegen.rs) enforces. `resolve`
    // introduces no alternate, relaxed code path.
    let result = proxy.greet(ServiceContext::new()).await;

    assert!(
        result.is_err(),
        "tenant-scoped op resolved via `resolve` must fail closed without a resolvable tenant"
    );
}
