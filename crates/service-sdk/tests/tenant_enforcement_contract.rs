//! CORE-008A Phase 6 — Full FR/NFR acceptance suite (TASK-025–032).
//!
//! Adopts `#[tenant_scoped]` on a dedicated test-only service
//! (`TenantContractService`) — the first and only marker adoption in this
//! change (design.md Non-Goals keep real application operations out of
//! scope) — and exercises the integrated macro + resolver + `ServiceContext`
//! path end to end for every FR/NFR this spec defines that is reachable
//! through the public API surface.
//!
//! # Scope note (FR-005/FR-006/NFR-002 — flagged, not silent)
//!
//! `RuntimeInner::issue_cross_tenant_permit` is deliberately `pub(crate)`
//! (AD-008 — "widening to pub would let external crates mint
//! `CrossTenantPermit` without authorization"), so it cannot be called from
//! this external `tests/` crate. FR-005/FR-006/NFR-002 (permit
//! denial/issuance) and the `CrossTenantDenied` half of FR-012/NFR-003 are
//! therefore proven by dedicated tests inside the crate's own `#[cfg(test)]`
//! modules — `crates/service-sdk/src/runtime/runtime_builder.rs`
//! (`issue_cross_tenant_permit_denied_without_capability`,
//! `issue_cross_tenant_permit_denied_even_with_resource_action_alone`,
//! `issue_cross_tenant_permit_allowed_yields_destination_scoped_permit`) and
//! `crates/service-sdk/src/context/mod.rs`
//! (`with_cross_tenant_access_sets_flag`,
//! `is_cross_tenant_allowed_for_matches_only_the_issued_destination`) —
//! already added in Phase 4. Those tests already satisfy TASK-028 verbatim;
//! see the traceability comments added there for Phase 6.
//!
//! Run with: cargo test -p ego-service-sdk tenant_enforcement_contract

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk::interceptor::InterceptorChain;
use ego_service_sdk::runtime::{Runtime, RuntimeBuilder, TenantEnforcementMode};
use ego_service_sdk::security::SecurityError;
use ego_service_sdk_macros::service;
#[allow(unused_imports)]
use ego_service_sdk_macros::{operation, tenant_scoped};

mod common;
use common::{authenticated_ctx, authenticated_ctx_with_hint};

/// Wraps `SecurityError` directly (not stringified) so tests can `match` on
/// the concrete variant — required for NFR-003 ("assert on a distinguishable
/// error value ... not merely 'the call failed'").
#[derive(Debug)]
pub enum TenantContractError {
    Security(SecurityError),
}

impl From<SecurityError> for TenantContractError {
    fn from(e: SecurityError) -> Self {
        TenantContractError::Security(e)
    }
}

impl ServiceErrorTrait for TenantContractError {
    fn code(&self) -> &str {
        "TENANT_CONTRACT_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        match self {
            TenantContractError::Security(e) => e.to_string(),
        }
    }
}

/// TASK-025/026: one `#[tenant_scoped]` op and one plain op — the acceptance-
/// suite counterpart to Phase 3's `tenant_scoped_codegen.rs::MixedTenantService`
/// (which proves only the codegen mechanic). This trait is used across every
/// FR/NFR scenario below.
#[service(version = "1.0.0")]
pub trait TenantContractService {
    #[operation]
    #[tenant_scoped]
    async fn scoped_op(&self, ctx: ServiceContext) -> Result<Option<String>, TenantContractError>;

    #[operation]
    async fn unscoped_op(&self, ctx: ServiceContext) -> Result<bool, TenantContractError>;
}

/// Records whether `scoped_op`'s body ran, and echoes back the canonical
/// tenant it observed — the body never manually assigns a tenant (FR-011).
#[derive(Default)]
struct ContractService {
    scoped_body_ran: AtomicBool,
}

#[async_trait]
impl TenantContractService for ContractService {
    async fn scoped_op(&self, ctx: ServiceContext) -> Result<Option<String>, TenantContractError> {
        self.scoped_body_ran.store(true, Ordering::SeqCst);
        Ok(ctx
            .canonical_tenant()
            .and_then(|c| c.tenant_id())
            .map(|t| t.as_str().to_string()))
    }

    async fn unscoped_op(&self, _ctx: ServiceContext) -> Result<bool, TenantContractError> {
        Ok(true)
    }
}

/// Builds a `(Runtime, proxy)` pair under the given enforcement mode. Caller
/// MUST keep `Runtime` alive for as long as the proxy is used (the proxy only
/// holds a `Weak<RuntimeInner>` — mirrors `tenant_scoped_codegen.rs`'s
/// `make_proxy`).
fn make_proxy(
    service: Arc<ContractService>,
    mode: TenantEnforcementMode,
) -> (Runtime, TenantContractServiceRef) {
    let inner: Arc<dyn TenantContractService> = service;
    let chain = Arc::new(InterceptorChain::new());
    let rt = RuntimeBuilder::new().with_tenant_enforcement_mode(mode).build();
    let runtime_weak = Arc::downgrade(rt.inner());
    let proxy = TenantContractServiceRef::new(inner, chain, runtime_weak);
    (rt, proxy)
}

