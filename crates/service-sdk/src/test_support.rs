//! Shared `#[cfg(test)]` fixtures for this crate's own internal unit tests
//! (code-review fix, CORE-008A): `runtime::runtime_builder`'s and
//! `context`'s test modules each hand-rolled an identical
//! `AllowCrossTenant`/`authenticated_ctx` pair, and `context`'s copy was
//! confirmed missing the `DenyCrossTenant` variant `runtime_builder`'s copy
//! has — a real, demonstrated drift. Consolidated here instead.
//!
//! Not for use outside this crate's own `#[cfg(test)]` modules — external
//! integration tests under `tests/*.rs` cannot see `pub(crate)` items and
//! have their own separate `tests/common/mod.rs`.

use std::sync::Arc;

use async_trait::async_trait;
use ego_security_sdk::authorization::{AccessRequest, AuthorizationDecision, AuthorizationProvider};
use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::error::SecurityError;
use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};

use crate::context::ServiceContext;

pub(crate) struct AllowCrossTenant;

#[async_trait]
impl AuthorizationProvider for AllowCrossTenant {
    async fn authorize(
        &self,
        _: &Principal,
        _: &AccessRequest,
        _: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Allow)
    }
}

pub(crate) struct DenyCrossTenant;

#[async_trait]
impl AuthorizationProvider for DenyCrossTenant {
    async fn authorize(
        &self,
        _: &Principal,
        _: &AccessRequest,
        _: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Deny {
            reason: "no cross-tenant capability".into(),
        })
    }
}

/// An authenticated `ServiceContext` for a fixed test principal ("alice"),
/// with no tenant claim set. Callers needing a specific tenant use
/// `ServiceContext::with_tenant_id`/`Principal.tenant_id` directly, same as
/// before consolidation.
pub(crate) fn authenticated_ctx() -> ServiceContext {
    let principal = Principal::new(PrincipalKind::User, SubjectId::new("alice").unwrap());
    let security = SecurityContext::empty(principal);
    ServiceContext::new().with_security(Arc::new(security))
}
