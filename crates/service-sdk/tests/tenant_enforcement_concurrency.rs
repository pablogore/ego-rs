//! Concurrency, retry, and clone-behavior tests for tenant resolution
//! (CORE-008A Phase 5, Mandatory Seed 3 — TASK-020/021/022).
//!
//! These exercise `RuntimeInner::enforce_tenant` (public) through the public
//! `RuntimeBuilder`/`Runtime` surface, proving `CanonicalTenant` behaves as
//! the small, owned, per-operation value AD-004/AD-005 require: no shared
//! mutable state between concurrent resolutions, idempotent on retry, and
//! immutable/non-divergent across a `ServiceContext` clone.

use std::sync::Arc;

use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::runtime::RuntimeBuilder;

fn authenticated_ctx(tenant: &str) -> ServiceContext {
    let mut principal = Principal::new(PrincipalKind::User, SubjectId::new("alice").unwrap());
    principal.tenant_id = Some(tenant.to_string());
    let security = SecurityContext::empty(principal);
    ServiceContext::new().with_security(Arc::new(security))
}

fn resolved_tenant_id(ctx: &ServiceContext) -> Option<&str> {
    ctx.canonical_tenant().and_then(|c| c.tenant_id()).map(|t| t.as_str())
}

// TASK-020: two concurrent operations carrying different tenant hints must
// resolve independently, with no cross-contamination — `CanonicalTenant` is
// a small owned value per AD-004/AD-005, not shared state.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_operations_with_different_tenants_do_not_cross_contaminate() {
    let runtime = RuntimeBuilder::new().build();
    let inner_a = runtime.inner().clone();
    let inner_b = runtime.inner().clone();

    let handle_a = tokio::spawn(async move {
        let mut ctx = authenticated_ctx("tenant-a");
        inner_a.enforce_tenant(&mut ctx).expect("tenant-a resolves");
        resolved_tenant_id(&ctx).map(str::to_owned)
    });
    let handle_b = tokio::spawn(async move {
        let mut ctx = authenticated_ctx("tenant-b");
        inner_b.enforce_tenant(&mut ctx).expect("tenant-b resolves");
        resolved_tenant_id(&ctx).map(str::to_owned)
    });

    let (resolved_a, resolved_b) = tokio::join!(handle_a, handle_b);
    assert_eq!(resolved_a.unwrap().as_deref(), Some("tenant-a"));
    assert_eq!(resolved_b.unwrap().as_deref(), Some("tenant-b"));
}

// TASK-021: a caller retrying the same operation after a transient failure
// (e.g. a provider hiccup unrelated to tenant resolution) with the same
// Principal/hint resolves to the identical CanonicalTenant — the resolver
// holds no mutable state (AD-001), so a retry can never observe leftovers
// from the failed attempt.
#[test]
fn retried_call_resolves_to_the_identical_canonical_tenant() {
    let runtime = RuntimeBuilder::new().build();
    let inner = runtime.inner();

    let mut first_attempt = authenticated_ctx("tenant-a");
    inner
        .enforce_tenant(&mut first_attempt)
        .expect("first attempt resolves");

    // Retry: a fresh ServiceContext, same Principal/tenant, as a real
    // retried call would construct.
    let mut retried_attempt = authenticated_ctx("tenant-a");
    inner
        .enforce_tenant(&mut retried_attempt)
        .expect("retried attempt resolves");

    assert_eq!(
        resolved_tenant_id(&first_attempt),
        resolved_tenant_id(&retried_attempt)
    );
    assert_eq!(resolved_tenant_id(&retried_attempt), Some("tenant-a"));
}

// TASK-022: `ServiceContext` clone behavior under tenant resolution.
#[test]
fn clone_before_resolution_neither_copy_has_a_canonical_tenant() {
    let ctx = authenticated_ctx("tenant-a");
    let cloned = ctx.clone();

    assert!(ctx.canonical_tenant().is_none());
    assert!(cloned.canonical_tenant().is_none());
}

#[test]
fn clone_after_resolution_carries_the_same_canonical_tenant_and_cannot_diverge() {
    let runtime = RuntimeBuilder::new().build();
    let mut ctx = authenticated_ctx("tenant-a");
    runtime
        .inner()
        .enforce_tenant(&mut ctx)
        .expect("resolves");

    let cloned = ctx.clone();

    // Both the original and a fresh clone carry the same resolved value.
    assert_eq!(ctx.canonical_tenant(), cloned.canonical_tenant());
    assert_eq!(resolved_tenant_id(&ctx), Some("tenant-a"));
    assert_eq!(resolved_tenant_id(&cloned), Some("tenant-a"));

    // No public mutator exists (AD-004) — there is no API through which
    // either copy could independently diverge from the other.
}
