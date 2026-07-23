// Integration tests for CORE-015 — runtime behavior of the `#[authorize]` guard.
//
// These tests cover T-18 to T-24 (Phase 5):
//   T-18: Allow path — provider allows; body executes (AC-5.5)
//   T-19: Deny path — provider denies; body does not execute (AC-4.1, AC-5.4)
//   T-20: Exactly one authorize_in_context call per annotated invocation (AC-4.2)
//   T-21: Missing SecurityContext — ctx.security() is None; returns Err(MissingContext), body does NOT execute (AC-5.1)
//   T-22: Runtime dropped — Weak::upgrade() returns None; Err(ProviderError) (AC-5.2)
//   T-23: CapabilityNotEnabled path — authz resolves to CapabilityNotEnabled (AC-5.3)
//   T-24: Multiple annotated methods — guards are independent per method

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use ego_security_sdk::{
    authorization::{AccessRequest, AuthorizationDecision, AuthorizationProvider},
    context::SecurityContext,
    error::SecurityError,
    principal::{Principal, PrincipalKind, SubjectId},
};
use ego_service_sdk::{
    context::ServiceContext,
    error::category::ErrorCategory,
    error::ServiceErrorTrait,
    interceptor::InterceptorChain,
    runtime::{Runtime, RuntimeBuilder, RuntimeInner},
};
#[allow(unused_imports)]
use ego_service_sdk_macros::{authorize, operation, service};

mod common;
use common::RecordingObservability;

// ---------------------------------------------------------------------------
// Shared error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct AuthTestError(String);

impl From<SecurityError> for AuthTestError {
    fn from(e: SecurityError) -> Self {
        AuthTestError(e.to_string())
    }
}