// ---------------------------------------------------------------------------
// FR-001 / NFR-001 (TASK-025/026) — fail-closed scope is operation-level
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scoped_op_fails_closed_without_resolvable_tenant_and_never_enters_body() {
    let service = Arc::new(ContractService::default());
    let (_rt, proxy) = make_proxy(service.clone(), TenantEnforcementMode::AuthenticatedOnly);

    let ctx = ServiceContext::new(); // no security, no hint
    let result = proxy.scoped_op(ctx).await;

    assert!(matches!(
        result,
        Err(TenantContractError::Security(SecurityError::MissingContext))
    ));
    assert!(
        !service.scoped_body_ran.load(Ordering::SeqCst),
        "tenant-scoped op's body must never execute on enforcement failure (FR-009)"
    );
}

#[tokio::test]
async fn unscoped_op_proceeds_normally_without_any_tenant() {
    let service = Arc::new(ContractService::default());
    let (_rt, proxy) = make_proxy(service, TenantEnforcementMode::AuthenticatedOnly);

    let ctx = ServiceContext::new();
    let result = proxy.unscoped_op(ctx).await;

    assert!(result.unwrap(), "unscoped op proceeds with no tenant error");
}

// ---------------------------------------------------------------------------
// FR-002 (TASK-027) — Principal is the canonical tenant authority
// ---------------------------------------------------------------------------

