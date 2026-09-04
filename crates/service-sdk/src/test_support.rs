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
use ego_domain::{Level, MetricAttribute, MetricObservation, Observability, SemanticEvent};
use ego_security_sdk::authorization::{
    AccessRequest, AuthorizationDecision, AuthorizationProvider,
};
use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::error::SecurityError;
use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};

use crate::context::ServiceContext;

use ego_testkit::RecordedMetric;

/// Test double capturing every [`SemanticEvent`] passed to `trace()`
/// (CORE-012A design.md Testing Strategy). Shared by `runtime_builder`'s and
/// `builder`'s internal `#[cfg(test)]` modules — both compile inside this
/// crate, so one definition here avoids two near-identical copies.
#[derive(Default)]
pub(crate) struct RecordingObservability {
    pub(crate) events: Mutex<Vec<SemanticEvent>>,
    /// Every `metric` call, in order, whole.
    ///
    /// This used to be discarded — `metric` had an empty body — which meant no test
    /// could assert on a counter at all. Recording it is what makes the AD-10 signals
    /// checkable, and what lets a mutation to a metric name or value fail something.
    ///
    /// One mutex over one collection: name, value and attributes are appended in a
    /// single critical section, so they are associated atomically no matter how many
    /// threads emit at once.
    pub(crate) metrics: Mutex<Vec<RecordedMetric>>,
}

impl RecordingObservability {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The names of every recorded metric, in call order.
    #[allow(dead_code)] // not every internal unit test asserts on metrics
    pub(crate) fn metric_names(&self) -> Vec<String> {
        self.metrics
            .lock()
            .unwrap()
            .iter()
            .map(|m| m.name.clone())
            .collect()
    }

    /// Whole records — kind, name, value and dimensions.
    ///
    /// Needed wherever the assertion is about attributes: `metrics()` projects a
    /// name and a value, which was enough while every dimension was folded into
    /// the name and is exactly what stops being enough once they are not.
    #[allow(dead_code)] // not every internal unit test asserts on metrics
    pub(crate) fn records(&self) -> Vec<RecordedMetric> {
        self.metrics.lock().unwrap().clone()
    }

    /// Every recorded metric as `(name, value)`, in call order.
    #[allow(dead_code)] // not every internal unit test asserts on metrics
    pub(crate) fn metrics(&self) -> Vec<(String, f64)> {
        self.metrics
            .lock()
            .unwrap()
            .iter()
            .map(|m| (m.name.clone(), m.value))
            .collect()
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
    fn record_metric(&self, observation: MetricObservation<'_>) {
        self.metrics
            .lock()
            .unwrap()
            .push(RecordedMetric::capture(&observation));
    }
    fn log(&self, _level: Level, _message: &str) {}
}

#[cfg(test)]
mod recording_observability_contract {
    use super::*;

    /// The fixture keeps every field of the observations it is handed.
    #[test]
    fn it_preserves_metric_observations() {
        let obs = RecordingObservability::new();
        ego_testkit::assert_metric_observations_are_preserved(&obs, || {
            obs.metrics.lock().unwrap().clone()
        });
    }

    /// Name, value and dimensions stay together under concurrent emission.
    ///
    /// The alternative this rejects — and it is a rejected design, not something
    /// this repository ever shipped — is keeping the three in parallel collections
    /// under a lock each. Those can only stay aligned if every writer holds every
    /// lock at once; with one lock per collection, two threads interleave as
    /// name(A), name(B), attrs(B), attrs(A), and a reader asking for A's
    /// dimensions is handed B's. That fixture reports a false result in either
    /// direction, so it is worth a test rather than a comment.
    ///
    /// Each thread tags its dimension with its own id, which is also its metric
    /// value, so a mispairing is visible in a single record without having to
    /// reconstruct the interleaving that produced it.
    ///
    /// The barrier is load-bearing. Without it each thread could run its whole
    /// loop before another starts, and the test would pass by never overlapping —
    /// proving nothing while a real hazard sat underneath. Releasing all threads
    /// together forces the contention the assertion is about.
    #[test]
    fn every_recorded_emission_keeps_its_own_dimensions_under_concurrency() {
        const THREADS: usize = 8;
        const PER_THREAD: usize = 64;

        let obs = Arc::new(RecordingObservability::new());
        let start = Arc::new(std::sync::Barrier::new(THREADS));
        std::thread::scope(|scope| {
            for thread in 0..THREADS {
                let obs = Arc::clone(&obs);
                let start = Arc::clone(&start);
                scope.spawn(move || {
                    let tag = thread.to_string();
                    start.wait();
                    for _ in 0..PER_THREAD {
                        obs.counter(
                            "concurrency.probe",
                            thread as f64,
                            &[MetricAttribute::new("thread", &tag)],
                        );
                    }
                });
            }
        });

        let recorded = obs.metrics.lock().unwrap();
        assert_eq!(
            recorded.len(),
            THREADS * PER_THREAD,
            "every emission is recorded exactly once"
        );
        for entry in recorded.iter() {
            assert_eq!(
                entry.attributes,
                vec![("thread".to_string(), (entry.value as usize).to_string())],
                "each record must carry the dimensions of its own emission, not another's"
            );
        }
    }
}

/// Test double proving a misbehaving sink is isolated (RESIL-001, CORE-012A 4R
/// review; issue #306): both `trace()` and `record_metric()` always panic.
///
/// # Two different isolation mechanisms, both covered
///
/// `trace()` panics escape unless the *caller* wraps the call in
/// `catch_unwind` — which every SDK call site does (`record_security_denial`,
/// `record_app_starting`, `record_app_started`, `record_completion_lost` in
/// `runtime_builder.rs`). `record_metric()` panics are instead caught by the
/// `Observability` trait's own `counter`/`histogram`/`gauge` default methods
/// (`ego_domain::observability`), so every metric call site inherits the
/// guarantee without repeating it. This double panics from both methods so
/// either kind of call site can be exercised against it.
#[derive(Default)]
pub(crate) struct PanickingObservability;

impl Observability for PanickingObservability {
    fn trace(&self, _event: SemanticEvent) {
        panic!("PanickingObservability::trace always panics (test double)");
    }
    fn record_metric(&self, _observation: MetricObservation<'_>) {
        panic!("PanickingObservability::record_metric always panics (test double)");
    }
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