impl ServiceErrorTrait for AuthTestError {
    fn code(&self) -> &str {
        "AUTH_TEST_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

// ---------------------------------------------------------------------------
// Stub authorization providers
// ---------------------------------------------------------------------------

/// Always allows; counts every `authorize` call.
struct CountingAllowProvider {
    calls: AtomicUsize,
}

impl CountingAllowProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl AuthorizationProvider for CountingAllowProvider {
    async fn authorize(
        &self,
        _principal: &Principal,
        _request: &AccessRequest,
        _ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(AuthorizationDecision::Allow)
    }
}

/// Always denies.
struct StubDenyProvider;

#[async_trait]
impl AuthorizationProvider for StubDenyProvider {
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

/// Always returns CapabilityNotEnabled (models "authz not configured").
struct CapabilityNotEnabledProvider;

#[async_trait]
impl AuthorizationProvider for CapabilityNotEnabledProvider {
    async fn authorize(
        &self,
        _principal: &Principal,
        _request: &AccessRequest,
        _ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Err(SecurityError::CapabilityNotEnabled)
    }
}

/// Stub authentication provider — never called in these tests.
struct StubAuthnProvider;

impl ego_security_sdk::authentication::AuthenticationProvider for StubAuthnProvider {
    fn authenticate(
        &self,
        _credential: &ego_security_sdk::credential::Credential,
    ) -> Result<SecurityContext, ego_security_sdk::AuthenticationError> {
        unimplemented!("not used in authorization integration tests")
    }
}

// ---------------------------------------------------------------------------
// Service under test
// ---------------------------------------------------------------------------

/// Service body tracks whether it ran.
struct TrackingOrderService {
    body_ran: Arc<AtomicUsize>,
}

#[async_trait]
impl OrderService for TrackingOrderService {
    async fn get_order(&self, _ctx: ServiceContext, _id: String) -> Result<String, AuthTestError> {
        self.body_ran.fetch_add(1, Ordering::Relaxed);
        Ok("order-result".to_string())
    }
}

#[service(version = "1.0.0")]
pub trait OrderService {
    #[operation]
    #[authorize(context = ctx, permission = "orders:read")]
    async fn get_order(&self, ctx: ServiceContext, id: String) -> Result<String, AuthTestError>;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_security_ctx() -> SecurityContext {
    let subject = SubjectId::new("user:test").unwrap();
    let principal = Principal::new(PrincipalKind::User, subject);
    SecurityContext::empty(principal)
}

/// Build a live `Runtime` with the given authorization provider and return
/// both the `Runtime` (keeps the inner Arc alive) and a Weak reference to its
/// `RuntimeInner`.
fn make_runtime(authz: Arc<dyn AuthorizationProvider>) -> (Runtime, std::sync::Weak<RuntimeInner>) {
    make_runtime_with_observability(authz, None)
}

/// Same as `make_runtime`, plus an optional `Observability` implementor
/// (CORE-012A) — extends the shared fixture instead of duplicating it, per
/// tasks.md TASK-008.
fn make_runtime_with_observability(
    authz: Arc<dyn AuthorizationProvider>,
    observability: Option<Arc<dyn ego_domain::Observability>>,
) -> (Runtime, std::sync::Weak<RuntimeInner>) {
    let mut builder = RuntimeBuilder::new().with_security(Arc::new(StubAuthnProvider), authz);
    if let Some(obs) = observability {
        builder = builder.with_observability(obs);
    }
    let rt = builder.build();
    let weak = Arc::downgrade(rt.inner());
    (rt, weak)
}

/// Build a proxy backed by the given runtime weak ref.
fn make_proxy(
    body_ran: Arc<AtomicUsize>,
    runtime_weak: std::sync::Weak<RuntimeInner>,
) -> OrderServiceRef {
    let inner: Arc<dyn OrderService> = Arc::new(TrackingOrderService { body_ran });
    OrderServiceRef::new(inner, Arc::new(InterceptorChain::new()), runtime_weak)
}

// ---------------------------------------------------------------------------
// T-18: Allow path — body executes when provider allows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t18_allow_path_body_executes() {
    let provider = Arc::new(CountingAllowProvider::new());
    let (_rt, weak) = make_runtime(provider.clone());
    let body_ran = Arc::new(AtomicUsize::new(0));
    let proxy = make_proxy(body_ran.clone(), weak);

    let sec = Arc::new(make_security_ctx());
    let ctx = ServiceContext::new().with_security(Arc::clone(&sec));

    let result = proxy.get_order(ctx, "order-1".to_string()).await;

    assert!(result.is_ok(), "expected Ok, got: {:?}", result);
    assert_eq!(result.unwrap(), "order-result");
    assert_eq!(
        body_ran.load(Ordering::Relaxed),
        1,
        "body must have executed exactly once"
    );
}

// ---------------------------------------------------------------------------
// T-19: Deny path — body does NOT execute when provider denies
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t19_deny_path_body_does_not_execute() {
    let provider = Arc::new(StubDenyProvider);
    let observability = Arc::new(RecordingObservability::new());
    let (_rt, weak) = make_runtime_with_observability(provider, Some(observability.clone()));
    let body_ran = Arc::new(AtomicUsize::new(0));
    let proxy = make_proxy(body_ran.clone(), weak);

    let sec = Arc::new(make_security_ctx());
    let ctx = ServiceContext::new().with_security(Arc::clone(&sec));

    let result = proxy.get_order(ctx, "order-1".to_string()).await;

    assert!(result.is_err(), "expected Err from deny, got: {:?}", result);
    let err_str = result.unwrap_err().0;
    assert!(
        err_str.contains("authorization denied"),
        "expected 'authorization denied' in error, got: {err_str}"
    );
    assert_eq!(
        body_ran.load(Ordering::Relaxed),
        0,
        "body must NOT execute on deny"
    );

    // CORE-012A: exactly one AuthorizationDenied event recorded (site: lib.rs:312).
    assert_eq!(
        observability.denial_kinds(),
        vec!["AuthorizationDenied".to_string()],
        "expected exactly one AuthorizationDenied event"
    );
}

// ---------------------------------------------------------------------------
// T-20: Exactly one authorize_in_context call per annotated method invocation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t20_exactly_one_authorize_call_per_invocation() {
    let provider = Arc::new(CountingAllowProvider::new());
    let (_rt, weak) = make_runtime(provider.clone());
    let body_ran = Arc::new(AtomicUsize::new(0));
    let proxy = make_proxy(body_ran.clone(), weak);

    let sec = Arc::new(make_security_ctx());

    // Three separate calls.
    for i in 0..3u32 {
        let ctx = ServiceContext::new().with_security(Arc::clone(&sec));
        let _ = proxy.get_order(ctx, format!("order-{i}")).await;
    }

    assert_eq!(
        provider.call_count(),
        3,
        "provider must be called exactly once per method invocation"
    );
}

// ---------------------------------------------------------------------------
// T-21: Missing SecurityContext — ctx.security() is None; returns Err(MissingContext), body does NOT execute
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t21_missing_security_context_fails_closed() {
    // ctx has no SecurityContext — #[authorize] must fail closed with MissingContext.
    let provider = Arc::new(CountingAllowProvider::new());
    let observability = Arc::new(RecordingObservability::new());
    let (_rt, weak) =
        make_runtime_with_observability(provider.clone(), Some(observability.clone()));
    let body_ran = Arc::new(AtomicUsize::new(0));
    let proxy = make_proxy(body_ran.clone(), weak);

    let ctx = ServiceContext::new(); // no with_security()

    let result = proxy.get_order(ctx, "order-1".to_string()).await;

    assert!(
        result.is_err(),
        "expected Err when SecurityContext is absent"
    );
    assert!(
        result.unwrap_err().0.contains("missing security context"),
        "expected MissingContext error"
    );
    assert_eq!(
        body_ran.load(Ordering::Relaxed),
        0,
        "body must NOT execute when SecurityContext is absent"
    );
    assert_eq!(
        provider.call_count(),
        0,
        "provider must NOT be called when SecurityContext is absent"
    );

    // CORE-012A: exactly one MissingContext event recorded (site: lib.rs:285).
    assert_eq!(
        observability.denial_kinds(),
        vec!["MissingContext".to_string()],
        "expected exactly one MissingContext event"
    );
}

// ---------------------------------------------------------------------------
// T-22: Runtime dropped — Weak::upgrade() returns None; Err(ProviderError)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t22_runtime_dropped_returns_provider_error() {
    let provider = Arc::new(CountingAllowProvider::new());
    let observability = Arc::new(RecordingObservability::new());
    let (rt, weak) = make_runtime_with_observability(provider, Some(observability.clone()));
    let body_ran = Arc::new(AtomicUsize::new(0));
    let proxy = make_proxy(body_ran.clone(), weak);

    // Drop the strong Arc — Weak::upgrade() will now return None.
    drop(rt);

    let sec = Arc::new(make_security_ctx());
    let ctx = ServiceContext::new().with_security(Arc::clone(&sec));

    let result = proxy.get_order(ctx, "order-1".to_string()).await;

    assert!(
        result.is_err(),
        "expected Err when runtime is dropped, got: {:?}",
        result
    );
    let err_str = result.unwrap_err().0;
    assert!(
        err_str.contains("provider error"),
        "expected 'provider error' in error, got: {err_str}"
    );
    assert_eq!(
        body_ran.load(Ordering::Relaxed),
        0,
        "body must NOT execute when runtime is dropped"
    );

    // CORE-012A: ProviderError (dropped runtime) is an infra failure, not one
    // of the 3 spec denial kinds — design.md explicitly excludes it. No event.
    assert!(
        observability.denial_kinds().is_empty(),
        "dropped-runtime ProviderError must not record any denial event"
    );
}

// ---------------------------------------------------------------------------
// CORE-012A regression: MissingContext precedes ProviderError when BOTH the
// SecurityContext is absent AND the runtime is dropped (TASK-009 site-1
// reorder must not change this precedence — the security-context check must
// still be evaluated, and fail, before the runtime-upgrade requirement is
// enforced).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_context_precedes_dropped_runtime_provider_error() {
    let provider = Arc::new(CountingAllowProvider::new());
    let observability = Arc::new(RecordingObservability::new());
    let (rt, weak) = make_runtime_with_observability(provider.clone(), Some(observability.clone()));
    let body_ran = Arc::new(AtomicUsize::new(0));
    let proxy = make_proxy(body_ran.clone(), weak);