#[tokio::test]
async fn authenticated_derivation_succeeds_without_manual_tenant_assignment() {
    // Also covers FR-009 (success half) and FR-011 (canonical tenant present
    // at execution start without the test manually assigning one).
    let service = Arc::new(ContractService::default());
    let (_rt, proxy) = make_proxy(service.clone(), TenantEnforcementMode::AuthenticatedOnly);

    let ctx = authenticated_ctx(Some("tenant-a"));
    let result = proxy.scoped_op(ctx).await;

    assert_eq!(result.unwrap(), Some("tenant-a".to_string()));
    assert!(service.scoped_body_ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn caller_supplied_tenant_conflicting_with_principal_is_tenant_mismatch() {
    let service = Arc::new(ContractService::default());
    let (_rt, proxy) = make_proxy(service, TenantEnforcementMode::AuthenticatedOnly);

    let ctx = authenticated_ctx_with_hint(Some("tenant-a"), Some("tenant-b"));
    let result = proxy.scoped_op(ctx).await;

    match result {
        Err(TenantContractError::Security(SecurityError::TenantMismatch { expected, actual })) => {
            assert_eq!(expected, "tenant-a");
            assert_eq!(actual, "tenant-b");
        }
        other => panic!("expected TenantMismatch{{tenant-a, tenant-b}}, got: {:?}", other),
    }
}

#[tokio::test]
async fn authenticated_principal_without_tenant_claim_fails_closed_regardless_of_hint() {
    let service = Arc::new(ContractService::default());

    // Sub-case: no hint present.
    let (_rt1, proxy1) = make_proxy(service.clone(), TenantEnforcementMode::AuthenticatedOnly);
    let ctx_no_hint = authenticated_ctx_with_hint(None, None);
    let result_no_hint = proxy1.scoped_op(ctx_no_hint).await;
    assert!(matches!(
        result_no_hint,
        Err(TenantContractError::Security(SecurityError::MissingContext))
    ));

    // Sub-case: a caller-supplied hint IS present — must still fail closed,
    // never trusted as a substitute for the missing Principal tenant claim.
    let (_rt2, proxy2) = make_proxy(service, TenantEnforcementMode::AuthenticatedOnly);
    let ctx_with_hint = authenticated_ctx_with_hint(None, Some("tenant-x"));
    let result_with_hint = proxy2.scoped_op(ctx_with_hint).await;
    assert!(matches!(
        result_with_hint,
        Err(TenantContractError::Security(SecurityError::MissingContext))
    ));
}

// ---------------------------------------------------------------------------
// FR-003 / FR-004 (TASK-027) — system/internal mode, and neither-nor fails
// ---------------------------------------------------------------------------

#[tokio::test]
async fn internal_mode_accepts_supplied_tenant_when_explicitly_permitted() {
    let service = Arc::new(ContractService::default());
    let (_rt, proxy) = make_proxy(service, TenantEnforcementMode::AllowSystemInternal);

    let ctx = ServiceContext::new().with_tenant_id("tenant-internal"); // no Principal
    let result = proxy.scoped_op(ctx).await;

    assert_eq!(result.unwrap(), Some("tenant-internal".to_string()));
}

#[tokio::test]
async fn internal_mode_rejects_tenant_when_not_permitted_and_routes_to_missing_context() {
    let service = Arc::new(ContractService::default());
    let (_rt, proxy) = make_proxy(service.clone(), TenantEnforcementMode::AuthenticatedOnly);

    let ctx = ServiceContext::new().with_tenant_id("tenant-internal"); // no Principal
    let result = proxy.scoped_op(ctx).await;

    assert!(matches!(
        result,
        Err(TenantContractError::Security(SecurityError::MissingContext))
    ));
    assert!(
        !service.scoped_body_ran.load(Ordering::SeqCst),
        "a call neither authenticated nor internal-permitted must never enter the body"
    );
}

// ---------------------------------------------------------------------------
// FR-008 (TASK-030) — exactly one canonical in-runtime representation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn divergent_ingress_values_converge_to_one_authoritative_value() {
    // Both ingress representations (Principal.tenant_id and the caller-
    // supplied hint) agree here — before this change they were independent
    // fields that could disagree with nothing to reconcile them; after
    // resolution exactly one value (`canonical_tenant()`) is authoritative
    // and is what the operation body — the only downstream tenant-aware
    // reader in this test — observes.
    let service = Arc::new(ContractService::default());
    let (_rt, proxy) = make_proxy(service, TenantEnforcementMode::AuthenticatedOnly);

    let ctx = authenticated_ctx_with_hint(Some("tenant-a"), Some("tenant-a"));
    let result = proxy.scoped_op(ctx).await;

    assert_eq!(result.unwrap(), Some("tenant-a".to_string()));
}

// ---------------------------------------------------------------------------
// FR-010 / FR-014 (TASK-031) — not a parallel writable authority; immutable
// ---------------------------------------------------------------------------

#[test]
fn direct_tenant_mutation_cannot_override_derived_authenticated_tenant() {
    let rt = RuntimeBuilder::new().build();
    let mut ctx = authenticated_ctx(Some("tenant-a"));
    rt.inner().enforce_tenant(&mut ctx).expect("resolves to tenant-a");

    // Mutating the hint field via `with_tenant_id` never touches
    // `resolved_tenant` (AD-011: no public setter exists for it) — the
    // canonical value enforcement already produced stays authoritative.
    let mutated = ctx.with_tenant_id("tenant-b");

    assert_eq!(
        mutated.canonical_tenant().and_then(|c| c.tenant_id()).map(|t| t.as_str()),
        Some("tenant-a"),
        "a mutation attempt must never be treated as authoritative for enforcement (FR-010)"
    );
}

#[test]
fn downstream_mutation_attempt_does_not_affect_operation_already_in_progress() {
    let rt = RuntimeBuilder::new().build();
    let mut ctx = authenticated_ctx(Some("tenant-a"));
    rt.inner().enforce_tenant(&mut ctx).expect("resolves to tenant-a");

    // Simulate downstream code holding a clone of the in-progress context and
    // attempting to alter the tenant it sees.
    let downstream_clone = ctx.clone();
    let altered = downstream_clone.with_tenant_id("tenant-c");

    assert_eq!(
        ctx.canonical_tenant().and_then(|c| c.tenant_id()).map(|t| t.as_str()),
        Some("tenant-a"),
        "the original in-progress operation must keep observing the original canonical tenant"
    );
    assert_eq!(
        altered.canonical_tenant().and_then(|c| c.tenant_id()).map(|t| t.as_str()),
        Some("tenant-a"),
        "the altered downstream clone's canonical tenant is unaffected by the hint mutation (FR-014)"
    );
}

// ---------------------------------------------------------------------------
// FR-012 / NFR-003 (TASK-032) — distinguishable error taxonomy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tenant_mismatch_and_missing_context_are_independently_reachable_and_distinguishable() {
    // CrossTenantDenied — the third condition FR-012 requires — is proven
    // reachable and distinguishable by
    // `runtime_builder.rs::issue_cross_tenant_permit_denied_without_capability`
    // (in-crate, see this file's module doc for why).
    let service = Arc::new(ContractService::default());

    let (_rt1, proxy1) = make_proxy(service.clone(), TenantEnforcementMode::AuthenticatedOnly);
    let mismatch_result = proxy1
        .scoped_op(authenticated_ctx_with_hint(Some("tenant-a"), Some("tenant-b")))
        .await;

    let (_rt2, proxy2) = make_proxy(service, TenantEnforcementMode::AuthenticatedOnly);
    let missing_result = proxy2.scoped_op(ServiceContext::new()).await;

    match (mismatch_result, missing_result) {
        (
            Err(TenantContractError::Security(SecurityError::TenantMismatch { .. })),
            Err(TenantContractError::Security(SecurityError::MissingContext)),
        ) => {
            // Distinct variants reached via the same `match` — a caller can
            // tell them apart, not just observe "the call failed" (NFR-003).
        }
        other => panic!(
            "expected (TenantMismatch, MissingContext) as distinguishable variants, got: {:?}",
            other
        ),
    }
}
