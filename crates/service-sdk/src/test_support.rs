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

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ego_domain::{Level, Observability, SemanticEvent};
use ego_security_sdk::authorization::{AccessRequest, AuthorizationDecision, AuthorizationProvider};
use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::error::SecurityError;
use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};

use crate::context::ServiceContext;

/// Test double capturing every [`SemanticEvent`] passed to `trace()`
/// (CORE-012A design.md Testing Strategy). Shared by `runtime_builder`'s and
/// `builder`'s internal `#[cfg(test)]` modules — both compile inside this
/// crate, so one definition here avoids two near-identical copies.
#[derive(Default)]
pub(crate) struct RecordingObservability {
    pub(crate) events: Mutex<Vec<SemanticEvent>>,
}

impl RecordingObservability {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns the recorded `denial_kind` metadata values, in call order.
    #[allow(dead_code)] // not every internal unit test uses this fixture method
    pub(crate) fn denial_kinds(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| e.metadata.get("denial_kind").cloned())
            .collect()
    }
}

impl Observability for RecordingObservability {
    fn trace(&self, event: SemanticEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn metric(&self, _name: &str, _value: f64) {}
    fn log(&self, _level: Level, _message: &str) {}
}

/// Test double proving `record_security_denial` isolates a misbehaving sink
/// (RESIL-001, CORE-012A 4R review): `trace()` always panics.
#[derive(Default)]
pub(crate) struct PanickingObservability;

impl Observability for PanickingObservability {
    fn trace(&self, _event: SemanticEvent) {
        panic!("PanickingObservability::trace always panics (test double)");
    }
    fn metric(&self, _name: &str, _value: f64) {}
    fn log(&self, _level: Level, _message: &str) {}
}

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