    // Drop the strong Arc first — Weak::upgrade() will return None.
    drop(rt);

    // No SecurityContext either — both failure conditions are true at once.
    let ctx = ServiceContext::new();

    let result = proxy.get_order(ctx, "order-1".to_string()).await;

    assert!(result.is_err(), "expected Err, got: {:?}", result);
    let err_str = result.unwrap_err().0;
    assert!(
        err_str.contains("missing security context"),
        "expected MissingContext to win over ProviderError when both apply, got: {err_str}"
    );
    assert_eq!(body_ran.load(Ordering::Relaxed), 0, "body must NOT execute");
    assert_eq!(provider.call_count(), 0, "provider must NOT be called");

    // No event is recorded here: with the runtime already dropped, there is
    // no live RuntimeInner handle left to record through (recording requires
    // an upgraded Weak, same as the error path itself needs one for the
    // provider lookup) — this is a physical consequence of the dropped
    // runtime, not a precedence violation. The returned error value above is
    // the actual precedence proof: it is MissingContext, not ProviderError.
    assert!(
        observability.denial_kinds().is_empty(),
        "no event can be recorded once the runtime handle itself is gone"
    );
}

// ---------------------------------------------------------------------------
// T-23: CapabilityNotEnabled — authz resolves to CapabilityNotEnabled
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t23_capability_not_enabled_returns_error() {
    let provider = Arc::new(CapabilityNotEnabledProvider);
    let observability = Arc::new(RecordingObservability::new());
    let (_rt, weak) = make_runtime_with_observability(provider, Some(observability.clone()));
    let body_ran = Arc::new(AtomicUsize::new(0));
    let proxy = make_proxy(body_ran.clone(), weak);

    let sec = Arc::new(make_security_ctx());
    let ctx = ServiceContext::new().with_security(Arc::clone(&sec));

    let result = proxy.get_order(ctx, "order-1".to_string()).await;

    assert!(
        result.is_err(),
        "expected Err for CapabilityNotEnabled, got: {:?}",
        result
    );
    let err_str = result.unwrap_err().0;
    assert!(
        err_str.contains("security capability not enabled"),
        "expected 'security capability not enabled' in error, got: {err_str}"
    );
    assert_eq!(
        body_ran.load(Ordering::Relaxed),
        0,
        "body must NOT execute when CapabilityNotEnabled"
    );

    // CORE-012A: CapabilityNotEnabled is an infra failure, not one of the 3
    // spec denial kinds — design.md explicitly excludes it. No event.
    assert!(
        observability.denial_kinds().is_empty(),
        "CapabilityNotEnabled must not record any denial event"
    );
}

