//! Shared fixtures for `service-sdk`'s integration test binaries
//! (code-review fix, CORE-008A): `tenant_enforcement_contract.rs` and
//! `tenant_enforcement_concurrency.rs` each hand-rolled an identical
//! `authenticated_ctx` (differing only in `Option<&str>` vs `&str`).
//!
//! `tests/common/mod.rs` is the standard Cargo convention for code shared
//! across integration-test binaries: a file directly under `tests/` would
//! itself compile as a separate test binary, but a `mod.rs` inside a
//! subdirectory does not — each consumer opts in with `mod common;`.

use std::sync::{Arc, Mutex};

use ego_domain::context::TenantId;
use ego_domain::{Level, Observability, SemanticEvent};
use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};
use ego_service_sdk::context::ServiceContext;

/// Test double capturing every [`SemanticEvent`] passed to `trace()`
/// (CORE-012A). Shared across this crate's integration-test binaries —
/// `tests/*.rs` files cannot see the crate's own internal
/// `#[cfg(test)] mod test_support`, so this is a separate, deliberately
/// near-identical copy for the `tests/` compilation unit.
#[derive(Default)]
#[allow(dead_code)] // not every consumer of this module uses this fixture
pub struct RecordingObservability {
    pub events: Mutex<Vec<SemanticEvent>>,
    /// Every metric emission, whole and in order.
    ///
    /// See [`ego_testkit::RecordedMetric::capture`] for why a trace-focused
    /// double records these at all, and why it appends rather than overwrites.
    pub metrics: Mutex<Vec<ego_testkit::RecordedMetric>>,
}

#[allow(dead_code)]
impl RecordingObservability {
    pub fn new() -> Self {
        Self::default()
    }

    /// The dimensions recorded for the most recent emission.
    pub fn last_metric_attributes(&self) -> Vec<(String, String)> {
        self.metrics
            .lock()
            .unwrap()
            .last()
            .map(|m| m.attributes.clone())
            .unwrap_or_default()
    }

    /// Returns the recorded `denial_kind` metadata values, in call order.
    pub fn denial_kinds(&self) -> Vec<String> {
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
    fn metric_with_attributes(
        &self,
        name: &'static str,
        value: f64,
        attributes: &[ego_domain::MetricAttribute<'_>],
    ) {
        self.metrics
            .lock()
            .unwrap()
            .push(ego_testkit::RecordedMetric::capture(
                name, value, attributes,
            ));
    }
    fn log(&self, _level: Level, _message: &str) {}
}

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

#[cfg(test)]
mod observability_contract {
    use super::RecordingObservability;

    /// This module's double preserves the dimensions it is handed.
    #[test]
    fn the_double_preserves_metric_attributes() {
        let obs = RecordingObservability::new();
        ego_testkit::assert_metric_attributes_are_preserved(&obs, || obs.last_metric_attributes());
    }
}
