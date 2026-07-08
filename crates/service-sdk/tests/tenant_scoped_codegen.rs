//! Behavioral test for CORE-008A TASK-011/012 — `#[tenant_scoped]` codegen.
//!
//! Compile + runtime test (mirrors `proxy_codegen.rs`'s style — not a
//! literal-token trybuild assertion): proves observably that a
//! `#[tenant_scoped]` operation's generated call site is fallible and aborts
//! before the inner body runs on an unresolvable tenant, while an unmarked
//! operation in the SAME trait keeps discarding `enforce_tenant`'s `Result`
//! (zero behavior change, TASK-013).
//!
//! Run with: cargo test -p ego-service-sdk tenant_scoped_codegen

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk::interceptor::InterceptorChain;
use ego_service_sdk::runtime::{Runtime, RuntimeBuilder};
use ego_service_sdk::security::SecurityError;
#[allow(unused_imports)]
use ego_service_sdk_macros::{operation, tenant_scoped};
use ego_service_sdk_macros::service;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug)]
pub struct TenantTestError(String);

impl From<SecurityError> for TenantTestError {
    fn from(e: SecurityError) -> Self {
        TenantTestError(e.to_string())
    }
}

impl ServiceErrorTrait for TenantTestError {
    fn code(&self) -> &str {
        "TENANT_TEST_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

/// One `#[tenant_scoped]` operation and one plain operation, side by side —
/// the exact pairing TASK-011 requires to prove the generated call sites
/// differ only for the marked method.
#[service(version = "1.0.0")]
pub trait MixedTenantService {
    #[operation]
    #[tenant_scoped]
    async fn scoped_op(&self, ctx: ServiceContext) -> Result<bool, TenantTestError>;

    #[operation]
    async fn unscoped_op(&self, ctx: ServiceContext) -> Result<bool, TenantTestError>;
}

/// Records whether each method's body executed — proves the marked method's
/// body is never entered on enforcement failure (FR-009) while the unmarked
/// method's body always runs regardless of tenant resolvability.
#[derive(Default)]
struct RecordingService {
    scoped_body_ran: AtomicBool,
    unscoped_body_ran: AtomicBool,
}

#[async_trait]
impl MixedTenantService for RecordingService {
    async fn scoped_op(&self, ctx: ServiceContext) -> Result<bool, TenantTestError> {
        self.scoped_body_ran.store(true, Ordering::SeqCst);
        Ok(ctx.canonical_tenant().is_some())
    }

    async fn unscoped_op(&self, _ctx: ServiceContext) -> Result<bool, TenantTestError> {
        self.unscoped_body_ran.store(true, Ordering::SeqCst);
        Ok(true)
    }
}

/// Returns `(Runtime, proxy)` — the caller MUST keep `Runtime` alive for as
/// long as the proxy is used: the proxy only holds a `Weak<RuntimeInner>`
/// (mirrors production usage), so dropping the returned `Runtime` early would
/// make every `enforce_tenant` call fail via a dropped-runtime `MissingContext`
/// rather than via genuine tenant resolution — a false-positive this helper
/// avoids by handing ownership back to the test.
fn make_proxy(service: Arc<RecordingService>) -> (Runtime, MixedTenantServiceRef) {
    let inner: Arc<dyn MixedTenantService> = service;
    let chain = Arc::new(InterceptorChain::new());
    let rt = RuntimeBuilder::new().build();
    let runtime_weak = Arc::downgrade(rt.inner());
    let proxy = MixedTenantServiceRef::new(inner, chain, runtime_weak);
    (rt, proxy)
}

#[tokio::test]
async fn tenant_scoped_op_fails_closed_and_never_enters_body_without_resolvable_tenant() {
    let service = Arc::new(RecordingService::default());
    let (_rt, proxy) = make_proxy(service.clone());

    // No security attached -> unresolvable under the default AuthenticatedOnly mode.
    let ctx = ServiceContext::new();

    let result = proxy.scoped_op(ctx).await;

    assert!(
        result.is_err(),
        "tenant-scoped op must fail closed without a resolvable tenant"
    );
    assert!(
        !service.scoped_body_ran.load(Ordering::SeqCst),
        "tenant-scoped op's body must never execute when enforcement fails (FR-009)"
    );
}

#[tokio::test]
async fn unmarked_op_ignores_unresolvable_tenant_and_runs_body() {
    let service = Arc::new(RecordingService::default());
    let (_rt, proxy) = make_proxy(service.clone());

    // Same unresolvable context as above — unmarked op must be unaffected (TASK-013).
    let ctx = ServiceContext::new();

    let result = proxy.unscoped_op(ctx).await;

    assert!(
        result.is_ok(),
        "unmarked op must proceed regardless of tenant resolvability"
    );
    assert!(
        service.unscoped_body_ran.load(Ordering::SeqCst),
        "unmarked op's body must execute exactly as before this change"
    );
}

#[tokio::test]
async fn tenant_scoped_op_succeeds_and_body_observes_canonical_tenant_when_resolvable() {
    use ego_security_sdk::context::SecurityContext;
    use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};

    let service = Arc::new(RecordingService::default());
    let (_rt, proxy) = make_proxy(service.clone());

    let mut principal = Principal::new(PrincipalKind::User, SubjectId::new("alice").unwrap());
    principal.tenant_id = Some("tenant-a".to_string());
    let security = SecurityContext::empty(principal);
    let ctx = ServiceContext::new().with_security(Arc::new(security));

    let result = proxy.scoped_op(ctx).await;

    assert!(result.is_ok(), "tenant-scoped op must succeed with a resolvable tenant");
    assert!(
        result.unwrap(),
        "body must observe canonical_tenant() populated by enforce_tenant before it runs"
    );
    assert!(service.scoped_body_ran.load(Ordering::SeqCst));
}
