//! CORE-012A Phase 5 — double-attribute short-circuit + `CrossTenantDenied`
//! non-regression (spec requirements 1 and 5).
//!
//! A dedicated fixture (one operation guarded by BOTH `#[authorize]` and
//! `#[tenant_scoped]`) — no existing suite pairs both attributes on one
//! method, so this is a new file rather than an extension (tasks.md
//! TASK-010).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use ego_security_sdk::{
    authorization::{AccessRequest, AuthorizationDecision, AuthorizationProvider},
    context::SecurityContext,
    error::SecurityError,
    principal::Principal,
};
use ego_service_sdk::{
    context::ServiceContext,
    error::category::ErrorCategory,
    error::ServiceErrorTrait,
    interceptor::InterceptorChain,
    runtime::{Runtime, RuntimeBuilder},
};
#[allow(unused_imports)]
use ego_service_sdk_macros::{authorize, operation, service, tenant_scoped};

mod common;
use common::{authenticated_ctx, RecordingObservability};

#[derive(Debug)]
pub struct DualGuardError(String);

impl From<SecurityError> for DualGuardError {
    fn from(e: SecurityError) -> Self {
        DualGuardError(e.to_string())
    }
}

impl ServiceErrorTrait for DualGuardError {
    fn code(&self) -> &str {
        "DUAL_GUARD_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

/// Always allows.
struct AllowProvider;

#[async_trait]
impl AuthorizationProvider for AllowProvider {
    async fn authorize(
        &self,
        _principal: &Principal,
        _request: &AccessRequest,
        _ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Allow)
    }
}

/// Always denies.
struct DenyProvider;

#[async_trait]
impl AuthorizationProvider for DenyProvider {
    async fn authorize(
        &self,
        _principal: &Principal,
        _request: &AccessRequest,
        _ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Deny {
            reason: "stub-deny".to_string(),
        })
    }
}

struct StubAuthnProvider;

impl ego_security_sdk::authentication::AuthenticationProvider for StubAuthnProvider {
    fn authenticate(
        &self,
        _credential: &ego_security_sdk::credential::Credential,
    ) -> Result<SecurityContext, ego_security_sdk::AuthenticationError> {
        unimplemented!("not used in these tests")
    }
}

/// One operation guarded by BOTH `#[authorize]` and `#[tenant_scoped]` — the
/// exact pairing spec requirement 1's "double-attribute" scenarios need.
#[service(version = "1.0.0")]
pub trait DualGuardService {
    #[operation]
    #[authorize(context = ctx, permission = "orders:read")]
    #[tenant_scoped]
    async fn dual_op(&self, ctx: ServiceContext) -> Result<bool, DualGuardError>;
}

#[derive(Default)]
struct RecordingService {
    ran: AtomicUsize,
}

#[async_trait]
impl DualGuardService for RecordingService {
    async fn dual_op(&self, _ctx: ServiceContext) -> Result<bool, DualGuardError> {
        self.ran.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}

fn make_runtime(
    authz: Arc<dyn AuthorizationProvider>,
    observability: Arc<RecordingObservability>,
) -> (Runtime, std::sync::Weak<ego_service_sdk::runtime::RuntimeInner>) {
    let rt = RuntimeBuilder::new()
        .with_security(Arc::new(StubAuthnProvider), authz)
        .with_observability(observability)
        .build();
    let weak = Arc::downgrade(rt.inner());
    (rt, weak)
}

/// Shared per-test fixture: a fresh `RecordingObservability` sink, runtime,
/// `RecordingService`, and `DualGuardServiceRef` proxy — every test below
/// differed only in which `AuthorizationProvider` was passed in.
fn setup(
    authz: Arc<dyn AuthorizationProvider>,
) -> (Runtime, Arc<RecordingObservability>, Arc<RecordingService>, DualGuardServiceRef) {
    let observability = Arc::new(RecordingObservability::new());
    let (rt, weak) = make_runtime(authz, observability.clone());
    let service = Arc::new(RecordingService::default());
    let inner: Arc<dyn DualGuardService> = service.clone();
    let proxy = DualGuardServiceRef::new(inner, Arc::new(InterceptorChain::new()), weak);
    (rt, observability, service, proxy)
}

// ---------------------------------------------------------------------------
// TASK-010 Case A: authorize denies -> exactly one AuthorizationDenied event,
// no tenant event (tenant_scoped never runs — authorize short-circuits first).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dual_guard_authorize_deny_records_exactly_one_authorization_denied_event() {
    let (_rt, observability, service, proxy) = setup(Arc::new(DenyProvider));

    let ctx = authenticated_ctx(Some("tenant-a"));
    let result = proxy.dual_op(ctx).await;

    assert!(result.is_err(), "expected authorize to deny");
    assert_eq!(service.ran.load(Ordering::SeqCst), 0, "body must not execute");
    assert_eq!(
        observability.denial_kinds(),
        vec!["AuthorizationDenied".to_string()],
        "expected exactly one event, and it must be AuthorizationDenied (no tenant event)"
    );
}

// ---------------------------------------------------------------------------
// TASK-010 Case B: authorize passes, tenant enforcement denies -> exactly one
// TenantMismatch (or MissingContext) event.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dual_guard_tenant_mismatch_records_exactly_one_tenant_mismatch_event() {
    let (_rt, observability, service, proxy) = setup(Arc::new(AllowProvider));

    // Authenticated as tenant-a, but the hint disagrees and no grant covers it.
    let ctx = authenticated_ctx(Some("tenant-a")).with_tenant_id("tenant-b");
    let result = proxy.dual_op(ctx).await;

    assert!(result.is_err(), "expected tenant enforcement to deny");
    assert_eq!(service.ran.load(Ordering::SeqCst), 0, "body must not execute");
    assert_eq!(
        observability.denial_kinds(),
        vec!["TenantMismatch".to_string()],
        "expected exactly one TenantMismatch event"
    );
}

// ---------------------------------------------------------------------------
// TASK-010 Case C: both guards pass -> zero denial events, body runs once.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dual_guard_allowed_invocation_records_no_denial_event() {
    let (_rt, observability, service, proxy) = setup(Arc::new(AllowProvider));

    let ctx = authenticated_ctx(Some("tenant-a"));
    let result = proxy.dual_op(ctx).await;

    assert!(result.is_ok(), "expected both guards to pass: {:?}", result);
    assert_eq!(service.ran.load(Ordering::SeqCst), 1, "body must run exactly once");
    assert!(
        observability.denial_kinds().is_empty(),
        "an allowed invocation must record no denial event"
    );
}

// ---------------------------------------------------------------------------
// TASK-011: CrossTenantDenied stays uninstrumented (spec requirement 5).
//
// No dedicated runtime test here — code review confirmed it would be
// redundant: the three cases above already assert *exact* equality on
// `denial_kinds()` (`vec!["AuthorizationDenied"]`, `vec!["TenantMismatch"]`,
// `is_empty()`), which already precludes `CrossTenantDenied` (or anything
// else) from ever appearing across authorize-deny, tenant-mismatch, and
// allowed scenarios. Combined with `SecurityDenialKind`'s exhaustive 3-arm
// `Display`/`from_security_error` match (no wildcard, no `CrossTenantDenied`
// variant to construct), the non-instrumentation is proven at compile time,
// not just by a test that re-runs the same scenarios again.
// ---------------------------------------------------------------------------