// ---------------------------------------------------------------------------
// T-24: Multiple annotated methods — guards are independent per method
// ---------------------------------------------------------------------------

#[service(version = "1.0.0")]
pub trait MultiOpService {
    #[operation]
    #[authorize(context = ctx, permission = "orders:read")]
    async fn read_order(&self, ctx: ServiceContext, id: String) -> Result<String, AuthTestError>;

    #[operation]
    #[authorize(context = ctx, permission = "orders:write")]
    async fn write_order(&self, ctx: ServiceContext, id: String) -> Result<String, AuthTestError>;
}

struct TrackingMultiOpService {
    read_ran: Arc<AtomicUsize>,
    write_ran: Arc<AtomicUsize>,
}

#[async_trait]
impl MultiOpService for TrackingMultiOpService {
    async fn read_order(&self, _ctx: ServiceContext, _id: String) -> Result<String, AuthTestError> {
        self.read_ran.fetch_add(1, Ordering::Relaxed);
        Ok("read-result".to_string())
    }

    async fn write_order(
        &self,
        _ctx: ServiceContext,
        _id: String,
    ) -> Result<String, AuthTestError> {
        self.write_ran.fetch_add(1, Ordering::Relaxed);
        Ok("write-result".to_string())
    }
}

#[tokio::test]
async fn t24_multiple_annotated_methods_independent_guards() {
    // Allow provider — counts every call.
    let provider = Arc::new(CountingAllowProvider::new());
    let (_rt, weak) = make_runtime(provider.clone());

    let read_ran = Arc::new(AtomicUsize::new(0));
    let write_ran = Arc::new(AtomicUsize::new(0));

    let inner: Arc<dyn MultiOpService> = Arc::new(TrackingMultiOpService {
        read_ran: read_ran.clone(),
        write_ran: write_ran.clone(),
    });
    let proxy = MultiOpServiceRef::new(inner, Arc::new(InterceptorChain::new()), weak);

    let sec = Arc::new(make_security_ctx());

    let ctx_read = ServiceContext::new().with_security(Arc::clone(&sec));
    let result_read = proxy.read_order(ctx_read, "ord-1".to_string()).await;

    let ctx_write = ServiceContext::new().with_security(Arc::clone(&sec));
    let result_write = proxy.write_order(ctx_write, "ord-1".to_string()).await;

    assert!(
        result_read.is_ok(),
        "read_order must succeed: {:?}",
        result_read
    );
    assert!(
        result_write.is_ok(),
        "write_order must succeed: {:?}",
        result_write
    );

    assert_eq!(
        read_ran.load(Ordering::Relaxed),
        1,
        "read body must run once"
    );
    assert_eq!(
        write_ran.load(Ordering::Relaxed),
        1,
        "write body must run once"
    );
    // Two method calls → two authorization checks.
    assert_eq!(
        provider.call_count(),
        2,
        "each annotated method invocation must trigger exactly one authorization check"
    );
}
