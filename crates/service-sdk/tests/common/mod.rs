//! Shared fixtures for `service-sdk`'s integration test binaries
//! (code-review fix, CORE-008A): `tenant_enforcement_contract.rs` and
//! `tenant_enforcement_concurrency.rs` each hand-rolled an identical
//! `authenticated_ctx` (differing only in `Option<&str>` vs `&str`).
//!
//! `tests/common/mod.rs` is the standard Cargo convention for code shared
//! across integration-test binaries: a file directly under `tests/` would
//! itself compile as a separate test binary, but a `mod.rs` inside a
//! subdirectory does not — each consumer opts in with `mod common;`.

use std::sync::Arc;

use ego_domain::context::TenantId;
use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};
use ego_service_sdk::context::ServiceContext;

/// An authenticated `ServiceContext` for a fixed test principal ("alice"),
/// with `tenant` as the Principal's tenant claim (`None` for no claim).
#[allow(dead_code)] // not every consumer uses every helper in this module
pub fn authenticated_ctx(tenant: Option<&str>) -> ServiceContext {
    let mut principal = Principal::new(PrincipalKind::User, SubjectId::new("alice").unwrap());
    principal.tenant_id = tenant.map(|t| TenantId::new(t).unwrap());
    let security = SecurityContext::empty(principal);
    ServiceContext::new().with_security(Arc::new(security))
}

/// `authenticated_ctx` plus a caller-supplied tenant hint on top (or none).
#[allow(dead_code)]
pub fn authenticated_ctx_with_hint(tenant: Option<&str>, hint: Option<&str>) -> ServiceContext {
    let ctx = authenticated_ctx(tenant);
    match hint {
        Some(h) => ctx.with_tenant_id(h),
        None => ctx,
    }
}
