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

#[tokio::test]
async fn duplicate_registration_is_rejected_and_the_original_remains_resolvable() {
    let first: Arc<dyn HelloService> = Arc::new(HelloServiceImpl);
    let second: Arc<dyn HelloService> = Arc::new(OtherHelloServiceImpl);

    let after_first = RuntimeBuilder::new()
        .with_service::<HelloServiceTag>(first)
        .expect("first registration must succeed");

    // `with_service` consumes `self` and drops it on `Err` — clone before
    // the risky call so the pre-duplicate-attempt state survives to prove
    // the spec's second guarantee below, not just the first.
    let snapshot_before_duplicate = after_first.clone();

    let result = after_first.with_service::<HelloServiceTag>(second);
    assert!(
        matches!(result, Err(RegistryError::DuplicateService { .. })),
        "second registration under the same tag must be rejected, not silently replace the original"
    );

    // The registry's own `register` (verified at that layer in
    // `registry.rs::register_rejects_duplicate`) early-returns before
    // pushing, so the rejected duplicate never touched `snapshot_before_duplicate`'s
    // registry — build it and prove the ORIGINAL registration still resolves
    // and invokes correctly, not just that the registry was left unmutated in theory.
    let rt = snapshot_before_duplicate.build();
    let proxy = rt
        .resolve::<HelloServiceTag>()
        .expect("the originally registered instance must remain resolvable after a rejected duplicate");
    let out = proxy
        .greet(ServiceContext::new(), "world".to_string())
        .await
        .expect("the original instance must still be invokable, not the rejected duplicate");
    assert_eq!(out, "hello, world", "must be HelloServiceImpl's output, not OtherHelloServiceImpl's \"other\"");
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
    assert!(matches!(result, Err(RuntimeError::ServiceNotFound { .. })));
}

/// Domain error with `From<SecurityError>` — required for `#[tenant_scoped]`
/// codegen's fallible `enforce_tenant(..)?` call site. Preserves the
/// originating `SecurityError` variant (not just its `Display` text) so
/// tests can assert on the actual cause, not a string a different error
/// could coincidentally also produce.
#[derive(Debug)]
pub enum TenantHelloError {
    Security(SecurityError),
}

impl From<SecurityError> for TenantHelloError {
    fn from(e: SecurityError) -> Self {
        Self::Security(e)
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
        match self {
            Self::Security(e) => e.to_string(),
        }
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
        matches!(result, Err(TenantHelloError::Security(SecurityError::MissingContext))),
        "tenant-scoped op resolved via `resolve` must fail closed with the same \
         SecurityError::MissingContext the hand-rolled path (tenant_scoped_codegen.rs) reports, \
         got {result:?}"
    );
}
